# T1-06: profiler zones, Tracy and benchmarks in CI

Status: code, CI workflow and overhead measurements done. The acceptance screenshot of zones in Tracy needs a desktop run on the owner's laptop (a cloud container has no display or Tracy GUI).

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
- **Docs:** `engine-core.md` (Profiling), `testing-and-benchmarks.md` (CI benchmarks).

**Overhead** — cloud **VM**, 4 vCPU Xeon 2.1 GHz. Criterion `--save-baseline` with the zone lines removed (`git stash`), then `--baseline` with zones; 30 samples, 3 s, 1 run each.

| Bench | Zones, feature off | Zones, feature on, no Tracy client |
| --- | --- | --- |
| `light/add_torch` | −2.9% (no change, p = 0.25) | +9.6% |
| `light/remove_torch` | −11.7% | −7.9% |
| `light/full_relight_3x3x3` | −4.9% | +1.1% (no change, p = 0.70) |
| `mesh_chunk/build` | −6.0% | — |
| `mesh_chunk/remesh` | +0.3% (no change, p = 0.82) | — |

With the feature off, the zones compile to nothing; the differences are VM noise, in both directions. With the feature on, the mixed signs show no consistent cost at this noise level.

## What went wrong

- The first baseline run passed several filters to Criterion, which accepts only one, so no baseline was saved; it was redone with a single regex.
- The first mesh baseline used non-existent bench names (`full_build`, `remesh_after_edit`); the correct names are `build` and `remesh`.

## Issues encountered

- **Tracy screenshot (acceptance item):**
  - **For the owner's local agent:** run `cargo run --release -p client --features profiling -- --offline-name Dev` with the Tracy GUI version that matches `tracy-client` 0.19 (version table in its README).
  - **Limitation:** the client binary does almost nothing yet, so the meaningful zones appear once T1-10/T1-11/T1-13 exist. A bench binary built with the feature (for example the `light` bench) shows the `light` zones today.
- **Spec deviation:** `engine-core.md` mentions `tracing` zones. Profiling uses `tracy-client` spans directly, which compile to nothing when off. `tracing` remains for logs. No ADR is involved.
- **The bench workflow is advisory;** a stable runner is a hardware decision for the owner. It also roughly doubles the CI minutes per PR.
- **The workflow has not run yet:** its first run happens on this PR, and the comparison step starts working after the first green run on `main`.
