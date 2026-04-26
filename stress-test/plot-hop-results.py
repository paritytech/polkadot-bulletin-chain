#!/usr/bin/env python3
"""
Render a PNG summary chart from a stress-test --output-file JSON.

The Rust stress-test writes a JSON array of ScenarioResult records.
This script reads that file and produces a 4-panel chart:
  - Submit throughput at varying payload sizes (ops/s + MB/s)
  - Inclusion latency percentiles (p50/p90/p95/p99/max)
  - Mixed scenario read vs write throughput
  - Per-item inclusion latency CDF (per scenario)

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
    """Extract payload size in bytes from scenario name suffix like '1 KB'."""
    m = PAYLOAD_RE.search(name)
    if not m:
        return None
    val, unit = float(m.group(1)), m.group(2)
    return val * {"B": 1, "KB": 1024, "MB": 1024 ** 2, "GB": 1024 ** 3}[unit]


def fmt_payload(size: float) -> str:
    if size >= 1024 ** 2:
        return f"{size / 1024 ** 2:.0f} MB"
    if size >= 1024:
        return f"{size / 1024:.0f} KB"
    return f"{int(size)} B"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("input", type=Path, help="results JSON from --output-file")
    ap.add_argument("-o", "--output", type=Path, required=True)
    ap.add_argument("--title", default="HOP stress test — local zombienet (2 collators)")
    args = ap.parse_args()

    with args.input.open() as f:
        results = json.load(f)

    fig, axes = plt.subplots(2, 2, figsize=(13, 9))
    fig.suptitle(args.title, fontsize=14, fontweight="bold")

    # 1. Submit-only throughput (across payload sizes)
    submits = [r for r in results if r["name"].startswith("HOP submit ")]
    ax = axes[0, 0]
    if submits:
        sizes = [parse_payload(r["name"]) or 0 for r in submits]
        ops = [r.get("throughput_tps", 0.0) for r in submits]
        mbs = [r.get("throughput_bytes_per_sec", 0.0) / (1024 ** 2) for r in submits]
        labels = [fmt_payload(s) for s in sizes]
        x = np.arange(len(submits))
        bw = 0.36
        ax.bar(x - bw / 2, ops, bw, label="ops/s", color="#1f77b4")
        ax2 = ax.twinx()
        ax2.bar(x + bw / 2, mbs, bw, label="MB/s", color="#ff7f0e")
        ax.set_xticks(x); ax.set_xticklabels(labels)
        ax.set_ylabel("ops/s", color="#1f77b4")
        ax2.set_ylabel("MB/s", color="#ff7f0e")
        ax.set_title("Submit throughput by payload size")
        ax.set_ylim(0, max(ops) * 1.18 if ops else 1)
        ax2.set_ylim(0, max(mbs) * 1.18 if mbs else 1)
        for i, (o, m) in enumerate(zip(ops, mbs)):
            ax.annotate(f"{o:.0f}", (i - bw / 2, o), ha="center", va="bottom", fontsize=8)
            ax2.annotate(f"{m:.1f}", (i + bw / 2, m), ha="center", va="bottom", fontsize=8)
    else:
        ax.set_title("(no submit-only results)")

    # 2. Inclusion latency percentiles for all scenarios that have it
    with_lat = [r for r in results if r.get("inclusion_latency")]
    ax = axes[0, 1]
    if with_lat:
        plot_percentile_bars(ax, with_lat,
                            "Inclusion latency (broadcast → block)")
    else:
        ax.set_title("(no inclusion-latency samples)")

    # 3. Mixed read/write or other special scenario throughput summary
    ax = axes[1, 0]
    mixed = next((r for r in results if "mixed" in r["name"].lower()), None)
    if mixed:
        labels, vals, colors = [], [], []
        if mixed.get("throughput_tps", 0):
            labels.append("write\nops/s"); vals.append(mixed["throughput_tps"]); colors.append("#1f77b4")
        if mixed.get("reads_per_sec"):
            labels.append("read\nops/s"); vals.append(mixed["reads_per_sec"]); colors.append("#2ca02c")
        if mixed.get("throughput_bytes_per_sec"):
            labels.append("write\nMB/s"); vals.append(mixed["throughput_bytes_per_sec"] / 1024 ** 2); colors.append("#ff7f0e")
        if mixed.get("read_bytes_per_sec"):
            labels.append("read\nMB/s"); vals.append(mixed["read_bytes_per_sec"] / 1024 ** 2); colors.append("#d62728")
        bars = ax.bar(np.arange(len(labels)), vals, color=colors)
        ax.set_xticks(np.arange(len(labels))); ax.set_xticklabels(labels)
        ax.set_ylabel("rate")
        dur = mixed.get("duration", {})
        if isinstance(dur, dict):
            dur_s = dur.get("secs", 0) + dur.get("nanos", 0) / 1e9
            dur_label = f"{dur_s:.0f}s"
        else:
            dur_label = f"{dur}s"
        ax.set_title(f"Mixed read+write ({dur_label})")
        for b, v in zip(bars, vals):
            ax.annotate(
                f"{v:.0f}" if v >= 10 else f"{v:.2f}",
                (b.get_x() + b.get_width() / 2, v),
                ha="center", va="bottom", fontsize=9,
            )
    else:
        # Fallback: show overall scenario throughput as bar chart
        all_with_tps = [r for r in results if r.get("throughput_tps", 0) > 0]
        labels = [r["name"][:24] for r in all_with_tps]
        vals = [r.get("throughput_tps", 0) for r in all_with_tps]
        ax.barh(np.arange(len(labels)), vals, color="#1f77b4")
        ax.set_yticks(np.arange(len(labels))); ax.set_yticklabels(labels, fontsize=8)
        ax.set_xlabel("ops/s")
        ax.set_title("Throughput per scenario")

    # 4. Inclusion latency CDF per scenario (where samples exist as a series)
    ax = axes[1, 1]
    drew = False
    for r in with_lat:
        # No raw arrays in the JSON — synthesize a step CDF from p50/p90/p95/p99/max
        lat = r["inclusion_latency"]
        if not lat:
            continue
        # Stair from min/p50/p90/p95/p99/max
        xs_ms = [
            duration_ms(lat.get("min", lat.get("p50", 0))),
            duration_ms(lat["p50"]),
            duration_ms(lat.get("p90", lat["p95"])),
            duration_ms(lat["p95"]),
            duration_ms(lat["p99"]),
            duration_ms(lat["max"]),
        ]
        ys = [0.0, 0.5, 0.9, 0.95, 0.99, 1.0]
        ax.plot(xs_ms, ys, drawstyle="steps-post", linewidth=1.5,
                label=r["name"][:32])
        drew = True
    ax.set_xlabel("Inclusion latency (ms)")
    ax.set_ylabel("CDF (estimated from percentiles)")
    ax.set_title("Inclusion latency CDF per scenario")
    if drew:
        ax.legend(fontsize=7, loc="lower right")
        ax.set_xscale("log")
    ax.grid(True, alpha=0.3)
    ax.set_ylim(0.0, 1.05)

    plt.tight_layout()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    fig.savefig(args.output, dpi=140)
    print(f"Wrote {args.output}", file=sys.stderr)
    return 0


def duration_ms(d) -> float:
    """ScenarioResult durations serialise as { secs, nanos }."""
    if isinstance(d, dict):
        return float(d.get("secs", 0)) * 1000.0 + float(d.get("nanos", 0)) / 1_000_000.0
    return float(d)


def plot_percentile_bars(ax, results: list[dict], title: str) -> None:
    metrics = ["p50", "p90", "p95", "p99", "max"]
    colors = ["#4c72b0", "#55a868", "#c44e52", "#8172b2", "#937860"]
    width = 0.14
    x = np.arange(len(results))

    max_val = 0.0
    for i, m in enumerate(metrics):
        vals = []
        for r in results:
            lat = r.get("inclusion_latency") or {}
            v = duration_ms(lat.get(m, lat.get("p99", 0))) / 1000.0
            vals.append(v)
        max_val = max(max_val, max(vals) if vals else 0.0)
        offset = (i - (len(metrics) - 1) / 2) * width
        bars = ax.bar(x + offset, vals, width, label=m, color=colors[i])
        for b, v in zip(bars, vals):
            if v > 0:
                ax.annotate(
                    f"{v:.1f}s" if v >= 1 else f"{v * 1000:.0f}ms",
                    (b.get_x() + b.get_width() / 2, v),
                    ha="center", va="bottom", fontsize=7,
                )
    labels = [r["name"].replace("HOP ", "")[:18] for r in results]
    ax.set_xticks(x); ax.set_xticklabels(labels, fontsize=8, rotation=15)
    ax.set_ylim(0, max_val * 1.25 if max_val else 1)
    ax.set_ylabel("Latency (s)")
    ax.set_title(title)
    ax.legend(loc="upper right", fontsize=8, ncol=5)
    ax.grid(True, alpha=0.3, axis="y")


if __name__ == "__main__":
    sys.exit(main())
