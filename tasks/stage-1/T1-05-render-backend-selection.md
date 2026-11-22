# T1-05. Renderer backend and adapter selection with fallback

- **Status:** in-progress (2026-11-21: selection logic, wgpu layer, `gpu-select` and hardware tests done; laptop runs pending)
- **Depends on:** T0-14
- **Owner:** agent (cloud) + owner's laptop for hardware checks

## Goal

Make the renderer start on any supported machine without user tweaks. T0-14 showed: Vulkan can enumerate zero adapters on AMD switchable-graphics laptops, the NVIDIA preferred-GPU setting reorders adapters, and the probe crashed at exit with GL on the Vega 10.

## Specs

- [meshing-and-rendering.md](../../docs/architecture/meshing-and-rendering.md) — platform findings
- [GPU summary](../../docs/research/gpu/summary-2026-11-19.md)
- [ADR-0003](../../docs/adr/0003-wgpu-own-renderer.md)

## Deliverables

- `render::select_backend_and_adapter`: try Vulkan → DX12 (Windows) → GL, fall back when a backend returns no adapters or fails device creation; choose the adapter by type/capabilities/user setting, never by index. Pure logic with unit tests on fake adapter lists (CI without GPU).
- User override in the client settings (backend + adapter name), persisted.
- GL shutdown test on the Vega 10 (ignored test, run by the owner's local agent); fix or document the driver crash.
- No environment variables required from players.

## Acceptance criteria

- [x] Unit tests for selection pass in CI.
- [ ] On the owner's laptop the client starts with default settings on Vega 10 and on GTX 1660 Ti, with and without the NVIDIA program setting (results file in docs/research/gpu/).
- [ ] GL exit crash fixed or isolated with a documented workaround.

## Out of scope

Path A / GPU-driven culling (stage 2+).
