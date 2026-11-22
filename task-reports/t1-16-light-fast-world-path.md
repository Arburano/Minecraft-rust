# T1-16 — Faster world-path lighting (same look)

Status: done (bare-metal laptop verification completed on ASUS TUF FX505DU, 2026-11-22).

## What was done

All in `light` (plus a `Send + Sync` bound in `client::edit`). The light stays bit-for-bit equal to a fresh full lighting: every T1-09 test is unchanged and green.

1. **The box.** A pass loads, imports and freezes only a box of ±15 cells around its section's edits, not the whole 48³ snapshot. Light changes reach at most 14 cells.
   - `LightNeighborhood` gained a loaded region; loaders, imports, sky runs and `set_props_raw` are clipped to it.
   - Sections the box covers only partly are read cell by cell (`Section::get`); mostly covered ones are unpacked whole.
   - The frozen layer is the six faces of the box, decoded once per section (`Nibbles::unpack_into`).
   - `begin_load` clears only the box written before.
   - Exports merge the box into the stored light outside it.
2. **Sky gating.** An edit whose opacity flips at or above its column's sky height − 1 extends the box down to the snapshot's bottom. The heightmap counts exactly the opacity > 0 blocks, so no other edit can move a run. If a run moves anyway while the box does not reach the bottom, the pass is redone with the whole snapshot.
3. **`sky_low`.** The pass records the lowest row whose direct-sky source changed, which decides what happens below:
   - ≥ 15: nothing below changes.
   - 1..14: a slab ±15 rows around the snapshot's bottom is recomputed, sky only (block light is unchanged below the pass's section). Its top face is frozen at the now-exact light from `B + 15`; its bottom and side faces are frozen at the unchanged world light.
   - 0: the old level-by-level recompute, now sky only and with the 9 sections of each level in parallel (rayon, a reusable snapshot per thread).
4. **Faster full relights:** only sources that can raise a neighbor are queued.
5. **Bug fix:** BFS from a frozen cell in the bottom or top border row stepped outside the grid, an index-out-of-bounds panic. This was latent since T1-09 and reachable when a decrease hit the bottom border; the new tests found it. Neighbors outside the grid are now skipped.

**Tests (new):**

- `sky_edits_at_section_borders_equal_fresh_lighting`: 5 seeds × 12 shafts of random depth with rooms and descending tunnels, roofs on section border rows, random batch sizes.
- `roof_over_a_shallow_run_darkens_below_the_snapshot`: runs ending 1, 5 and 12 rows above the snapshot's bottom, roof placed and removed.

**Sensitivity checked:** the tests fail if the `sky_low` threshold is 12 (13 is the exact bound), if the box margin is 12, if the slab bottom is at +6, or if its top is at 21. The bench `world_light` gained a "roof over a 60-block shaft" case.

**Docs:** `lighting.md` (new section), `performance-budgets.md` (two rows), the task file (status, criteria, laptop commands), `tasks/README.md`.

### Numbers

Cloud VM (4 vCPU Intel Xeon 2.1 GHz). `cargo bench -p light --bench world_light`, 192 edits per kind (48 for the shaft), several runs each; ranges over runs.

| Edit | Before (T1-09/T1-12 code, same bench) | After |
| --- | --- | --- |
| Torch place / remove | p50 0.67–0.69 ms, p95 0.95–1.03 ms | p50 0.27–0.28 ms, p95 0.32–0.35 ms |
| Stone placed on the surface (moves direct sky one cell) | p50 10.3–10.5 ms, p95 17.3–17.4 ms | p50 0.28–0.29 ms, p95 0.33–0.41 ms |
| Roof over a 60-block shaft (direct sky to any depth) | p50 25.6 ms, p95 36.9–37.9 ms | p50 10.2–11.6 ms, p95 11.9–14.7 ms |

How the steps added up (p50 on the same VM):

| Step | Torch | Stone on surface | Roof over shaft |
| --- | --- | --- | --- |
| Before | 0.69 ms | 10.5 ms | 25.6 ms |
| `sky_low` gating | — | 0.58 ms | — |
| Sky-only relight with source pruning | — | — | 19.5 ms |
| 9 sections in parallel | — | — | 9–11 ms |
| Box | 0.53 ms | — | — |
| Partial clear and row-copy export | 0.45 ms | — | — |
| Single sky-run rebuild, per-section freeze, partial reads | 0.27 ms | 0.28 ms | — |

