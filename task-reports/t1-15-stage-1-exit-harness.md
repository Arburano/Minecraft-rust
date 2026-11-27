# T1-15. Stage 1 exit measurement — report (harness part)

## What was done

- **`crates/client/src/bench.rs`** (new): a scripted route.
  - Starts a fresh world with seed 12345, then jumps and switches flight on.
  - Flies a square route with 1000-block sides at fly speed 0.5, 12 blocks above the ground taken from the height map. New terrain keeps streaming in, which is the worst case for the loader.
  - After a 30 s warm-up it records every frame time and samples process RAM once per second (`memory-stats`).
  - Writes `frames-<time>.csv` and `summary-<time>.txt` to `<data>/bench/`. The summary has average FPS, 1% low, p50/p99/max, frames over 33 ms, RAM peak and average, the adapter and the profile.
  - The unit test `summary_numbers` checks the statistics.
- **`client` options:**
  - `--bench-route <SECONDS>`;
  - `--gpu <NAME>`, which overrides the adapter for one run;
  - `--safe-mode`;
  - in bench mode the focus-loss pause is off, and the bench world is deleted at the end.
- **`Session::set_fly_speed`:** clamped to the physics profile's range.
- **Dependency:** `memory-stats = "1"` (tiny; reads the process working set on Windows/Linux/macOS without `unsafe` in our code). The justification is in `Cargo.toml`.
- **`scripts/stage1_exit.ps1`:** builds, runs 3× on the Vega and 1× on the GTX 1660 Ti.
- **Docs and tasks:**
  - task file with laptop instructions;
  - T1-12 closed;
  - T1-20 (stage 1 exit review) created;
  - stage 2 backlog `tasks/stage-2/T2-01 … T2-09` plus waves in `tasks/README.md`;
  - roadmap note on the owner's 2026-11-23 decision.
- **Cloud check** (VM, Xvfb + mesa llvmpipe, one 60 s run): 333 blocks flown at ≈ 5.5 blocks/s, 58 memory samples, RAM 353–374 MiB. Software rendering, so no FPS conclusions.
- **Checks:** `cargo fmt`, `cargo clippy --workspace --all-targets` with `-D warnings`, and `cargo test -p client` all pass.

## What went wrong

- **The player did not move in the first cloud run (0 blocks).** Under Xvfb the window never gets focus, so the game paused itself. Fixed by turning off the focus pause in bench mode. The fly speed is now set once with `set_fly_speed` rather than by simulating wheel input.
- **The disk filled up** (`target/` at 58 GB) and the linker crashed with a Bus error. Fixed by deleting `target/debug`.

## Issues encountered

- **Laptop exit numbers recorded (2026-11-27):** see [docs/research/stage-1-exit-2026-11-27/summary.md](../docs/research/stage-1-exit-2026-11-27/summary.md). Start-to-menu (1.31–1.67 s), FPS (257.9–295.8 avg / 50.9–103.0 1% low on Vega 10, `High` 16 chunks), and RAM (peak 324–346 MiB) all meet their targets; frames > 33 ms had 36–189 spikes on Vega 10 and 98 on GTX 1660 Ti (target 0).
- **GTX 1050 targets stay unverified** (open question 12).
- **The owner decided to start stage 2 before these runs.** The stage 1 exit review is T1-20.
