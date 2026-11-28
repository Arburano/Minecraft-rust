# Tests and benchmarks

## Kinds of tests

| Kind | Where | When mandatory |
| --- | --- | --- |
| Unit tests | `#[cfg(test)]` in the module | every public function with logic |
| Property tests (`proptest`) | crate `tests/` | coordinates, palette, lighting, masks, region merging, serialization |
| Reference comparisons | `tests/` | optimized implementation against a naive one (mesher, lighting, gather) |
| Golden files | `tests/golden/` | disk and network formats, asset conversion |
| Golden images | `crates/render/tests/golden/` | anything that changes what the terrain looks like (T1-21, below) |
| Integration | `crates/<bin>/tests/` | server + bots, handshake, mods |
| Mod scenario tests | `engine-cli test` | every mod in `mods/` |
| Fuzzing | `fuzz/` (cargo-fuzz), later | network and disk decoders |

## Benchmarks

- `criterion` in `crates/<crate>/benches/`. CI checks that they compile (`cargo bench --no-run`).
- Each benchmark corresponds to a row in [`performance-budgets.md`](performance-budgets.md); the name or a comment contains a reference to that row.
- A fixed data corpus: `worldgen::test_corpus` with a fixed seed (T0-12; `research/bench` was the starting point), later — an export of a real world.
- Stage 0 results and the test machine are recorded in `docs/research/`. A 2 vCPU VM gives ±30% noise — final numbers are taken on bare metal with a fixed frequency (spread ≤ 5%).
- A regression > 10% on a hot benchmark is a PR blocker (automatically, once a stable runner exists).
- CI cost: `ci.yml` skips documentation-only changes (`docs/`, `tasks/`, `task-reports/`, `*.md`), cancels a run when a newer commit arrives on the same PR, and restores the Rust build cache that only `main` writes.
- CI (T1-06): `.github/workflows/bench.yml` runs a fast subset (`world` section + map, `mesh`, `light`, `assets`; 20 samples, 2 s) after every merge to `main` and on demand for a branch (Actions → Benchmarks → Run workflow; not on every PR, to save Actions minutes of the private repository), uploads `bench-results.json` (`scripts/bench_collect.py`: mean ns per benchmark) and on a branch writes a comparison with the latest successful `main` run to the job summary (`scripts/bench_compare.py`). It is **advisory** on shared runners; `bench_compare.py --strict` fails on regressions once a stable runner exists.

## Load runs with bots (T2-05)

The `bots` binary (`crates/client/src/bin/bots.rs`, library `client::bots`) runs N headless game clients: real `Session`s in headless mode (prediction, reconciliation, edits; no light, meshes or rendering; columns around the bot decoded, the rest kept compressed). All bots share one network runtime and one loop. Behaviors: **walk** (turns now and then, jumps, steps around walls), **fly** (circles at a varying height, sprinting — this streams columns constantly, like a real fast flyer), **build** (places and digs a stone next to itself every half second). With `--groups G` the bots first fly to G homes on a square grid `--group-spacing` blocks apart (default 400: farther than the view and simulation radii, so groups are separate regions and do not see each other), then land and start their behavior.

```text
# On the server machine (the stage 2 exit run, T2-09):
server --offline --bind <LAN address> --stats-out server-stats.txt     # Enter stops it and writes the file
# On the bot machine (or the same one):
bots --connect <address>:47777 --count 200 --groups 10 --mix 8,0,2 --warmup 200 --seconds 120 --out results/
# Everything in one process (smoke runs; bots and server share the CPU):
bots --embedded /tmp/bot-world --count 50
```

Outputs: `bots-summary-<time>.txt` (bots joined/failed/stopped, mix, received and sent kbit/s per bot p50/p95/max, others in view, distance from home, corrections, edits sent/refused, bot process memory; with `--embedded` also the server tick percentiles while measuring) and `bots-<time>.csv` (one line per bot, including the columns it received while measuring — non-zero means it was still streaming). The server's `--stats-out` file has the tick percentiles after the warm-up, players, regions, memory and CPU time; it also prints a line every 10 s.

