# Hardware Adapter & Backend Selection Evidence (T1-05, 2026-11-22)

This document records the hardware acceptance verification of `render::selection`
and the `gpu-select` tool on the owner's dual-GPU laptop.

## Hardware & Environment

- **Machine:** ASUS TUF Gaming FX505DU, AMD Ryzen 7 3750H, 13.94 GiB RAM, Windows 11.
- **GPUs:**
  1. Discrete: NVIDIA GeForce GTX 1660 Ti Mobile (6 GiB VRAM)
  2. Integrated: AMD Radeon(TM) RX Vega 10 Graphics (2 GiB shared)
- **Rust Toolchain:** 1.97.1 pinned, release profile.

## Integration Tests (`tests/gpu_select.rs`)

Command: `cargo test --release -p render --test gpu_select -- --ignored --nocapture`

Output:
```text
running 2 tests
{
  "chosen": {
    "backend": "gl",
    "device": "Other",
    "name": "NVIDIA GeForce GTX 1660 Ti/PCIe/SSE2",
    "path": "B"
  },
  "failed_before": []
}
test gl_device_exits_cleanly ... ok

{
  "chosen": {
    "backend": "dx12",
    "device": "Discrete",
    "name": "NVIDIA GeForce GTX 1660 Ti",
    "path": "B"
  },
  "failed_before": []
}
test default_selection_finds_a_hardware_adapter ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.25s
```

**Results:**
- `default_selection_finds_a_hardware_adapter`: Successfully selected hardware adapter (DX12 GTX 1660 Ti) without any environment variables.
- `gl_device_exits_cleanly`: Successfully created, used, and dropped a GL device, and the process exited cleanly with code 0 (no driver crash at process exit).

## Binary CLI Selection Tests (`gpu-select`)

### 1. Default Selection (No Flags)
```json
{
  "chosen": {
    "backend": "dx12",
    "device": "Discrete",
    "name": "NVIDIA GeForce GTX 1660 Ti",
    "path": "B"
  },
  "failed_before": []
}
```

### 2. Low-Power Selection (`--low-power`)
```json
{
  "chosen": {
    "backend": "dx12",
    "device": "Integrated",
    "name": "AMD Radeon(TM) RX Vega 10 Graphics",
    "path": "B"
  },
  "failed_before": []
}
```

### 3. Vulkan Requested on Switchable Graphics (`--backend vulkan`)
Vulkan enumerates 0 adapters due to the AMD switchable graphics layer. The selection logic seamlessly falls back to DX12 without crashing or requiring user tweaks:
```json
{
  "chosen": {
    "backend": "dx12",
    "device": "Discrete",
    "name": "NVIDIA GeForce GTX 1660 Ti",
    "path": "B"
  },
  "failed_before": []
}
```

### 4. Adapter Name Substring Override (`--adapter Vega`)
```json
{
  "chosen": {
    "backend": "dx12",
    "device": "Integrated",
    "name": "AMD Radeon(TM) RX Vega 10 Graphics",
    "path": "B"
  },
  "failed_before": []
}
```

### 5. GL Backend Requested (`--backend gl`)
```json
{
  "chosen": {
    "backend": "gl",
    "device": "Other",
    "name": "NVIDIA GeForce GTX 1660 Ti/PCIe/SSE2",
    "path": "B"
  },
  "failed_before": []
}
```
Exited cleanly with status code 0.
