# T0-12. Deterministic test terrain

- **Status:** todo
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

- [ ] Column hash does not depend on generation order (test).
- [ ] Benchmarks T0-03/04/08 use this corpus.

## Out of scope

This is not the game's final generator.
