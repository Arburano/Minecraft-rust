# Tests and benchmarks

## Kinds of tests

| Kind | Where | When mandatory |
| --- | --- | --- |
| Unit tests | `#[cfg(test)]` in the module | every public function with logic |
| Property tests (`proptest`) | crate `tests/` | coordinates, palette, lighting, masks, region merging, serialization |
| Reference comparisons | `tests/` | optimized implementation against a naive one (mesher, lighting, gather) |
| Golden files | `tests/golden/` | disk and network formats, asset conversion |
| Integration | `crates/<bin>/tests/` | server + bots, handshake, mods |
| Mod scenario tests | `engine-cli test` | every mod in `mods/` |
| Fuzzing | `fuzz/` (cargo-fuzz), later | network and disk decoders |

## Benchmarks

- `criterion` in `crates/<crate>/benches/`. CI checks that they compile (`cargo bench --no-run`).
- Each benchmark corresponds to a row in [`performance-budgets.md`](performance-budgets.md); the name or a comment contains a reference to that row.
- A fixed data corpus: `worldgen::test_corpus` with a fixed seed (T0-12; `research/bench` was the starting point), later — an export of a real world.
- Stage 0 results and the test machine are recorded in `docs/research/`. A 2 vCPU VM gives ±30% noise — final numbers are taken on bare metal with a fixed frequency (spread ≤ 5%).
- A regression > 10% on a hot benchmark is a PR blocker (automatically, once a stable runner exists).
- CI (T1-06): `.github/workflows/bench.yml` runs a fast subset (`world` section + map, `mesh`, `light`, `assets`; 20 samples, 2 s) on every PR and on `main`, uploads `bench-results.json` (`scripts/bench_collect.py`: mean ns per benchmark) and writes a comparison with the latest successful `main` run to the job summary (`scripts/bench_compare.py`). It is **advisory** on shared runners; `bench_compare.py --strict` fails on regressions once a stable runner exists.

## What every PR needs

1. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` — green.
2. New logic — with tests. An optimization — with a before/after benchmark.
3. If a disk/network format changed — updated golden files and the format version.
4. If a specification changed — the updated document in `docs/`.

## Without a GPU

CI has no GPU. Render code is split into pure logic (culling, packing, path selection by capabilities) with unit tests and a thin wgpu layer. Tests with a GPU are marked `#[ignore]` and run manually/on a separate runner.
