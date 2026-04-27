#!/usr/bin/env python3
"""
Render a PNG summary chart from one or more hop-stress JSON results.

Each input file is the JSON written by hop-stress.ts via --output-json.
The chart shows, per scenario:
  - throughput (ops/s + MB/s)
  - inclusion latency percentiles (p50/p90/p95/p99/max)
  - per-op latency CDF
  - mixed read/write breakdown if present

Usage:
  ./plot-hop-results.py results/hop-ts-submit-*.json results/hop-ts-group.json \
    -o hop-results.png
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
    scenario = cfg.get("scenario") or "?"
    size = cfg.get("payloadSize") or 0
    if size >= 1024 * 1024:
        sl = f"{size // (1024 * 1024)}MB"
    elif size >= 1024:
        sl = f"{size // 1024}KB"
    elif size:
        sl = f"{size}B"
    else:
        sl = "?"
    return f"{scenario}\n{sl}"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("inputs", nargs="+", type=Path)
    ap.add_argument("-o", "--output", type=Path, required=True)
    ap.add_argument("--title", default="HOP stress test (TS) — local zombienet")
    args = ap.parse_args()

    payloads = [load(p) for p in args.inputs]

    # Flatten reports — each payload may have multiple phases (e.g. mixed has
    # writers + readers; full-cycle has submit + claim).
    flat = []  # list of (label, report, scenario_label)
    for p, path in zip(payloads, args.inputs):
        scen = label_for(p, path.stem)
        for r in p.get("reports", []):
            flat.append((f"{scen}\n{r['name']}", r, scen))

    if not flat:
        print("No reports found in inputs", file=sys.stderr)
        return 1

    fig, axes = plt.subplots(2, 2, figsize=(13, 9))
    fig.suptitle(args.title, fontsize=14, fontweight="bold")

    labels = [label for label, _r, _s in flat]
    reports = [r for _l, r, _s in flat]

    # 1. Throughput (ops/s + MB/s)
    ax = axes[0, 0]
    x = np.arange(len(reports))
    ops = [r.get("opsPerSec", 0) for r in reports]
    mbs = [r.get("bytesPerSec", 0) / (1024 ** 2) for r in reports]
    bw = 0.36
    ax.bar(x - bw / 2, ops, bw, label="ops/s", color="#1f77b4")
    ax2 = ax.twinx()
    ax2.bar(x + bw / 2, mbs, bw, label="MB/s", color="#ff7f0e")
    ax.set_xticks(x); ax.set_xticklabels(labels, fontsize=8, rotation=15)
    ax.set_ylabel("ops/s", color="#1f77b4")
    ax2.set_ylabel("MB/s", color="#ff7f0e")
    ax.set_title("Throughput")
    if ops:
        ax.set_ylim(0, max(ops) * 1.18)
    if mbs:
        ax2.set_ylim(0, max(mbs) * 1.18)
    for i, (o, m) in enumerate(zip(ops, mbs)):
        ax.annotate(f"{o:.0f}", (i - bw / 2, o), ha="center", va="bottom", fontsize=8)
        ax2.annotate(f"{m:.1f}", (i + bw / 2, m), ha="center", va="bottom", fontsize=8)

    # 2. Latency percentiles
    ax = axes[0, 1]
    plot_percentile_bars(ax, reports, labels)

    # 3. CDF of per-op latency
    ax = axes[1, 1]
    drew = False
    for label, r, _ in flat:
        lats = r.get("latenciesMs") or []
        if not lats:
            continue
        sorted_l = np.sort(np.asarray(lats, dtype=float))
        cdf = np.linspace(1.0 / len(sorted_l), 1.0, len(sorted_l))
        ax.plot(sorted_l, cdf, label=label.replace("\n", " "), linewidth=1.5)
        drew = True
    ax.set_xlabel("Latency (ms)")
    ax.set_ylabel("CDF")
    ax.set_title("Per-op latency CDF")
    ax.set_ylim(0.0, 1.0)
    if drew:
        ax.set_xscale("log")
        ax.legend(fontsize=7, loc="lower right")
    ax.grid(True, alpha=0.3)

    # 4. Throughput-by-payload-size focus (submit-only sweep)
    ax = axes[1, 0]
    submits = [(p, r) for p in payloads
               for r in p.get("reports", [])
               if (p.get("config", {}).get("scenario") == "submit-only"
                   and r.get("name") == "Submit")]
    if submits:
        sizes = [p["config"]["payloadSize"] for p, _ in submits]
        ops_v = [r["opsPerSec"] for _, r in submits]
        mbs_v = [r["bytesPerSec"] / (1024 ** 2) for _, r in submits]
        order = np.argsort(sizes)
        sizes = [sizes[i] for i in order]
        ops_v = [ops_v[i] for i in order]
        mbs_v = [mbs_v[i] for i in order]
        size_labels = [
            f"{s // (1024 ** 2)}MB" if s >= 1024 ** 2
            else f"{s // 1024}KB" if s >= 1024
            else f"{s}B"
            for s in sizes
        ]
        xs = np.arange(len(sizes))
        ax.bar(xs - bw / 2, ops_v, bw, label="ops/s", color="#1f77b4")
        ax3 = ax.twinx()
        ax3.bar(xs + bw / 2, mbs_v, bw, label="MB/s", color="#ff7f0e")
        ax.set_xticks(xs); ax.set_xticklabels(size_labels)
        ax.set_ylabel("ops/s", color="#1f77b4")
        ax3.set_ylabel("MB/s", color="#ff7f0e")
        ax.set_title("submit-only: throughput vs payload size")
        ax.set_ylim(0, max(ops_v) * 1.18 if ops_v else 1)
        ax3.set_ylim(0, max(mbs_v) * 1.18 if mbs_v else 1)
        for i, (o, m) in enumerate(zip(ops_v, mbs_v)):
            ax.annotate(f"{o:.0f}", (i - bw / 2, o), ha="center", va="bottom", fontsize=9)
            ax3.annotate(f"{m:.1f}", (i + bw / 2, m), ha="center", va="bottom", fontsize=9)
    else:
        ax.set_title("(no submit-only data)")

    plt.tight_layout()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(args.output, dpi=140)
    print(f"Wrote {args.output}", file=sys.stderr)
    return 0


def plot_percentile_bars(ax, reports, labels) -> None:
    metrics = ["p50", "p90", "p95", "p99", "max"]
    colors = ["#4c72b0", "#55a868", "#c44e52", "#8172b2", "#937860"]
    width = 0.14
    x = np.arange(len(reports))
    max_val = 0.0
    for i, m in enumerate(metrics):
        vals = []
        for r in reports:
            lat = r.get("latency") or {}
            vals.append(float(lat.get(m, 0.0)))
        if not vals:
            continue
        max_val = max(max_val, max(vals))
        offset = (i - (len(metrics) - 1) / 2) * width
        bars = ax.bar(x + offset, vals, width, label=m, color=colors[i])
        for b, v in zip(bars, vals):
            if v > 0:
                ax.annotate(
                    f"{v:.0f}ms",
                    (b.get_x() + b.get_width() / 2, v),
                    ha="center", va="bottom", fontsize=7,
                )
    ax.set_xticks(x); ax.set_xticklabels(labels, fontsize=7, rotation=20)
    ax.set_ylim(0, max_val * 1.25 if max_val else 1)
    ax.set_ylabel("Latency (ms)")
    ax.set_title("Per-op latency percentiles")
    ax.legend(loc="upper right", fontsize=7, ncol=5)
    ax.grid(True, alpha=0.3, axis="y")


if __name__ == "__main__":
    sys.exit(main())
