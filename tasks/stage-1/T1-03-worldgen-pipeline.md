# T1-03. Terrain generator pipeline

- **Status:** done (2026-11-22; speed target ≤ 1 ms per column confirmed by the owner)
- **Depends on:** T1-01, T0-12
- **Owner:** agent (cloud)

## Goal

Replace the T0-12 test corpus as the game's world source with the real generation pipeline: heightmap pre-pass → terrain → biomes → caves → ores → decorations (trees, water), deterministic per column and parallel on the job scheduler. The test corpus stays for benchmarks.

## Specs

- [worldgen.md](../../docs/architecture/worldgen.md)
- [T0-12 report](../../task-reports/t0-12-worldgen-test-corpus.md)

## Deliverables

- `worldgen::Generator` with the pipeline stages; block choice through the registry (no hard-coded block IDs in the engine — the block set comes from `mods/core` data).
- Determinism tests: same seed → same column hash regardless of order and thread count; structures crossing column borders are consistent.
- Benchmark: ms per column per core (fills the "column ≤ N ms per core" number in worldgen.md); target proposed to the owner with the measurement.

## Acceptance criteria

- [x] Determinism tests pass.
- [x] Generation speed measured and the worldgen.md number filled in; owner agrees on the target.
- [x] Generated worlds look plausible: a screenshot set from the T1-11 renderer is attached when available (or a heightmap image before that).

## Out of scope

Game zones (stage 4), mod-defined generation stages (stage 3+), LOD source data (stage 5).
