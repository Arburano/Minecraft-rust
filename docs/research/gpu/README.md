# T0-06: GPU probe and quad benchmark

The task remains **in-progress** until a human supplies measurements from two
machines (an iGPU and a GTX 1050) and updates the GPU geometry and CPU recording
rows in `docs/process/performance-budgets.md`.

**Owner decision (open question No. 12):** no GTX 1050 is available. The evidence
is taken on one laptop: Radeon RX Vega 10 as the iGPU and GTX 1660 Ti instead of
the GTX 1050. 1660 Ti numbers do not verify GTX 1050 targets. The step-by-step
run is [T0-14](../../../tasks/stage-0/T0-14-gpu-hardware-evidence.md).

## Build and identify adapters

From the repository root, using the pinned Rust toolchain:

```text
rustc --version
cargo build --release -p render --bins
cargo run --release -p render --bin gpu-probe
```

The probe enumerates Vulkan, DX12 and GL separately, without requesting a device.
JSON includes hardware/driver identity, `features`, `limits`,
`downlevel_capabilities`, and the selected stage 0–1 path. `adapter_index` is
**local to a backend**: look up the intended GPU separately for each backend.
An unavailable backend contributes no adapters; it does not imply that the
other backends failed. GL may expose only the GPU chosen by the driver.

Save stdout under this directory, for example
`igpu-probe-YYYY-MM-DD.json` and `gtx1050-probe-YYYY-MM-DD.json`. Diagnostics and
Cargo output stay on stderr. In Windows PowerShell 5.1, use `cmd.exe` redirection
to avoid its default UTF-16 encoding:

```powershell
cmd.exe /c "cargo run --release -p render --bin gpu-probe > docs/research/gpu/igpu-probe-YYYY-MM-DD.json"
```

Never assume adapter 0 is the desired GPU: software adapters may also be listed.
On a hybrid laptop, select the appropriate adapter index rather than relying on
the OS power preference. If the intended GL GPU is unavailable, record that
limitation and the driver configuration instead of silently using another GPU.

## Measure B and C on each machine

Use a release build, AC power, stable power/clock settings, no competing GPU
work, and record CPU model, RAM, OS, driver version and whether the machine is
bare metal or a VM. Run each supported backend/path combination **three times**.
The defaults are 500,000 quads, 600 draw commands, a 1280×720 offscreen RGBA8
target, 30 warmup frames and 120 measured frames.

For each of `vulkan`, `dx12`, and `gl`, substitute the probe's adapter index and
run both paths. This example selects Vulkan adapter 0; change it after inspecting
the dump:

```text
cargo run --release -p render --bin quad-bench -- --backend vulkan --adapter 0 --path b --machine "CPU; RAM; OS; driver versions" --environment bare-metal
cargo run --release -p render --bin quad-bench -- --backend vulkan --adapter 0 --path c --machine "CPU; RAM; OS; driver versions" --environment bare-metal
```

Use `--environment vm` for a VM. Without an explicit value, the report says
`unknown`; do not promote such a run to bare-metal evidence. `--path auto`
selects B only with `VERTEX_STORAGE`, otherwise C. Forcing B on an adapter
without vertex storage fails with an explanatory error. Path C does not declare
or bind a vertex storage buffer. Path A/compute culling is not implemented.

Save one JSON file per run, e.g.
`igpu-vulkan-b-YYYY-MM-DD-run1.json` or
`gtx1050-dx12-c-YYYY-MM-DD-run3.json`. In PowerShell 5.1:

```powershell
cmd.exe /c 'cargo run --release -p render --bin quad-bench -- --backend vulkan --adapter 0 --path b --machine "CPU; RAM; OS; driver versions" --environment bare-metal > docs/research/gpu/igpu-vulkan-b-YYYY-MM-DD-run1.json'
```

Record unsupported backend/path combinations explicitly in the measurement
summary, with the failure/driver details. Do not attach an empty file as a
successful result.

## What the numbers measure

- Both paths use the same synthetic 8-byte packed quads and batch transforms.
  Positions have 6 bits per axis, width/height are stored minus one, and the
  material index occupies the low 16 bits of the second word. All generated
  quads are unit-sized. This is a rendering-contract corpus, not an integration
  with `mesh::Quad` or T0-05 batch assembly.
- B draws triangle lists: `vertex_index / 6` selects a quad in one storage buffer.
  Each draw uses a vertex range in that buffer, without `first_instance`.
- C draws four-vertex triangle strips, one `Uint32x2` instance per quad. Each
  draw binds its own byte range; it uses neither vertex storage nor nonzero
  `first_instance`.
- B executes 3,000,000 vertex invocations at the default count; C executes
  2,000,000. Both produce 1,000,000 triangles. Batch tiles use a 64×64 unit-quad
  grid. More than 4,096 quads in a draw repeats screen positions, increasing
  overdraw; changing `--draws` changes the workload and must be disclosed.
