# T1-21: render reference scene and screenshot tests

## What was done

`crates/render/tests/reference_scene.rs`, golden images in `crates/render/tests/golden/` (14 PNG, 320×180, ≈ 0.7 MB in total).

**The scene** is built in code from the `core` + `overworld_plus` blocks, with fallback textures only. It is lit by `light::light_column` and meshed by `mesh::MeshChunkBuilder`, the same code the game uses. It contains:

- a grass floor;
- a material stand with one block of every kind;
- a "white furnace" (a closed snow room lit evenly by a glowstone grid);
- a light ramp (a closed corridor with glowstone at one end, block light 15 → 0);
- a sky-light shaft (a closed box with a 1×1 roof hole);
- a 6-deep water pool;
- a checker across the x = 0 / z = 0 mesh chunk and render batch borders; the room and the boxes are cut by the x = 32 borders too.

**Cameras:** 7 fixed cameras at noon (`overview`, `material_stand`, `white_furnace`, `light_ramp`, `sky_shaft`, `chunk_border`, `underwater`), with fixed fog.

**Tests:**

- `reference_scene_matches_golden_images_on_every_path`: renders every camera on paths Bi, B and C and compares each with one golden image per camera (written from Bi). A pixel differs when a channel differs by more than 8; at most 0.5% of pixels may differ.
- `shader_pack_has_its_own_golden_images`: the example pack `warm-glow` with its terrain hooks, its option defaults from `shaderpack.toml` and its post pass, against `warm-glow-<camera>.png`.
- `a_one_line_shading_change_fails_the_comparison`: a 30% darker surface shading must fail, with the message and artifacts.

On failure the test writes `target/render-diffs/<camera>-<path>-actual.png` and `-diff.png` (magenta pixels over the dimmed golden image), with a message such as "white_furnace-Pulling: 100.00% of pixels differ by more than 8 (max channel difference 137; allowed 0.50%). Actual: …, diff: …".

**Golden images** are refreshed only on purpose: `UPDATE_GOLDEN=1 cargo test -p render --test reference_scene`.

**Environment variables:**

- `REQUIRE_GPU=1` makes a missing adapter a failure instead of a skip.
- `REFERENCE_OUT=<dir>` writes every image.
- `ADAPTER_NAME` and `WGPU_BACKEND` pick the GPU.
- On a hardware GPU, mismatches are warnings unless `REFERENCE_STRICT=1`.

**CI** (`.github/workflows/ci.yml`): installs `mesa-vulkan-drivers` (lavapipe), runs the tests with `REQUIRE_GPU=1`, and uploads `target/render-diffs/` as the `render-diffs` artifact when a step fails.

**Docs:** `docs/process/testing-and-benchmarks.md`. "Without a GPU" now describes software Vulkan in CI. A new section "Render reference scene" covers the cameras, the comparison, refreshing, and comparing GPUs and RenderDoc on the laptop.

**Results** (cloud VM, llvmpipe LLVM 20.1.2, Vulkan; one run):

| Path | Pixels differing from the golden images | Max channel difference |
| --- | --- | --- |
| Bi | 0% | 0 |
| B | 0% | 0 |
| C | ≤ 0.003% (underwater) | 1–4 on 6 cameras, 10 on underwater |

- **Deliberate one-line change** in `terrain-common.wgsl`: the cell whose light a face reads, `floor(in.world + n * 0.5)` → `floor(in.world - n * 0.5)`. All 7 cameras failed on Bi (13.8–100% of pixels differ), and the first B results did too (I read only the first 8 messages), with the messages and diff images; the change was reverted.
- The self-test with 30% darker shading fails at 100% of pixels.
- Run time: all three tests ≈ 0.7 s on the VM after building.
- `cargo fmt --all`; `cargo clippy -p render --all-targets` with `-D warnings`; the render tests.

## What went wrong

- The first sky-shaft box was small and the camera looked past the lit spot, so the picture showed nothing specific. The box was enlarged and the camera aimed at the spot before the golden images were kept.
- The first self-test made the shading only 10% darker. After sRGB that is about 9 levels, so it failed by a hair (0.66% against 0.5%) and would be fragile. It is now 30% darker and fails at 100%.
- Artifacts of different paths overwrote each other (same file name); the path is now part of the name.

## Issues encountered

- **Owner check left** (acceptance): compare the scene on Vega 10 and GTX 1660 Ti with the golden images: `REFERENCE_OUT=ref-vega ADAPTER_NAME=vega cargo test -p render --test reference_scene -- --nocapture`, the same with `1660`.
- **RenderDoc and offscreen rendering:** RenderDoc captures at presentation, and the test has no window. The guide therefore captures the game client. Showing the reference scene in the client as a debug world would allow frame captures of the exact scene; that is a follow-up.
- **One texture per block on every face**, as in the screenshot example, so the grass floor shows the grass side texture (dirt with a green stripe) on top. This is the same in every golden image and does not hide regressions. Per-face textures (`client::textures`) live in the client crate, which `render` does not depend on.
- **Water** is drawn as an opaque cube in this renderer (no transparent pass yet). The `underwater` camera checks what exists; water surfaces and transparency get their own cameras with that work.
- **CI's lavapipe may be a different Mesa version** from the one that made the golden images (LLVM 20.1.2 here). If the first CI run shows differences above the tolerance, the images should be regenerated from CI's artifacts after looking at them. I could not run GitHub Actions from here.
- **Stage 1 exit (T1-15), for the record:** the laptop runs merged from `main` meet start time, FPS and RAM, but show 36–189 frames over 33 ms per 30 minutes (target 0) while flying over new terrain. That belongs to the T1-20 review, not to this task.
