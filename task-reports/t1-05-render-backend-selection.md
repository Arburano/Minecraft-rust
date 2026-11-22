# T1-05: renderer backend and adapter selection with fallback

Status: done (2026-11-22: logic, tooling, and hardware acceptance on owner's laptop with Vega 10 + GTX 1660 Ti complete).

## What was done

- **`render::selection`:**
  - `BackendKind`, `OsFamily`, `backend_order`: Windows Vulkan → DX12 → GL, Linux Vulkan → GL, macOS Metal.
  - `DeviceKind`, `AdapterCandidate`, and `GpuPreference` (backend, adapter-name substring, power preference; serde-serializable for client settings).
  - `attempt_order`, a pure function: the user's adapter name first, then the user's backend and the OS order, then GPU class by power preference, then name. Software adapters come after all hardware adapters. It never uses enumeration indices.
  - `select_device`: enumerates each backend with its own instance, so an empty Vulkan (AMD switchable-graphics layer, T0-14) does not affect DX12. It tries `request_device` along the attempt order and returns the device plus the list of failed attempts, or every failure.
- **`gpu-select` binary:** prints the chosen adapter and failed attempts as JSON; flags `--backend`, `--adapter`, `--low-power`.
- **`tests/gpu_select.rs`** (hardware checks):
  - `default_selection_finds_a_hardware_adapter`: passes on bare metal, chooses DX12 NVIDIA GTX 1660 Ti without environment variables.
  - `gl_device_exits_cleanly`: passes on bare metal, creates, uses, and drops a GL device, exiting cleanly with exit code 0.
- **Hardware Acceptance on Owner's Laptop (2026-11-22):**
  - Verified default selection (DX12 GTX 1660 Ti).
  - Verified `--low-power` and `--adapter Vega` selection (DX12 AMD Radeon RX Vega 10 Graphics).
  - Verified Vulkan fallback when 0 adapters enumerated on AMD switchable graphics (`--backend vulkan` falls back to DX12 without crash).
  - Verified GL backend selection (`--backend gl`) and clean shutdown.
  - Recorded in [`docs/research/gpu/gpu-select-2026-11-22.md`](../docs/research/gpu/gpu-select-2026-11-22.md).
- **Docs:** `meshing-and-rendering.md`.

## What went wrong

None. All hardware checks and fallback paths behaved exactly as specified.

## Issues encountered

- **AMD Switchable Graphics Vulkan Layer:** Vulkan continues to enumerate 0 adapters on this dual-GPU laptop. The per-backend instance enumeration in `select_device` completely solved this: Vulkan failures do not poison or prevent DX12 selection.
- **GL Exit Crash:** Did not reproduce under `tests/gpu_select.rs` and `gpu-select --backend gl`; process dropped GL resources and terminated with code 0. GL remains safely last in the Windows backend order.
- **Settings persistence** of `GpuPreference` and the settings screen are in T1-13.
