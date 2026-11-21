# T1-05: renderer backend and adapter selection with fallback

Status: logic and tooling done in the cloud; the hardware acceptance runs (laptop with Vega 10 + GTX 1660 Ti) are pending.

## What was done

- **`render::selection`:**
  - `BackendKind`, `OsFamily`, `backend_order`: Windows Vulkan → DX12 → GL, Linux Vulkan → GL, macOS Metal.
  - `DeviceKind`, `AdapterCandidate`, and `GpuPreference` (backend, adapter-name substring, power preference; serde-serializable for client settings).
  - `attempt_order`, a pure function: the user's adapter name first, then the user's backend and the OS order, then GPU class by power preference, then name. Software adapters come after all hardware adapters. It never uses enumeration indices.
  - `select_device`: enumerates each backend with its own instance, so an empty Vulkan (AMD switchable-graphics layer, T0-14) does not affect DX12. It tries `request_device` along the attempt order and returns the device plus the list of failed attempts, or every failure.
- **`gpu-select` binary:** prints the chosen adapter and failed attempts as JSON; flags `--backend`, `--adapter`, `--low-power`.
- **`tests/gpu_select.rs`** (ignored without a GPU):
  - default selection finds a hardware adapter;
  - selecting GL creates and drops a device, and the separate process exits cleanly. This is the T0-14 exit-crash check.
- **Docs:** `meshing-and-rendering.md`.

**Tests (CI, no GPU):** 4 unit tests built on the owner's laptop from T0-14:
- Vulkan empty → DX12 GTX 1660 Ti, Vega 10, then GL, software last — the same with the DX12 order swapped;
- low power and name/backend overrides reorder the list without dropping fallbacks;
- with Vulkan present it comes first and software adapters go last;
- empty input, OS orders, and the serialization format.

In the container `gpu-select` reports "no usable GPU adapter (0 attempts)" and exits with an error, which is expected without a GPU.

## What went wrong

None.

## Issues encountered

- **Pending on the laptop** (owner's local agent):
  - `cargo test --release -p render --test gpu_select -- --ignored --nocapture`, with and without the NVIDIA "Integrated graphics" program setting for the test binary, and with `--backend gl`;
  - save the outputs under `docs/research/gpu/` and tick the acceptance criteria.
  - If the GL exit crash reproduces, isolate it: for example, prefer DX12 over GL on AMD iGPUs, or skip GL when another backend works. It is already last in the default order.
- **No in-process environment-variable workaround** for the AMD Vulkan layer: `std::env::set_var` is `unsafe` in Rust 2024 (AGENTS.md rule 4), and the DX12 fallback makes it unnecessary.
- **Settings persistence** of `GpuPreference` and the settings screen are T1-13.
