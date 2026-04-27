#!/usr/bin/env python3
"""
Render a 2-panel chart from one or more pipeline-stress JSON results.

Left panel:  throughput (tx/s + KB/s) per scenario.
Right panel: latency percentile bars stacked as p50 + (p90 - p50) +
             (p95 - p90) + (p99 - p95). Each band shows the *additional*
             latency contributed by tightening the percentile.

By default the chart plots finalization latency (the practical
end-to-end metric); pass --latency inclusion to switch to
broadcast-to-best-block.

Usage:
  ./plot-pipeline-results.py results/pr435-500x128kb-acct*.json -o pipeline.png
"""

import argparse
import json
import sys
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np


def load(path: Path) -> dict:
    with path.open() as f:
        return json.load(f)


def label_for(payload: dict, fallback: str) -> str:
    cfg = payload.get("config", {})
    items = cfg.get("items")
    size = cfg.get("payloadSize")
    accounts = cfg.get("accounts", 1)
    if items is None or size is None:
        return fallback
    if size >= 1024 * 1024:
        size_label = f"{size // (1024 * 1024)}MB"
    elif size >= 1024:
        size_label = f"{size // 1024}KB"
    else:
        size_label = f"{size}B"
    if accounts and accounts >= 1:
        plural = "acct" if accounts == 1 else "accts"
        return f"{items} × {size_label}\n{accounts} {plural}"
    return f"{items} × {size_label}"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("inputs", nargs="+", type=Path, help="result JSON files")
    ap.add_argument("-o", "--output", type=Path, required=True)
    ap.add_argument("--title", default="pipelineStore — Versi (4 RPCs)")
    ap.add_argument("--latency", choices=["inclusion", "finalization"],
                    default="finalization",
                    help="which latency series to chart (default: finalization)")
    args = ap.parse_args()

    payloads = [load(p) for p in args.inputs]
    labels = [label_for(p, args.inputs[i].stem) for i, p in enumerate(payloads)]
    results = [p["result"] for p in payloads]

    key = "finalizationLatency" if args.latency == "finalization" else "inclusionLatency"
    side_label = "Finalization" if args.latency == "finalization" else "Inclusion"

    p50 = np.array([float((r.get(key) or {}).get("p50", 0.0)) for r in results])
    p90 = np.array([
        float((r.get(key) or {}).get("p90", (r.get(key) or {}).get("p95", 0.0)))
        for r in results
    ])
    p95 = np.array([float((r.get(key) or {}).get("p95", 0.0)) for r in results])
    p99 = np.array([float((r.get(key) or {}).get("p99", 0.0)) for r in results])
    p50, p90, p95, p99 = p50 / 1000, p90 / 1000, p95 / 1000, p99 / 1000

    tps = [r.get("txPerSec", 0.0) for r in results]
    kbs = [r.get("throughputBytesPerSec", 0.0) / 1024.0 for r in results]

    fig, (ax_left, ax_right) = plt.subplots(
        1, 2, figsize=(13, 7), gridspec_kw={"width_ratios": [1, 1.2]}
    )
    fig.suptitle(args.title, fontsize=15, fontweight="bold")

    # ---------------- Left: throughput ----------------
    x = np.arange(len(results))
    bw = 0.36
    ax_left.bar(x - bw / 2, tps, bw, label="tx/s", color="#1f77b4")
    ax_l2 = ax_left.twinx()
    ax_l2.bar(x + bw / 2, kbs, bw, label="KB/s", color="#ff7f0e")
    ax_left.set_xticks(x); ax_left.set_xticklabels(labels)
    ax_left.set_xlim(-0.7, len(results) - 0.3)
    ax_left.set_ylabel("tx/s", color="#1f77b4")
    ax_l2.set_ylabel("KB/s", color="#ff7f0e")
    ax_left.set_title("Finalized Throughput")
    if tps:
        ax_left.set_ylim(0, max(tps) * 1.18)
    if kbs:
        ax_l2.set_ylim(0, max(kbs) * 1.18)
    for i, (t, k) in enumerate(zip(tps, kbs)):
        ax_left.annotate(f"{t:.2f}", (i - bw / 2, t),
                         ha="center", va="bottom", fontsize=9)
        ax_l2.annotate(f"{k:.0f}", (i + bw / 2, k),
                       ha="center", va="bottom", fontsize=9)
    h1, l1 = ax_left.get_legend_handles_labels()
    h2, l2 = ax_l2.get_legend_handles_labels()
    ax_left.legend(h1 + h2, l1 + l2, loc="upper left", fontsize=10)
    ax_left.grid(True, alpha=0.3, axis="y")

    # ---------------- Right: stacked latency ----------------
    seg_p90 = np.maximum(p90 - p50, 0)
    seg_p95 = np.maximum(p95 - p90, 0)
    seg_p99 = np.maximum(p99 - p95, 0)

    bar_w = 0.55
    ax_right.bar(x, p50,    bar_w, label="p50", color="#7ac74f")
    ax_right.bar(x, seg_p90, bar_w, bottom=p50,
                 label="p90", color="#a8d65c")
    ax_right.bar(x, seg_p95, bar_w, bottom=p50 + seg_p90,
                 label="p95", color="#f7c948")
    ax_right.bar(x, seg_p99, bar_w, bottom=p50 + seg_p90 + seg_p95,
                 label="p99", color="#e57373")

    for i, total in enumerate(p99):
        ax_right.annotate(
            f"{total:.1f}s",
            (i, total), ha="center", va="bottom", fontsize=9, fontweight="bold",
        )

    ax_right.set_xticks(x); ax_right.set_xticklabels(labels)
    ax_right.set_xlim(-0.7, len(results) - 0.3)
    ax_right.set_ylabel("Latency (s)")
    ax_right.set_title(f"{side_label} Latency (p50 / p90 / p95 / p99)")
    ax_right.legend(loc="upper left", fontsize=10)
    ax_right.set_ylim(0, max(p99) * 1.15 if p99.size else 1)
    ax_right.grid(True, alpha=0.3, axis="y")

    plt.tight_layout()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(args.output, dpi=140)
    print(f"Wrote {args.output}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