For the steady-state traffic budget ("no chunk streaming, ≤ 20 moving players nearby") use walkers and builders only (`--mix 8,0,2`) in groups of ≤ 20, and a warm-up long enough for every group to arrive (the CSV's `from_home` and `columns` show it).

CI smoke test: `crates/client/tests/bots.rs` — 50 bots against an in-process server, no errors.

## What every PR needs

1. `cargo fmt --all --check`, `cargo clippy --workspace --all-targets`, `cargo test --workspace` — green.
2. New logic — with tests. An optimization — with a before/after benchmark.
3. If a disk/network format changed — updated golden files and the format version. If the picture changed on purpose — refreshed golden images, looked at before committing.
4. If a specification changed — the updated document in `docs/`.

## Without a GPU

CI has no GPU. Render code is split into pure logic (culling, packing, path selection by capabilities) with unit tests and a thin wgpu layer. GPU tests run on **software Vulkan** (Mesa lavapipe, `mesa-vulkan-drivers`): CI installs it and sets `REQUIRE_GPU=1`, so a GPU test that finds no adapter fails there instead of skipping. Locally, without an adapter, they skip.

## Render reference scene (T1-21)

`crates/render/tests/reference_scene.rs` builds a fixed scene in code (no world file, no Mojang assets, fallback textures only), lights and meshes it with the real `light` and `mesh` code, and renders it offscreen at 320×180 from 7 fixed cameras at noon:

| Camera | What a regression looks like there |
| --- | --- |
| `overview` | the whole scene from above: missing chunks, wrong batch placement |
| `material_stand` | one block of every kind (all fallback textures, cutout glass and leaves, water): wrong texture layers, alpha |
| `white_furnace` | a closed snow room lit by a glowstone grid: any seam, tint or unexpected step is a bug |
| `light_ramp` | a corridor with glowstone at one end: block light 15 → 0 on the floor |
| `sky_shaft` | a closed box with a 1×1 roof hole: sky light spreading from the spot |
| `chunk_border` | a checker across x = 0 / z = 0, the mesh chunk and render batch borders: seams, gaps, flipped faces |
| `underwater` | the camera inside a water pool |

- **Golden images:** `crates/render/tests/golden/<camera>.png` (and `warm-glow-<camera>.png` for the example shader pack, `docs/examples/shaderpacks/warm-glow`, with its option defaults and its post pass). Paths B, Bi and C are all compared with the same images (written from Bi). On lavapipe B and Bi are bit-identical to them and C differs by at most a few levels on a handful of pixels.
- **Comparison:** a pixel differs when a channel differs by more than 8; at most 0.5% of pixels may differ. On failure the test writes `target/render-diffs/<camera>-<path>-actual.png` and `-diff.png` (magenta = different, over the dimmed golden image); CI uploads them as the `render-diffs` artifact.
- **Refreshing on purpose:** `UPDATE_GOLDEN=1 cargo test -p render --test reference_scene`, then **look** at the new images (e.g. side by side with the old ones in a diff viewer) before committing them. Never refresh to make a red test green without knowing why the picture changed.
- **Self-test:** `a_one_line_shading_change_fails_the_comparison` renders the white room with a 30% darker surface shading and checks that the comparison fails with a readable message.
- **New cameras** come with new features (water surface, shadows, PBR, model blocks; T1-22, T1-23): add a camera, generate its image, review it.

### On the laptop: comparing GPUs and RenderDoc

The golden images come from software Vulkan (the CI authority). On a hardware GPU the test reports mismatches as warnings and writes their images, but fails only with `REFERENCE_STRICT=1`, because rasterization rules allow tiny differences between GPUs.

- **See the scene from a real GPU:** `REFERENCE_OUT=ref-vega ADAPTER_NAME=vega cargo test -p render --test reference_scene -- --nocapture` (PowerShell: `$env:REFERENCE_OUT="ref-vega"; $env:ADAPTER_NAME="vega"; cargo test ...`), the same with `1660`, then compare the folders with `crates/render/tests/golden/` side by side. `WGPU_BACKEND=vulkan|dx12` picks the backend. Every image is written, named `<camera>-<path>.png`.
- **RenderDoc:** it captures frames at presentation, and the reference test has no window. To inspect a frame, capture the game client instead (`client --offline-name Dev`, RenderDoc → Launch Application with the client executable, F12 in game): the terrain pass, the `post` pass of a shader pack, the Texture Viewer for color and depth, the Mesh Viewer for a batch's quads and Pixel History on a wrong pixel. If a frame debug of the exact reference scene is needed, the scene can be shown in the client as a debug world (a follow-up, not done).