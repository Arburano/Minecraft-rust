#!/usr/bin/env python3
"""Collect Criterion results into one JSON file: {"bench id": mean_ns}.

Usage: bench_collect.py <target/criterion dir> <out.json>
Reads every `<id>/new/estimates.json` (the latest run of each benchmark).
"""
import json
import pathlib
import sys


def main() -> int:
    root, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    results = {}
    for est in sorted(root.glob("**/new/estimates.json")):
        bench_id = str(est.parent.parent.relative_to(root))
        mean = json.loads(est.read_text())["mean"]["point_estimate"]
        results[bench_id] = mean
    out.write_text(json.dumps(results, indent=1, sort_keys=True))
    print(f"collected {len(results)} benchmarks into {out}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
