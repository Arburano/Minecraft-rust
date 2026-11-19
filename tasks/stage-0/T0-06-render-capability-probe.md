# T0-06. GPU capability probe and vertex pulling prototype

- **Status:** in-progress
- **Depends on:** T0-04
- **Owner:** agent writes, human runs on hardware

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

- [ ] Unit tests for path selection pass in CI without a GPU.
- [ ] Human has attached dumps and numbers from two machines; the "GPU geometry time" and "CPU command recording" rows are updated.

## Out of scope

No compute culling (path A) — stage 2.
