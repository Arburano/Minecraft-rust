#!/usr/bin/env python3
"""Compare two bench_collect.py results and flag regressions.

Usage: bench_compare.py <base.json> <new.json> [--threshold 0.10] [--strict]

Prints a Markdown table (suitable for $GITHUB_STEP_SUMMARY). A benchmark
slower than the base by more than the threshold is a regression. Advisory by
default (exit 0) because shared CI runners are noisy
(docs/process/testing-and-benchmarks.md); --strict exits 1 on regressions,
for use once a stable runner exists.
"""
import argparse
import json
import sys


def fmt(ns: float) -> str:
    for unit, scale in (("s", 1e9), ("ms", 1e6), ("µs", 1e3)):
        if ns >= scale:
            return f"{ns / scale:.3g} {unit}"
    return f"{ns:.3g} ns"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("base")
    ap.add_argument("new")
    ap.add_argument("--threshold", type=float, default=0.10)
    ap.add_argument("--strict", action="store_true")
    args = ap.parse_args()
    base = json.load(open(args.base))
    new = json.load(open(args.new))
    rows, regressions = [], 0
    for name in sorted(set(base) | set(new)):
        if name not in base or name not in new:
            rows.append(f"| `{name}` | {fmt(base[name]) if name in base else '—'} | "
                        f"{fmt(new[name]) if name in new else '—'} | — | new/removed |")
            continue
        change = new[name] / base[name] - 1.0
        flag = ""
        if change > args.threshold:
            flag = "**regression**"
            regressions += 1
        elif change < -args.threshold:
            flag = "faster"
        rows.append(f"| `{name}` | {fmt(base[name])} | {fmt(new[name])} | {change:+.1%} | {flag} |")
    print(f"### Benchmarks vs base (threshold {args.threshold:.0%}, "
          f"{'strict' if args.strict else 'advisory'})\n")
    print("| Benchmark | Base | This run | Change | |\n| --- | --- | --- | --- | --- |")
    print("\n".join(rows))
    print(f"\n{regressions} regression(s). Shared-runner noise is often ±10–30%.")
    return 1 if (args.strict and regressions) else 0


if __name__ == "__main__":
    sys.exit(main())
