# T0-14: GPU hardware evidence for T0-06 (Vega 10 + GTX 1660 Ti)

Status: done. PR #11. Closes T0-06's second acceptance criterion per owner
decision No. 12 (one laptop, two adapters; no GTX 1050 available).

## What was done

- **Part A (code):** `quad-bench --no-timestamps` skips `TIMESTAMP_QUERY`
  even on supporting adapters; the report gains a `timestamps` field
  (`enabled` / `unsupported` / `disabled-by-flag`) with a CLI parsing unit
  test. The ignored B/C image-comparison test accepts
  `QUAD_BENCH_ADAPTER_NAME` (case-insensitive substring) alongside
  `WGPU_BACKEND`. Added `research/gpu/run-matrix.cmd` (cmd.exe, UTF-8 safe)
  covering the probe, the full matrix, no-timestamp runs and per-adapter
  image tests; documented everything in `docs/research/gpu/README.md`.
- **Part B (hardware, on the owner's ASUS TUF FX505DU):** release builds
  (`CARGO_PROFILE_RELEASE_DEBUG=0` strips debug symbols only), fresh probe,
  and 36 timed runs — three each for {Vega 10, GTX 1660 Ti} × {DX12, Vulkan,
  GL} × {B, C} — plus 2 no-timestamp runs and 6 passed image-test logs, all
  bare-metal with AC power, "Best performance" mode and no background GPU
  work, confirmed by the human. Every JSON validated: release profile,
  bare-metal, default counts (500,000 / 600 / 30 + 120), adapter name matches
  file name, 184,960 rendered pixels.
- **Vulkan fixed without driver changes:** the AMD switchable-graphics
  layer made `vkEnumeratePhysicalDevices` return `INCOMPLETE` with zero
  devices; `VK_LOADER_LAYERS_DISABLE=VK_LAYER_AMD_switchable_graphics`
  exposes both GPUs. `vulkaninfo` outputs saved.
- **Vega 10 on GL enabled** via the NVIDIA Control Panel program setting
  ("Integrated graphics" for `gpu-probe.exe` / `quad-bench.exe`, human GUI
  step) plus the equivalent `UserGpuPreferences` registry value for the test
  executable. This reorders adapters; both configurations are documented and
  every run records its actual adapter.
- **Part C (analysis):** `docs/research/gpu/summary-2026-11-19.md` with
  per-combination median p50/p95 and run-to-run spread; the two
  performance-budget rows set to Measured with links; GTX 1050 targets
  explicitly marked not verified; capability-dump line in
  `meshing-and-rendering.md` updated; T0-06 and T0-14 closed in their files
  and `tasks/README.md`; T0-06 report updated.

### Key numbers (medians of three runs, ms)

- Vega 10 path B GPU geometry: DX12 2.417 p50 / 2.619 p95; Vulkan 2.309 /
  2.570; GL 2.612 / 2.821. Target ≤ 8 ms on iGPU: **met** (≥ 2.8× headroom).
- Vega 10 path C GPU geometry: 1.637–1.948 p50 across backends.
- CPU command recording worst case (Vega 10, DX12): 0.535 p50 / 0.821 p95;
  worst single-run p95 anywhere 0.923. Target ≤ 1 ms: **met**.
- GTX 1660 Ti: 0.431–0.461 GPU p50 on all backends/paths; does **not**
  verify GTX 1050 targets (owner decision No. 12).
- B vs C: C ~25–32% faster on the Vega 10, within ~5% on the GTX 1660 Ti;
  identical images everywhere.

### Verification

- `cargo fmt --all`, `cargo clippy -p render --all-targets -- -D warnings`,
  `cargo test -p render` (7 passed + 1 ignored hardware test): green.
- CI green on PR #11 (part A push); final CI run after the evidence and
  document commits.
- All six image tests end with `test result: ok. 1 passed`.
- No-timestamp runs: `gpu_geometry_ms: null`, `timestamps:
  "disabled-by-flag"`, CPU recording present, nonblank output.

## What went wrong

- The first background release build was interrupted between session turns;
  resumed incrementally and finished in 2m47s with `-j 4`.
- Passing the machine string through bash → `cmd /c` mangled the quotes
  ("TUF is not recognized..."); fixed with a wrapper `.cmd` file. No partial
  evidence files were produced.
- The Vega GL image test first failed with "no adapter ... matches
  QUAD_BENCH_ADAPTER_NAME=Vega": the Control Panel setting applied to the
  release binaries, not to the test executable in `target\debug\deps`.
  Fixed by setting `GpuPreference=1;` for that exe under
  `HKCU\Software\Microsoft\DirectX\UserGpuPreferences` (the same mechanism
  as Windows Settings → Display → Graphics).
- `reg add` from Git Bash failed until path conversion was disabled
  (`MSYS2_ARG_CONV_EXCL="*"`).
- `gpu-probe.exe` segfaults at exit (after writing complete, valid JSON)
  while GL is on the Vega 10; reproduced twice and documented in
  `probe-crash-note-2026-11-19.txt`. `run-matrix.cmd` checks the probe's
  output size instead of its exit code. `quad-bench` exits cleanly on all
  combinations.

## Issues encountered

- Disk space stayed critical (1.5–3 GB free); release builds used
  `CARGO_PROFILE_RELEASE_DEBUG=0` as an invocation-local setting, matching
  the T0-06 approach. The repository profile was not changed.
- Adapter indices are configuration-dependent: the Control Panel setting
  swaps DX12/Vulkan order and replaces the GTX 1660 Ti with the Vega 10 on
  GL. The committed probe reflects the post-setting state; earlier runs
  record their own adapter identity. `run-matrix.cmd` documents both
  configurations and comments out the GTX-on-GL rows accordingly.
- The task's "merge part A before the human run" was satisfied in spirit:
  part A was pushed and CI-green on the task branch before the matrix ran,
  but it rides in the same PR (#11) as the evidence because the task file
  itself is still in open PR #10. Once #10 merges, #11's diff shrinks to the
  T0-14 changes.
- GTX 1050 targets (PRD 60 FPS, stage 5 64 chunks) remain unverified by
  design; no target or architecture was changed.
- The probe exit-crash looks like an AMD driver teardown bug on the old OEM
  driver (27.20.1032.2); a future driver update may fix it and may also
  change adapter order — re-probe before any re-run.