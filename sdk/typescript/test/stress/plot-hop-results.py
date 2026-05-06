#!/usr/bin/env python3
"""
Render a 2-panel chart from one or more hop-stress JSON results.

Left panel:  throughput (ops/s + MB/s) per payload size.
Right panel: latency percentile bars stacked as p50 + (p90 - p50) +
             (p95 - p90) + (p99 - p95).

The script picks the "Submit" phase out of each input file (so a
submit-only run, a group run, or the writers leg of a mixed run all
contribute one bar). Inputs are arranged by payload size.

Usage:
  ./plot-hop-results.py results/hop-ts-submit-*.json -o hop-results.png
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


def fmt_payload(size: float) -> str:
    if size >= 1024 ** 2:
        return f"{int(size // 1024 ** 2)}MB"
    if size >= 1024:
        return f"{int(size // 1024)}KB"
    return f"{int(size)}B"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("inputs", nargs="+", type=Path)
    ap.add_argument("-o", "--output", type=Path, required=True)
    ap.add_argument("--title",
                    default="HOP Stress Test Results (TS, local zombienet, 2 collators)")
    args = ap.parse_args()

    payloads = [load(p) for p in args.inputs]

    rows = []  # (payload_size, label, ops/s, MB/s, p50, p90, p99)
    for p in payloads:
        cfg = p.get("config", {})
        size = cfg.get("payloadSize", 0)
        scen = cfg.get("scenario", "?")
        # Pick the "Submit" or "Submit (writers)" phase if present, else first.
        rep = next(
            (r for r in p.get("reports", [])
             if r["name"].lower().startswith("submit")),
            (p.get("reports") or [None])[0],
        )
        if not rep or not rep.get("latency"):
            continue
        lat = rep["latency"]
        label = fmt_payload(size) if scen == "submit-only" else f"{scen}\n{fmt_payload(size)}"
        rows.append((
            size, label,
            rep.get("opsPerSec", 0.0),
            rep.get("bytesPerSec", 0.0) / (1024 ** 2),
            lat.get("p50", 0.0),
            lat.get("p90", lat.get("p99", 0.0)),
            lat.get("p99", 0.0),
        ))

    if not rows:
        print("No reports with latency data", file=sys.stderr)
        return 1

    rows.sort(key=lambda r: r[0])
    labels = [r[1] for r in rows]
    ops = [r[2] for r in rows]
    mbs = [r[3] for r in rows]
    p50 = np.array([r[4] for r in rows])
    p90 = np.array([r[5] for r in rows])
    p99 = np.array([r[6] for r in rows])

    fig, (ax_left, ax_right) = plt.subplots(
        1, 2, figsize=(13, 7), gridspec_kw={"width_ratios": [1, 1.2]}
    )
    fig.suptitle(args.title, fontsize=15, fontweight="bold")

    # ---------------- Left: throughput ----------------
    x = np.arange(len(rows))
    bw = 0.36
    ax_left.bar(x - bw / 2, ops, bw, label="ops/s", color="#1f77b4")
    ax_l2 = ax_left.twinx()
    ax_l2.bar(x + bw / 2, mbs, bw, label="MB/s", color="#ff7f0e")
    ax_left.set_xticks(x); ax_left.set_xticklabels(labels)
    ax_left.set_xlabel("Payload Size")
    ax_left.set_ylabel("ops/s", color="#1f77b4")
    ax_l2.set_ylabel("MB/s", color="#ff7f0e")
    ax_left.set_title("Submit Throughput")
    if ops:
        ax_left.set_ylim(0, max(ops) * 1.18)
    if mbs:
        ax_l2.set_ylim(0, max(mbs) * 1.18)
    for i, (o, m) in enumerate(zip(ops, mbs)):
        ax_left.annotate(f"{o:.0f}", (i - bw / 2, o), ha="center", va="bottom", fontsize=9)
        ax_l2.annotate(f"{m:.1f}", (i + bw / 2, m), ha="center", va="bottom", fontsize=9)
    h1, l1 = ax_left.get_legend_handles_labels()
    h2, l2 = ax_l2.get_legend_handles_labels()
    ax_left.legend(h1 + h2, l1 + l2, loc="upper left", fontsize=10)
    ax_left.grid(True, alpha=0.3, axis="y")

    # ---------------- Right: stacked latency ----------------
    seg_p90 = np.maximum(p90 - p50, 0)
    seg_p99 = np.maximum(p99 - p90, 0)

    bar_w = 0.55
    ax_right.bar(x, p50,     bar_w, label="p50", color="#7ac74f")
    ax_right.bar(x, seg_p90, bar_w, bottom=p50,
                 label="p90", color="#f7c948")
    ax_right.bar(x, seg_p99, bar_w, bottom=p50 + seg_p90,
                 label="p99", color="#e57373")

    for i, total in enumerate(p99):
        ax_right.annotate(
            f"{total:.0f}ms" if total < 1000 else f"{total / 1000:.2f}s",
            (i, total), ha="center", va="bottom", fontsize=9, fontweight="bold",
        )

    ax_right.set_xticks(x); ax_right.set_xticklabels(labels)
    ax_right.set_xlabel("Payload Size")
    ax_right.set_ylabel("Latency (ms)")
    ax_right.set_title("Submit Latency (p50 / p90 / p99)")
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
