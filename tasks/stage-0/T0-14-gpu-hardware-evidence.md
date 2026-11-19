# T0-14. GPU hardware evidence for T0-06 (Vega 10 + GTX 1660 Ti)

- **Status:** in-progress
- **Depends on:** T0-06 implementation (merged in PR #8)
- **Owner:** agent (parts A and C); part B — an agent running **on the laptop itself** (Claude Code / desktop app locally, or a cloud session attached to the laptop via Remote Control), with the human only for the steps marked [human]
- **Closes:** T0-06 (its second acceptance criterion)

## Goal

Produce the hardware evidence T0-06 is waiting for, on the hardware the owner actually has, and update the budgets from it.

## Owner decision (2026-11-19, [open question No. 12](../../docs/open-questions.md))

- The **iGPU** measurement is the AMD Radeon RX Vega 10 of the ASUS TUF FX505DU laptop (Ryzen 7 3750H).
- There is **no GTX 1050**. The discrete measurement is the **GTX 1660 Ti** of the same laptop.
- Consequences (do not hide them):
  - The "≤ 8 ms GPU geometry on iGPU" target is checked on the Vega 10 — a real iGPU, so this target is genuinely verified.
  - The "≤ 1 ms CPU command recording" target is checked on the Ryzen 7 3750H with both GPUs.
  - The GTX 1660 Ti is considerably faster than a GTX 1050. Its numbers do **not** verify GTX 1050 targets (PRD "60 FPS on iGPU / GTX 1050", stage 5 "64 chunks on GTX 1050"). Those targets stay unchanged and stay "not verified on GTX 1050".
  - Both adapters are in one laptop: shared CPU, RAM, OS and power. Report them as two adapters of one machine, not as two machines.

## Known facts from the 2026-11-18 probe ([local-probe-2026-11-18.json](../../docs/research/gpu/local-probe-2026-11-18.json))

| Backend | Index | Adapter | TIMESTAMP_QUERY | Path |
| --- | --- | --- | --- | --- |
| DX12 | 0 | GTX 1660 Ti (driver 32.0.15.5597) | yes | B |
| DX12 | 1 | Radeon RX Vega 10 (driver 27.20.1032.2) | yes | B |
| DX12 | 2 | Microsoft Basic Render Driver (software) — **do not measure** | yes | B |
| GL | 0 | GTX 1660 Ti (NVIDIA 555.97) — GL does not expose the Vega 10 | yes | B |
| Vulkan | — | **no adapters at all** | — | — |

Indices are per backend and can change after a driver update: always re-check them in the new probe. Every adapter here supports `TIMESTAMP_QUERY`, so the "no timestamps" path cannot be exercised on real hardware without the flag from part A.

## Part A — agent, before the human run

1. [x] Add a `--no-timestamps` flag to `quad-bench` that does not request `TIMESTAMP_QUERY` even if the adapter supports it, so the `gpu_geometry_ms: null` path runs end to end. The report must say timestamps were disabled by the flag (e.g. `"timestamps": "disabled-by-flag"` vs `"unsupported"` vs `"enabled"`). Unit test for the CLI parsing; no change to the measured code otherwise.
2. [x] Let the ignored image-comparison test choose the adapter: an env var (e.g. `QUAD_BENCH_ADAPTER_NAME`, substring match on the adapter name) in addition to `WGPU_BACKEND`. Today it takes the first adapter with vertex storage, which on DX12 is always the GTX 1660 Ti, so the Vega 10 is never compared.
3. [ ] Optional: a `research/gpu/run-matrix.cmd` (cmd.exe, so output stays UTF-8) that runs the probe and the whole matrix from part B with the correct file names. Test what you can without a GPU (`cargo test -p render`, CI green).
4. [x] Update [docs/research/gpu/README.md](../../docs/research/gpu/README.md) with the new flag/env var. Merge part A before the human run.

## Part B — on the laptop (agent running locally, human only where marked)

A cloud container has no GPU, so part B must run on the laptop. An agent running there can execute every command below. The human is needed only for:

- [human] plug in AC power, set Windows power mode "Best performance", close games/browsers/video before the matrix starts, and do not use the laptop while it runs;
- [human] approve driver installation (administrator rights) and reboots in step 2, if a driver update is needed;
- [human] the optional NVIDIA Control Panel setting for Vega 10 on GL (GUI);
- [human] approve the agent's commands and the push.

The agent must ask the human for these before starting, and record in the summary that power/background conditions were confirmed by the human.


Use **Command Prompt (cmd.exe)**, not PowerShell (PowerShell 5.1 writes UTF-16 and breaks the JSON). AC power, Windows power mode "Best performance", close games/browsers/video, no other GPU work. Replace `DATE` with the run date, e.g. `2026-11-20`.

1. [ ] **Build:** `git pull`, then `cargo build --release -p render --bins`.
2. [ ] **Vulkan.** wgpu found no Vulkan adapters, although both NVIDIA and AMD drivers normally ship Vulkan. Check in this order and write down what you found:
   - `C:\Windows\System32\vulkan-1.dll` exists?
   - Install the Vulkan runtime/SDK from LunarG (vulkan.lunarg.com) and run `vulkaninfo --summary`. Does it list the GTX 1660 Ti and the Vega 10?
   - If not: update/clean-install the NVIDIA driver (GeForce/Studio driver from nvidia.com) and the AMD driver (AMD Adrenalin for Vega 10 / Ryzen 3750H from amd.com; the laptop's OEM driver 27.20.1032.2 is old). Reboot, run `vulkaninfo --summary` again.
   - If Vulkan still does not work: it is an "unsupported combination". Save the `vulkaninfo` output (or the error text) to `docs/research/gpu/vulkan-unavailable-DATE.txt` with the driver versions. Do not skip it silently.
3. [ ] **Probe:** `target\release\gpu-probe.exe > docs\research\gpu\laptop-probe-DATE.json`. Check the adapter indices for each backend against the table above.
4. [ ] **Benchmark matrix — three runs each.** Set the machine string once (fill in current driver versions):

   ```bat
   set M=ASUS TUF FX505DU; Ryzen 7 3750H; about 14 GiB usable RAM; Windows 11 build 28000; NVIDIA <driver>; AMD <driver>
   ```

   Then for every row run (in an interactive cmd window, `%r` with one `%`):

   ```bat
   for %r in (1 2 3) do target\release\quad-bench.exe --backend BACKEND --adapter INDEX --path PATH --machine "%M%" --environment bare-metal > docs\research\gpu\NAME-BACKEND-PATH-DATE-run%r.json
   ```

   | NAME | BACKEND | INDEX (re-check) | PATH |
   | --- | --- | --- | --- |
   | vega10 | dx12 | 1 | b |
   | vega10 | dx12 | 1 | c |
   | gtx1660ti | dx12 | 0 | b |
   | gtx1660ti | dx12 | 0 | c |
   | gtx1660ti | gl | 0 | b |
   | gtx1660ti | gl | 0 | c |
   | vega10 | vulkan | from probe | b, c — only if step 2 fixed Vulkan |
   | gtx1660ti | vulkan | from probe | b, c — only if step 2 fixed Vulkan |

   That is 18 files without Vulkan, 30 with it. Vega 10 on GL: Windows GL uses the GPU chosen by the driver; optionally set "Integrated graphics" for `quad-bench.exe` in NVIDIA Control Panel → Manage 3D settings → Program settings, re-run the probe and, if GL now shows the Vega 10, add `vega10 gl b/c`. If not possible, note it as unsupported.
5. [ ] **No timestamps** (needs part A): one run per adapter, e.g.
   `target\release\quad-bench.exe --backend dx12 --adapter 1 --path b --no-timestamps --machine "%M%" --environment bare-metal > docs\research\gpu\vega10-dx12-b-notimestamps-DATE.json`
   and the same with `--adapter 0` as `gtx1660ti-...`. Expected: the run succeeds, `gpu_geometry_ms` is `null`, CPU recording is present, `rendered_pixels` > 0.
6. [ ] **Image comparison test (ignored by default)** — B and C must draw identical images. Run once per adapter/backend and save the console output:

   ```bat
   set WGPU_BACKEND=dx12
   set QUAD_BENCH_ADAPTER_NAME=Vega
   cargo test -p render --bin quad-bench gpu_paths_render_identical_nonblank_images -- --ignored > docs\research\gpu\imagetest-vega10-dx12-DATE.txt 2>&1
   ```

   Repeat with `QUAD_BENCH_ADAPTER_NAME=1660` for `dx12` and `gl` (and `vulkan` if it works). Each file must end with `test result: ok. 1 passed`.
7. [ ] **Sanity check before commit:** no file is empty; every `quad-bench` JSON contains `"build_profile": "release"` and `"environment": "bare-metal"`. If something failed, keep the error text in a `.txt` next to it instead of an empty JSON.
8. [ ] **Commit and push:**

   ```bat
   git checkout -b t0-14-gpu-hardware-evidence
   git add docs/research/gpu
   git commit -m "T0-14: add laptop GPU probe, benchmark and image-test evidence"
   git push -u origin t0-14-gpu-hardware-evidence
   ```

   Then start an agent session with: "Do part C of tasks/stage-0/T0-14-gpu-hardware-evidence.md on branch t0-14-gpu-hardware-evidence".

## Part C — agent, after the files are pushed

1. [ ] Validate every file: release profile, `bare-metal`, default quads/draws/frames (500,000 / 600 / 120 measured / 30 warmup), `rendered_pixels` > 0, adapter name matches the file name, three runs per combination, image tests passed, no-timestamp runs have `null` GPU time. List anything missing; do not invent numbers.
2. [ ] Write `docs/research/gpu/summary-DATE.md`: hardware and driver versions; per adapter/backend/path p50 and p95 of `gpu_geometry_ms` and `cpu_command_recording_ms` (median of the three runs) and run-to-run spread; B vs C on the same adapter/backend; unsupported combinations (Vulkan, Vega on GL) with reasons; the owner decision above and its consequences.
3. [ ] Update `docs/process/performance-budgets.md`:
   - "GPU geometry time ≤ 8 ms per frame on iGPU" → Measured, from the **Vega 10** numbers (path the engine would select, B; also give C).
   - "CPU command recording ≤ 1 ms per frame" → Measured, Ryzen 7 3750H, worst adapter/backend.
   - Add a note that GTX 1050 targets remain unverified (GTX 1660 Ti used, owner decision No. 12).
   - If a target is **not** met: stop and ask the owner (AGENTS.md); do not change the target or the architecture.
4. [ ] Update `docs/architecture/meshing-and-rendering.md` (capability dump line) and, only if the owner changes a target, the PRD identically.
5. [ ] Close: tick T0-06's second acceptance criterion (reworded per decision No. 12), set T0-06 and T0-14 to `done` in their files and in `tasks/README.md`, update `task-reports/t0-06-render-capability-probe.md`, write `task-reports/t0-14-gpu-hardware-evidence.md`, open the PR, CI green.

## Acceptance criteria

- [ ] `--no-timestamps` and adapter selection for the image test exist, are tested and documented (part A).
- [ ] Probe, 3 runs per supported adapter/backend/path, no-timestamp runs and image tests for both adapters are committed; unsupported combinations are documented with reasons (part B).
- [ ] Summary written; the two budget rows are Measured with links; GTX 1050 targets explicitly marked as not verified (part C).

## Out of scope

Path A / compute culling, GTX 1050 measurements, a full textured renderer, frame pipelining.
