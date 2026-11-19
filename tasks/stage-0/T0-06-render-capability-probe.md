# T0-06. GPU capability probe and vertex pulling prototype

- **Status:** in-progress
- **Depends on:** T0-04
- **Owner:** agent writes, human runs on hardware
- **PR:** https://github.com/Arburano/Minecraft-rust/pull/8 (merged; hardware acceptance pending)

## Goal

In the `render` crate: (1) a `gpu-probe` utility that prints `features`, `limits`, `downlevel_capabilities` for all adapters and backends (Vulkan, DX12, GL) as JSON; (2) a prototype that draws 0.5 million quads using path B (vertex pulling) or C (per-instance), measuring GPU time via timestamp queries where available.

## Specs

- [docs/architecture/meshing-and-rendering.md](../../docs/architecture/meshing-and-rendering.md)
- [docs/adr/0003-wgpu-own-renderer.md](../../docs/adr/0003-wgpu-own-renderer.md)

## Deliverables

- Binaries `cargo run -p render --bin gpu-probe` and `--bin quad-bench`.
- Path selection logic as a pure function with unit tests on fake capabilities (CI without GPU).
- Instructions for the human: how to run on the iGPU and GTX 1050, and where to put the JSON (`docs/research/gpu/`).

## Implementation and hardware follow-up

The prototype binaries, GPU-free path-selection/shader tests and an ignored
B/C image-comparison test are implemented. Run/save instructions and measurement
semantics are in [docs/research/gpu/README.md](../../docs/research/gpu/README.md).
The task remains `in-progress` until the required human hardware evidence and
green CI are available; no target-hardware numbers are assumed. Implementation
and verification details are in the [task report](../../task-reports/t0-06-render-capability-probe.md).

## Acceptance criteria

- [x] Unit tests for path selection pass in CI without a GPU (CI green on PR #8 and on `main` after the merge).
- [ ] Human has attached dumps and numbers from two machines; the "GPU geometry time" and "CPU command recording" rows are updated.

## Remaining work (review, 2026-11-19)

The agent part is done and merged. Only the hardware evidence is missing. Follow [docs/research/gpu/README.md](../../docs/research/gpu/README.md) exactly.

### Human

1. [ ] **Choose the two machines.** Required: one machine whose iGPU is the measured adapter, and one machine with a GTX 1050. The current laptop (Ryzen 7 3750H, Radeon RX Vega 10 + GTX 1660 Ti) is one machine: the GTX 1660 Ti is **not** a GTX 1050 substitute. The owner decides whether the Vega 10 of this laptop counts as "the iGPU machine"; if not, use a separate iGPU computer.
2. [ ] On each machine: release build (`cargo build --release -p render --bins`), AC power, stable power plan, no competing GPU work. Note CPU, RAM, OS, GPU driver versions, bare metal or VM.
3. [ ] Save `gpu-probe` output as `docs/research/gpu/<machine>-probe-YYYY-MM-DD.json` (in PowerShell 5.1 use the `cmd.exe /c` redirection from the README — UTF-16 output is invalid).
4. [ ] **Vulkan returned no adapters on the laptop.** Before measuring, check that the GPU vendor driver with Vulkan is installed (e.g. `vulkaninfo --summary` from the Vulkan SDK/runtime). If Vulkan is still unavailable, record that as an unsupported combination with the driver details — do not drop it silently.
5. [ ] For every supported backend (`vulkan`, `dx12`, `gl`) × path (`b`, `c`): **three** `quad-bench` runs with default arguments, `--machine "..."` and `--environment bare-metal`, using the adapter index from that backend's probe (indices are per backend; never assume 0). Save each as `<machine>-<backend>-<path>-YYYY-MM-DD-runN.json`.
6. [ ] Run at least one combination where `TIMESTAMP_QUERY` is unavailable (often GL) and confirm GPU fields are `null`, not CPU time — this path was never run on hardware.
7. [ ] Run the ignored image-comparison test on each machine: `cargo test -p render --bin quad-bench gpu_paths_render_identical_nonblank_images -- --ignored` (with `WGPU_BACKEND` per backend).
8. [ ] Commit the JSON files (or hand them to the agent) on a branch `t0-06-hardware-evidence`.

### Agent (after the JSON files are in the repository)

1. [ ] Optional, before the human run: add a script (e.g. `research/gpu/run-matrix.ps1` / `.sh`) that runs the probe and the 3× backend/path matrix and writes correctly named UTF-8 files, to avoid manual mistakes. No change to the measured code.
2. [ ] Validate every JSON: `build_profile` is release, `environment` is `bare-metal`, frame counts are the defaults, `rendered_pixels` > 0, adapter name matches the intended GPU.
3. [ ] Write `docs/research/gpu/summary-YYYY-MM-DD.md`: per machine/backend/path the p50/p95 of `gpu_geometry_ms` and `cpu_command_recording_ms`, run-to-run spread over the 3 runs, B vs C on the same adapter, unsupported combinations with reasons.
4. [ ] Update the "GPU geometry time" (≤ 8 ms on iGPU) and "CPU command recording" (≤ 1 ms) rows in `docs/process/performance-budgets.md` with links to the evidence. If a target is not met, **stop and ask the owner** (AGENTS.md) — do not change the budget or the architecture; if the owner changes a target, update the PRD identically.
5. [ ] Update `task-reports/t0-06-render-capability-probe.md` with the results, set the second acceptance criterion, status `done` here and in `tasks/README.md`, CI green.

## Out of scope

No compute culling (path A) — stage 2.
