#!/usr/bin/env python3
"""
Render a PNG summary chart from one or more pipeline-stress JSON results.

Usage:
  ./plot-pipeline-results.py results/pr420-500x128kb.json -o pipeline-results.png

Each input file is the JSON written by pipeline-stress.ts via --output-json.
The chart shows, per scenario:
  - throughput (tx/s and KB/s) bar chart
  - inclusion + finalization latency percentiles (p50/p90/p95/p99/max)
  - per-item latency CDF
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
    if items is None or size is None:
        return fallback
    if size >= 1024 * 1024:
        size_label = f"{size // (1024 * 1024)} MB"
    elif size >= 1024:
        size_label = f"{size // 1024} KB"
    else:
        size_label = f"{size} B"
    return f"{items} × {size_label}"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("inputs", nargs="+", type=Path, help="result JSON files")
    ap.add_argument("-o", "--output", type=Path, required=True, help="output PNG path")
    ap.add_argument("--title", default="pipelineStore — Versi (4 RPCs)")
    args = ap.parse_args()

    payloads = [load(p) for p in args.inputs]
    labels = [label_for(p, args.inputs[i].stem) for i, p in enumerate(payloads)]
    results = [p["result"] for p in payloads]

    fig, axes = plt.subplots(2, 2, figsize=(13, 9))
    fig.suptitle(args.title, fontsize=14, fontweight="bold")

    # 1. Throughput
    ax = axes[0, 0]
    x = np.arange(len(results))
    tps = [r.get("txPerSec", 0.0) for r in results]
    kbs = [r.get("throughputBytesPerSec", 0.0) / 1024.0 for r in results]
    bar_w = 0.32
    ax.bar(x - bar_w / 2, tps, bar_w, label="tx/s", color="#1f77b4")
    ax2 = ax.twinx()
    ax2.bar(x + bar_w / 2, kbs, bar_w, label="KB/s", color="#ff7f0e")
    ax.set_xticks(x)
    ax.set_xticklabels(labels)
    ax.set_xlim(-0.7, len(results) - 0.3)
    ax.set_ylabel("tx/s", color="#1f77b4")
    ax2.set_ylabel("KB/s", color="#ff7f0e")
    ax.set_title("Throughput (finalized)")
    ax.set_ylim(0, max(tps) * 1.18 if tps else 1)
    ax2.set_ylim(0, max(kbs) * 1.18 if kbs else 1)
    for i, (t, k) in enumerate(zip(tps, kbs)):
        ax.annotate(f"{t:.2f}", (i - bar_w / 2, t), ha="center", va="bottom", fontsize=9)
        ax2.annotate(f"{k:.0f}", (i + bar_w / 2, k), ha="center", va="bottom", fontsize=9)

    # 2. Inclusion latency percentiles
    ax = axes[0, 1]
    plot_percentile_bars(ax, results, labels, "inclusionLatency",
                        "Inclusion latency (broadcast → best block)")

    # 3. Finalization latency percentiles
    ax = axes[1, 0]
    plot_percentile_bars(ax, results, labels, "finalizationLatency",
                        "Finalization latency (broadcast → finalized)")

    # 4. CDF of per-item inclusion + finalization latency
    ax = axes[1, 1]
    for label, r in zip(labels, results):
        for key, style, suffix in [
            ("inclusionLatenciesMs", "-", "incl"),
            ("finalizationLatenciesMs", "--", "final"),
        ]:
            lats = r.get(key) or []
            if not lats:
                continue
            sorted_lats = np.sort(np.asarray(lats, dtype=float)) / 1000.0
            cdf = np.linspace(1.0 / len(sorted_lats), 1.0, len(sorted_lats))
            ax.plot(sorted_lats, cdf,
                    label=f"{label} ({suffix})" if len(results) > 1 else suffix,
                    linewidth=1.8, linestyle=style)
    ax.set_xlabel("Latency (s)")
    ax.set_ylabel("CDF")
    ax.set_title("Per-item latency CDF (solid=inclusion, dashed=final)")
    ax.set_ylim(0.0, 1.0)
    ax.grid(True, alpha=0.3)
    ax.legend(fontsize=9, loc="lower right")

    plt.tight_layout()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(args.output, dpi=140)
    print(f"Wrote {args.output}", file=sys.stderr)
    return 0


def plot_percentile_bars(ax, results, labels, key: str, title: str) -> None:
    metrics = ["p50", "p90", "p95", "p99", "max"]
    colors = ["#4c72b0", "#55a868", "#c44e52", "#8172b2", "#937860"]
    x = np.arange(len(results))
    width = 0.14

    max_val = 0.0
    for i, m in enumerate(metrics):
        vals = []
        for r in results:
            lat = r.get(key)
            if lat is None:
                vals.append(0.0)
            else:
                vals.append(float(lat.get(m, 0.0)) / 1000.0)
        max_val = max(max_val, max(vals) if vals else 0.0)
        offset = (i - (len(metrics) - 1) / 2) * width
        bars = ax.bar(x + offset, vals, width, label=m, color=colors[i])
        for b, v in zip(bars, vals):
            if v > 0:
                ax.annotate(
                    f"{v:.1f}s" if v >= 1 else f"{v*1000:.0f}ms",
                    (b.get_x() + b.get_width() / 2, v),
                    ha="center", va="bottom", fontsize=8,
                )
    ax.set_xticks(x)
    ax.set_xticklabels(labels)
    ax.set_xlim(-0.7, len(results) - 0.3)
    ax.set_ylim(0, max_val * 1.25 if max_val else 1)
    ax.set_ylabel("Latency (s)")
    ax.set_title(title)
    ax.legend(loc="upper right", fontsize=8, ncol=5)
    ax.grid(True, alpha=0.3, axis="y")


if __name__ == "__main__":
    sys.exit(main())
