#!/usr/bin/env python3
"""
Render a 2-panel chart from a stress-test --output-file JSON.

Left panel:  throughput (ops/s + MB/s) per payload size.
Right panel: latency percentile bars stacked as p50 + (p90 - p50) +
             (p95 - p90) + (p99 - p95). Each band shows the *additional*
             latency contributed by tightening the percentile.

Usage:
  ./plot-hop-results.py results/hop-all.json -o stress-test/hop-results.png
"""

import argparse
import json
import re
import sys
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np

PAYLOAD_RE = re.compile(r"(\d+(?:\.\d+)?)\s*(B|KB|MB|GB)$")


def parse_payload(name: str) -> float | None:
    m = PAYLOAD_RE.search(name)
    if not m:
        return None
    val, unit = float(m.group(1)), m.group(2)
    return val * {"B": 1, "KB": 1024, "MB": 1024 ** 2, "GB": 1024 ** 3}[unit]


def fmt_payload(size: float) -> str:
    if size >= 1024 ** 2:
        return f"{int(size // 1024 ** 2)}MB"
    if size >= 1024:
        return f"{int(size // 1024)}KB"
    return f"{int(size)}B"


def duration_ms(d) -> float:
    if isinstance(d, dict):
        return float(d.get("secs", 0)) * 1000.0 + float(d.get("nanos", 0)) / 1_000_000.0
    return float(d) * 1000.0


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("input", type=Path, help="results JSON from --output-file")
    ap.add_argument("-o", "--output", type=Path, required=True)
    ap.add_argument("--title",
                    default="HOP Stress Test Results (Rust, local zombienet, 2 collators)")
    args = ap.parse_args()

    with args.input.open() as f:
        results = json.load(f)

    submits = [r for r in results
               if r.get("name", "").startswith("HOP submit ")
               and r.get("inclusion_latency")]
    submits.sort(key=lambda r: parse_payload(r["name"]) or 0)
    if not submits:
        print("No HOP submit-only results with latency data", file=sys.stderr)
        return 1

    labels = [fmt_payload(parse_payload(r["name"]) or 0) for r in submits]
    ops = [r.get("throughput_tps", 0.0) for r in submits]
    mbs = [r.get("throughput_bytes_per_sec", 0.0) / (1024 ** 2) for r in submits]

    fig, (ax_left, ax_right) = plt.subplots(
        1, 2, figsize=(13, 7), gridspec_kw={"width_ratios": [1, 1.2]}
    )
    fig.suptitle(args.title, fontsize=15, fontweight="bold")

    # ---------------- Left: throughput ----------------
    x = np.arange(len(submits))
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
    p50 = np.array([duration_ms(r["inclusion_latency"]["p50"]) for r in submits])
    p90 = np.array([
        duration_ms(r["inclusion_latency"].get("p90", r["inclusion_latency"]["p95"]))
        for r in submits
    ])
    p95 = np.array([duration_ms(r["inclusion_latency"]["p95"]) for r in submits])
    p99 = np.array([duration_ms(r["inclusion_latency"]["p99"]) for r in submits])

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
            f"{total:.0f}ms" if total < 1000 else f"{total / 1000:.2f}s",
            (i, total), ha="center", va="bottom", fontsize=9, fontweight="bold",
        )

    ax_right.set_xticks(x); ax_right.set_xticklabels(labels)
    ax_right.set_xlabel("Payload Size")
    ax_right.set_ylabel("Latency (ms)")
    ax_right.set_title("Submit Latency (p50 / p90 / p95 / p99)")
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
