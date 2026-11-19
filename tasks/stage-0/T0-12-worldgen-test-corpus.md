# T0-12. Deterministic test terrain

- **Status:** done (branch `qoder/general-session-7lrerm`, [report](../../task-reports/t0-12-worldgen-test-corpus.md))
- **Depends on:** T0-03
- **Owner:** agent

## Goal

In the `worldgen` crate: a simple deterministic generator (heightmap + caves + ores + ~8% transparent blocks) for the benchmark corpus and tests. Starting point — the generator in `research/bench/meshing`.

## Specs

- [docs/architecture/worldgen.md](../../docs/architecture/worldgen.md)
- [docs/process/testing-and-benchmarks.md](../../docs/process/testing-and-benchmarks.md)

## Deliverables

- `worldgen::test_corpus(seed, area) -> columns`.
- Determinism test: different generation order and thread count → same hash.

## Acceptance criteria

- [x] Column hash does not depend on generation order (test).
- [x] Benchmarks T0-03/04/08 use this corpus. (T0-03: `world` bench `section_corpus`; T0-04/05: `mesh` benches. T0-08's benchmark does not exist yet — `light` is a skeleton; the wave plan in `tasks/README.md` binds it to this corpus.)

## Out of scope

This is not the game's final generator.
