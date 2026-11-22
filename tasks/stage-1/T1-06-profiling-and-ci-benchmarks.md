# T1-06. Profiler zones, Tracy and benchmarks in CI

- **Status:** in-progress (2026-11-21: code, CI job and overhead measurement done; the Tracy screenshot needs a desktop run on the owner's laptop)
- **Depends on:** T0-01, T0-02
- **Owner:** agent (cloud)

## Goal

Make performance visible every day: profiler zones in `engine-core` (tick phases, jobs, meshing, lighting, render), Tracy integration behind a feature flag, and benchmarks that run in CI with stored results.

## Specs

- [engine-core.md](../../docs/architecture/engine-core.md) — Profiling
- [testing-and-benchmarks.md](../../docs/process/testing-and-benchmarks.md)

## Deliverables

- `engine_core::profile` zones (zero cost when the feature is off; measured).
- Tracy client behind `--features tracy` in client and server; how-to in docs.
- CI job that runs a fast benchmark subset on every PR and uploads results as an artifact; a script that compares two result sets and flags > 10% regressions (advisory until a stable runner exists, per testing-and-benchmarks.md).

## Acceptance criteria

- [ ] (laptop) Zones visible in Tracy for a client run (screenshot in the report).
- [x] Feature-off overhead measured (≤ 1% on the light and mesh benches).
- [ ] CI benchmark job green and documented.

## Out of scope

A dedicated stable benchmark runner (hardware decision for the owner).
