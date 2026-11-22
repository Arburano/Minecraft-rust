# T1-02. On-disk world format: save and load

- **Status:** done (2026-11-21)
- **Depends on:** T1-01
- **Owner:** agent (cloud)

## Goal

Save and load columns (blocks + light + heightmap) to disk with a versioned format, so a single-player world survives restarts. Writes happen off the main thread from snapshots; a crash never leaves a half-written column.

## Specs

- [world-storage.md](../../docs/architecture/world-storage.md) — On-disk storage, Network format
- [testing-and-benchmarks.md](../../docs/process/testing-and-benchmarks.md) — golden files for disk formats

## Deliverables

- Format spec in world-storage.md (header with format version, per-column compressed records, atomic write via temp file + rename).
- `world::storage` save/load API; background writer job on the T0-02 scheduler.
- Golden files in `crates/world/tests/golden/` (generated from the T0-12 corpus) + round-trip proptest.
- Benchmark: save/load of one column and of a 12-chunk-radius area; compressed size per column (data for open question 7).

## Acceptance criteria

- [x] Round-trip proptest and golden-file tests pass; format version checked on load.
- [x] Crash safety test: an interrupted write leaves the previous version readable.
- [x] Numbers in performance-budgets; open question 7 updated with the measured column size.

## Out of scope

Network format, LOD storage (stage 5), migration between format versions beyond a version check.