End to end (`client` example `edit_latency`, 100 edits per kind):

| Edit | Total | Relight alone |
| --- | --- | --- |
| Break a surface block | p50 0.81–0.87 ms (was 9.8–16.4 ms) | p50 0.33–0.36 ms |
| Place a block in the air | p50 0.81–0.86 ms, p95 ≤ 1.34 ms | p95 6–7 ms before the slab pass |
| Underground torch | p50 0.73–0.76 ms | p50 0.25–0.27 ms |

Profile of a torch pass before the last step (μs per pass): reload 320 (blocks 131, frozen layer 119, import 50, sky runs 45, clear 16), revert 80, BFS 19, export 32. After it, the remaining ≈ 0.27 ms is the snapshot copy itself.

### Bare-Metal Laptop Numbers (ASUS TUF FX505DU, Ryzen 7 3750H, 2026-11-22)

**End-to-end edit latency** (`cargo run --release -p client --example edit_latency -- 100`, 3 runs, raw logs in `docs/research/bare-metal-2026-11-22-wave3/`):

| Edit | Total latency (p50 / p95) | Relight alone (p50 / p95) | Speedup vs T1-12 on laptop |
| --- | --- | --- | --- |
| **Break surface block** | **0.92–1.02 ms** / ≤ 1.71 ms | **0.39–0.45 ms** / ≤ 0.65 ms | **~28× faster relight** (was 11.8–13.3 ms) |
| **Place block in air** | **0.84–1.01 ms** / ≤ 2.08 ms | **0.36–0.43 ms** / ≤ 0.91 ms | **~30× faster relight** (was 11.8–12.8 ms) |
| **Torch underground** | **0.75–1.06 ms** / ≤ 1.63 ms | **0.29–0.42 ms** / ≤ 0.53 ms | **~2.5× faster relight** (was 0.78–0.95 ms) |

**World-path Criterion bench** (`cargo bench -p light --bench world_light`):
- `torch place / restore`: p50 **332–337 µs**, p95 **515–524 µs** (was ~700–900 µs on VM).
- `stone_on_surface place / restore`: p50 **317–319 µs**, p95 **470–488 µs** (was 10,000–13,000 µs).
- `roof_over_shaft` (60-block shaft): p50 **10.87 ms**, p95 **13.48–13.72 ms** (down from 25–36 ms).
- `initial_light_one_column`: p50 **7.9 ms** per column (24 sections).

## What went wrong

- The first version of the new sky test passed even with a deliberately broken threshold. Shafts in solid rock give light no path below the snapshot. Adding rooms and descending tunnels made it sensitive. A deterministic test now covers the exact boundary.
- That stronger test crashed with an index out of bounds in `propagate_increase`. This is a latent T1-09 bug (a frozen border cell's neighbors outside the grid), not a result of this task, and it is fixed.
- The first box version gained only 20% for a torch. Profiling showed that loading still unpacked whole sections and that the frozen layer was read cell by cell through the hash map; both were fixed.
- `relight_below` used to recompute at least two full levels (18 snapshots) for any sky change, even a one-cell one. That was the main cause of the 10–17 ms surface spikes.
- Clippy: an unused `mut` in the test.

## Issues encountered

- **≤ 100 µs not reached for non-sky edits (0.27–0.28 ms p50 on the VM).** What is left is copying a 31³ box from the world and back. Only persistent per-region light and opacity grids, with the BFS working in place, remove it. That is an architecture change (storage and ownership of light), so per the task constraints it needs an ADR with status "Proposed" first. Not started; owner decision.
- **Deep sky changes still cost ≈ 10 ms on the VM:** a roof over, or an opening into, a shaft or cave whose direct-sky run reaches below the snapshot. These are rare. Options for the owner:
  - (a) chain slabs downwards instead of 9 snapshots per level (exact; an estimated ≈ 1–2 ms);
  - (b) spread the downward work over frames (light below appears a few frames later);
  - (c) leave it until persistent grids exist.
- **Light delay:** unchanged from T1-09. Same pass, same cap per call. The slab pass runs inside the same call.
- **Threads:** the deep-case pass uses rayon's global pool next to the engine's job pool, so on a fully loaded machine they compete. This is acceptable for a rare path; persistent grids would remove it.
- All numbers are from the VM (rule 7). The laptop run and the visual check are pending (commands in the task file).
