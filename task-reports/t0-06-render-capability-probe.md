# T0-06: GPU capability probe and vertex pulling prototype

Status: agent implementation locally verified. The task remains `in-progress`
pending human evidence from two target machines and green CI.

## What was done

- Added `render::select_path` and serializable adapter capability reports.
  Stage 0–1 selects B with vertex storage, otherwise C; A is not implemented.
- Added `gpu-probe`, enumerating Vulkan, DX12 and GL adapters separately and
  reporting features, limits, downlevel capabilities, hardware and driver
  identity as JSON.
- Added `quad-bench`: a deterministic synthetic 8-byte quad corpus, B storage
  pulling and C `Uint32x2` instancing, 500,000 quads/600 draws by default, and
  1280×720 offscreen rendering. The B/C shaders share geometry decoding, but C
  never declares/binds storage. The prototype does not integrate T0-05 batch
  assembly; it isolates rendering of the specification's synthetic quad format.
- GPU timing uses optional pass timestamps; unavailable timing is `null`, not
  a CPU estimate. CPU command recording is measured separately. Reports retain
  raw frame samples, warmup count, percentiles, machine/environment/build
  metadata and an untimed nonblank-image verification.
- Added GPU-free capability, draw-range, data-format, WGSL-validation, CLI and
  statistics tests; an ignored hardware test compares complete B/C images.
- Added `docs/research/gpu/README.md` with human run/save instructions and the
  exact measurement boundaries. Updated research/task indexes and the two
  performance-budget rows, retaining their hypothesis status and unchanged
  targets. No target-hardware performance claim has been made.
- Dependencies use workspace versions: the already prescribed wgpu 30 supplies
  the renderer/backends; `serde_json` supplies JSON; zero-dependency `pollster`
  blocks on the small initialization futures instead of adding a runtime.
  Naga 30 is a direct test dependency already present transitively in wgpu,
  allowing shader validation without a GPU. These crates use MIT/Apache-2.0
  compatible licenses.

### Verification

- `cargo fmt --all --check`: passed.
- `cargo clippy --workspace --all-targets -j 2 -- -D warnings`: passed.
- `cargo bench --workspace --no-run -j 2`: passed with the unchanged release
  profile (thin LTO and line-table debug information).
- `cargo build --release -p render --bins -j 2`: passed.
- `cargo test --workspace`: passed, 55 unit/property tests and one doctest;
  the GPU-only image comparison is ignored by default. This includes all
  12 new GPU-free renderer tests, including validation of both WGSL variants.
- `cargo test -p render` passed again after making the ignored hardware test
  backend-selectable. The ignored B/C full-image comparison passed separately
  with `WGPU_BACKEND=dx12` and `WGPU_BACKEND=gl` (one invocation each).
- Both debug utilities built, and `gpu-probe` emitted valid JSON. The saved
  [local probe](../docs/research/gpu/local-probe-2026-11-18.json) lists GTX 1660 Ti,
  Radeon RX Vega 10 and the software adapter on DX12, plus GTX 1660 Ti on GL.
  Vulkan returned no adapters in this environment.
- Four debug smoke invocations succeeded: DX12 B and C on GTX 1660 Ti, GL C
  on GTX 1660 Ti, and DX12 auto (B) on Radeon RX Vega 10. Each used 500,000
  quads, 600 draws, two warmup frames and five measured frames; each produced
  184,960 nonblack pixels and five finite GPU timing samples. Raw JSON is
  saved beside the probe with `local-*-smoke.json` names.
- Minimum-size DX12 B/C runs (one quad/draw/frame, no warmup) succeeded.
  Invalid draw/quad counts, zero frames, unavailable Vulkan and an out-of-range
  adapter index failed with explanatory errors and no misleading JSON output.
- These smoke reports say `build_profile: debug` and `environment: unknown`.
  Power/clock settings and background load were not controlled. Their timings
  are not acceptance evidence or a basis for changing the performance budgets.
  Runtime testing on an adapter without timestamp support remains pending.

## What went wrong

- Rust was not installed. With the user's permission, installed the pinned Rust
  1.97 toolchain (rustc 1.97.1); existing Visual Studio 2019 C++ build tools were
  available. The repository toolchain file was not changed.
- A full cross-target Cargo fetch encountered TLS connection closures while
  downloading irrelevant GNU/platform packages. Fetching the Windows MSVC
  target with HTTP multiplexing disabled obtained the required dependencies.
- Initial compilation found two wgpu 30 API mismatches: mapped-buffer views are
  fallible and storage binding sizes are `u64`. Both were corrected. A shader
  test's dev dependency was added after that first build started; the retry
  uses the updated manifest.
- Windows linking failed with `LNK1318` while writing a PDB; the system had only
  about 2 GB free. Removed only build artifacts generated in this fresh clone
  and retried with `CARGO_PROFILE_DEV_DEBUG=0` and
  `CARGO_PROFILE_DEV_INCREMENTAL=false`. These are invocation-local settings,
  not repository configuration changes.
- The first release benchmark compilation was interrupted with Windows process
  exit 1073807364 and linker exit 0xc000026b across several crates. A retry
  uses two Cargo jobs without changing the repository's release profile.

## Issues encountered

- The available machine is an ASUS TUF Gaming FX505DU, AMD Ryzen 7 3750H
  (4 cores/8 threads), about 14 GiB usable RAM, Radeon RX Vega 10 iGPU
  (Windows driver 27.20.1032.2) and GTX 1660 Ti (driver 32.0.15.5597).
  It is not a GTX 1050 machine, and testing both adapters in one laptop is not
  evidence from two machines. Any local runs are smoke tests, not acceptance
  measurements or replacements for the requested target hardware.
- Human follow-up remains mandatory: capability dumps and three comparable
  runs per supported backend/path from an iGPU machine and a GTX 1050 machine,
  with machine, environment, driver, clock/power and spread details. Update
  the performance-budget rows from that evidence before marking T0-06 `done`.
- This is an offscreen geometry prototype, not a complete textured/lit renderer
  or a pipelined game-frame benchmark. GPU-driven culling remains out of scope.