- `gpu_geometry_ms` uses render-pass beginning/end timestamps, converted using
  `timestamp_period_ns`. It includes the clear, rasterization and color writes,
  but excludes query resolution, readback and synchronization. If
  `TIMESTAMP_QUERY` is unavailable, these fields are `null`; CPU wall time is
  never mislabeled as GPU time. The `timestamps` field says why:
  `"enabled"`, `"unsupported"` (adapter lacks the feature) or
  `"disabled-by-flag"` (`--no-timestamps` was passed). `--no-timestamps` does
  not request `TIMESTAMP_QUERY` even on adapters that support it, so the
  `gpu_geometry_ms: null` path can be exercised on real hardware; nothing else
  about the measured workload changes.
- `cpu_command_recording_ms` measures encoder creation, pass/draw recording,
  optional timestamp resolve/copy recording and encoder finish. It excludes
  queue submission, GPU waits, initialization and image readback. Application
  buffers and sample vectors are allocated before the measured frame loop;
  wgpu manages its own command-encoder allocation internally.
- Frames are submitted and completed serially to isolate samples. This is not
  a pipelined game-frame benchmark. No textures, lighting, depth, culling,
  gameplay or window/presentation are included.
- Both timing fields include raw samples and mean/min/p50/p95/max in
  milliseconds. Warmup samples are excluded. `rendered_pixels` is checked in
  an untimed readback so a blank render cannot be reported as success.

## Tests

GPU-free tests cover fake capability flags, draw partitioning, 8-byte data,
shader validation, CLI bounds and timing statistics:

```text
cargo test -p render
```

An ignored hardware test renders both paths and compares the complete nonblank
images, including multiple draw ranges:

```text
cargo test -p render --bin quad-bench gpu_paths_render_identical_nonblank_images -- --ignored
```

It uses the first enumerated Vulkan/DX12/GL adapter with vertex storage. Set
`WGPU_BACKEND=dx12`, `vulkan`, or `gl` to restrict the backend for this test.
Set `QUAD_BENCH_ADAPTER_NAME` to a case-insensitive substring of the adapter
name (e.g. `Vega` or `1660`) to pick a specific GPU; without it the first
adapter with vertex storage wins, which on DX12 is usually the discrete GPU.
It is deliberately ignored in GPU-free CI.

## Run the whole T0-14 matrix

On the laptop, from the repository root in **cmd.exe** (not PowerShell 5.1):

```bat
cargo build --release -p render --bins
research\gpu\run-matrix.cmd DATE "MACHINE STRING"
```

The script saves the probe, three runs per adapter/backend/path, the
`--no-timestamps` runs and the per-adapter image-test logs under this directory
with the T0-14 file names. Re-check its adapter indices against the fresh probe
first; Vulkan rows are not included until Vulkan works (see T0-14 part B).

On the FX505DU, `VK_LAYER_AMD_switchable_graphics` makes
`vkEnumeratePhysicalDevices` return `INCOMPLETE` with no devices, which is why
the 2026-11-18 probe saw no Vulkan adapters. Setting
`VK_LOADER_LAYERS_DISABLE=VK_LAYER_AMD_switchable_graphics` (the script does)
exposes both the GTX 1660 Ti and the Vega 10 without any driver change.

## Local smoke evidence (not acceptance)

`local-probe-2026-11-18.json` and `local-*-smoke.json` were captured on one ASUS
FX505DU laptop (Ryzen 7 3750H, about 14 GiB usable RAM, Windows 10.0.28000).
DX12 enumerated GTX 1660 Ti, Radeon RX Vega 10 and a software adapter; GL exposed
GTX 1660 Ti; Vulkan returned no adapters. The smoke reports cover DX12 B/C on
GTX 1660 Ti, GL C on GTX 1660 Ti and DX12 auto/B on Radeon RX Vega 10.

Each is **one debug invocation**, two warmup and five measured frames, with
500,000 quads and 600 draws. All produced 184,960 nonblack pixels. Environment
is explicitly `unknown`, and clock/power/background load were not controlled.
Do not compare these timings against acceptance budgets. They do not replace
three release runs on each of the two requested target machines.

## Close the human part

Attach capability dumps and all three runs per supported backend/path on both
machines, plus a dated summary documenting hardware, versions, power settings,
run-to-run spread and unsupported combinations. Compare B and C on the same
adapter/backend with identical arguments. Check the GPU geometry target
(≤ 8 ms on the iGPU) and CPU recording target (≤ 1 ms); if a target appears
unreachable, follow `AGENTS.md` and consult the owner rather than changing the
architecture or budget silently.

Update the two performance-budget rows with links to the actual evidence. If a
target changes, update the PRD identically. Only then, with green CI, change
T0-06 to `done` in both task files and link the PR.
