# T1-06: profiler zones, Tracy and benchmarks in CI

Status: done (2026-11-22: code, CI workflow, overhead measurements, and laptop desktop verification complete).

## What was done

- **`engine_core::profile`:**
  - `zone!("name")` macro. It is defined in two variants chosen when `engine-core` compiles: without the `profiling` feature it expands to nothing; with it, it opens a `tracy-client` span while a client is running.
  - `start()` returns a guard that keeps the Tracy client alive; `enabled()` reports whether zones are compiled in.
  - `tracy-client` (already in `[workspace.dependencies]`) is an optional dependency behind `profiling`.
- **Zones:** `jobs::run` (every scheduler job), `sim::tick` with `pre_tick` / `simulate` / `exchange`, `mesh::build`, `mesh::remesh`, `light::full_relight`, `light::relight`, `modhost::call`. `mesh` and `light` now depend on `engine-core`, a downward dependency.
- **`client` and `server`:** a `profiling` feature forwarding to `engine-core/profiling`; `main` starts the profiler.
- **CI:**
  - `.github/workflows/bench.yml` runs the `world` (section, map), `mesh`, `light` and `assets` benches on every PR and on `main` with short settings, and uploads `bench-results.json`. On PRs it downloads the latest successful `main` artifact with `gh` and writes a comparison table to the job summary.
  - `scripts/bench_collect.py` gathers Criterion `new/estimates.json` means; `scripts/bench_compare.py` builds the Markdown table and flags changes above 10%. It is advisory by default; `--strict` exits 1. Both scripts were checked locally on real Criterion output and a synthetic base, and `--strict` exits 1 on regressions.
- **Desktop / Laptop Verification (2026-11-22):**
  - Tested `cargo test -p engine-core --features profiling`: passes (`test profile::tests::zones_compile_in_any_configuration ... ok`).
  - Built and executed `cargo run --release -p client --features profiling -- --offline-name Dev`: client successfully initializes Tracy client guard and starts up offline mode.
- **Docs:** `engine-core.md` (Profiling), `testing-and-benchmarks.md` (CI benchmarks).

## What went wrong

- When compiling `tracy-client-sys` on Windows with MSVC 2019 BuildTools, the build initially failed because Windows 10 SDK headers older than 10.0.19041 lack ETW constants (`SYSTEM_PROCESS_KW_THREAD`, `SystemSchedulerProviderGuid`) and `RelationProcessorDie`.
- Resolution: passed `-DTRACY_NO_SYSTEM_TRACING -DTRACY_NO_CONTEXT_SWITCH -DRelationProcessorDie=LOGICAL_PROCESSOR_RELATIONSHIP(5)` via `TRACY_CLIENT_SYS_CXXFLAGS`, allowing clean compilation and execution on Windows without requiring SDK upgrades.

## Issues encountered

- **Tracy GUI integration:** Tracy GUI is an external desktop application (standalone GUI binary). The client binary successfully embeds and initializes the Tracy client server component; as noted in the report, heavier zone traces will become visible once T1-10/T1-11/T1-13 connect the client render loop.
- **Spec deviation:** `engine-core.md` mentions `tracing` zones. Profiling uses `tracy-client` spans directly, which compile to nothing when off. `tracing` remains for logs. No ADR is involved.
- **The bench workflow is advisory;** a stable runner is a hardware decision for the owner. It also roughly doubles the CI minutes per PR.
