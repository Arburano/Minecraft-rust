# T1-12 — Player physics, digging and placing blocks

Status: the cloud part is done. Still pending: the owner's laptop run of `edit_latency`, and the movement-feel review, which needs the playable client (T1-13).

## What was done

**`sim`:**

- `physics.rs` — `PhysicsConfig` read from the new datapack file `mods/core/data/player/physics.toml`; unknown keys are rejected and the swim blocks are resolved by name. `PlayerBody` (feet position, velocity, on-ground, in-fluid) and `PlayerInput` (forward, strafe, jump, sprint, yaw).
- `step()` runs one 20 TPS tick:
  - acceleration along the yaw;
  - jump, sprint jump, swim-up;
  - an axis-wise AABB sweep (Y, X, Z) against full cells;
  - step-up ("up, across, down", kept if it goes farther);
  - gravity, vertical drag, ground/air/fluid friction.
- Unloaded cells collide. `interpolate()` gives the render position between ticks. `MapBlocks` adapts the `WorldMap`.
- `raycast.rs` — voxel DDA: hit cell, entered face, distance, `place_pos()`.
- Tests:
  - landing exactly on the floor;
  - jump height 1.2–1.3;
  - walk speed 4.0–4.6 blocks/s;
  - walls stop the player, and a 1-block ledge needs a jump;
  - step-up with a 1.0 step;
  - swimming sinks slowly and jump rises;
  - **determinism**: 600 scripted ticks with obstacles, bit-identical twice;
  - config parsing;
  - interpolation;
  - raycast against a naive march, 300 random rays.

**`client`:**

- `edit.rs`:
  - `Editor::set_block` → `WorldMap::set_block` → `Loader::block_changed` (cancels stale mesh and light jobs and drops their finished results) → re-gather and re-mesh of every already meshed affected mesh chunk on the main thread, with the visibility recomputed → `Loader::store_mesh` → relight queued.
  - `Editor::process_light` handles the deferred relight with a per-call section cap.
  - `pick`, `dig`, and `place` (refuses occupied cells and blocks overlapping the player).
  - `BatchSync` re-copies the render batches of changed mesh chunks.
- `loading.rs`: `track_mesh_changes`, `take_changed_meshes`, `store_mesh`, `block_changed`, `has_mesh`. Finished job results are now applied only if their job is still in flight, so cancelled stale results are dropped.
- `tests/edit.rs`:
  - after edits on mesh chunk borders and inside, meshes equal a fresh build and light equals a fresh `light_column`;
  - loader ticks after an edit do not bring stale meshes back;
  - `BatchSync` repacks the edited batch with the right quad count;
  - pick, dig and place from the player's view, with the overlap rule.
- Example `edit_latency`.

**`light`:**

- `WorldLight::track_changes` and `take_changed` (sections to upload to the GPU).
- `LightUpdates::process` runs the downward direct-sky recompute only for edits at or above the column's current sky height − 1. The heightmap counts exactly the blocks with opacity > 0, so any other edit cannot change direct sky. All existing light tests pass, including the 3 seeds × 60 random edits compared with fresh lighting, and so does the new client edit test.

**Docs:** `simulation-and-ai.md` (implementation), `lighting.md` (the sky-height condition), `performance-budgets.md` (edit row), the task file (status, criteria, laptop command), `tasks/README.md`.

**CI (owner request in the same session):**

- benchmarks run only after merges to `main` and on demand;
- CI skips documentation-only changes and cancels superseded runs;
- only `main` writes the Rust cache;
- documented in `testing-and-benchmarks.md`.

### Numbers

`cargo run --release -p client --example edit_latency -- 100`: cloud VM (4 vCPU Intel Xeon 2.1 GHz), world seed 7, radius 4, y −64..320, 100 edits per kind, 3 runs. Ranges are over the runs; relight is processed to completion.

| Edit kind | set_block + re-mesh | relight | batch re-copy | total |
| --- | --- | --- | --- | --- |
| Break a surface block (changes direct sky) | p50 0.46–0.75 ms, p95 ≤ 1.49 ms | p50 9.3–15.5 ms, p95 ≤ 19.0 ms | p50 14–19 µs | p50 9.8–16.4 ms |
| Place a block in the air (changes direct sky) | p50 0.50–0.77 ms, p95 ≤ 1.47 ms | p50 11.6–17.1 ms, p95 ≤ 21.0 ms | p50 13–16 µs | p50 12.2–18.1 ms |
| Torch underground | p50 0.46–0.70 ms, p95 ≤ 1.47 ms | p50 0.71–1.23 ms, p95 ≤ 1.72 ms | p50 9–15 µs | p50 1.2–2.0 ms |

Before the sky-height condition (same example, 3 runs), the underground torch relight was p50 12.1–16.7 ms.

## What went wrong

- Two physics tests were wrong, not the physics:
  - holding jump kept the player bouncing at the end of the check;
  - the "wall" was one block deep, so the player jumped onto it and dropped off the far side.
  
  The tests now check the landing on top.
- The first edit tests treated `WorldMap::height` as the top block. It is the first open cell above it, so the "digs" changed nothing; the helper now uses `height − 1`.
- The first latency run showed 12–17 ms relight even for an underground torch: every opaque ↔ non-opaque change ran the downward direct-sky recompute. Fixed with the sky-height condition (see above).
- Clippy: `bool_assert_comparison`, and `too_many_arguments` on `place` (now takes a `Placer` struct).

## Issues encountered

- **Frame spike from sky-changing edits (owner decision needed).** Breaking a surface block or building in the open costs 9–17 ms of relight on the VM: the T1-09 world path rebuilds a 48³ snapshot per section, downwards. The block itself appears in the same frame (geometry is ready in < 1 ms), and light is deferred, but whichever frame processes the section pays the whole cost. That is a visible stutter at 60 FPS (PRD §14: feel wins). Options:
  - (a) move T1-16 (a faster light path with the same look) before T1-13;
  - (b) a smaller stopgap: run the downward sky pass as a background job on a copy, or spread it over frames.
  
  The laptop run will show the real size of the spike.
- **Latency criterion:** "edit-to-visible ≤ 1 frame" holds for geometry and for underground light on the VM. Sky light is one or more frames late by design (deferred light, owner decision 2026-11-22).
- **Movement feel** cannot be reviewed without the window and input of T1-13. The numbers are in the datapack file and can be tuned without code changes.
- **Simplifications:**
  - re-meshing always re-gathers the 34³ neighborhood (≈ 80 µs per chunk) instead of using the incremental mask path of T0-04. A builder cache needs invalidation when loader jobs replace a mesh, and the gather fits the frame budget;
  - `Collision::Model` counts as a full cube;
  - no ladders, sneaking or fall damage;
  - water is only a swim volume (no currents);
  - the `Save` dirty marks are left for the storage writer.
- Not measured on the laptop yet (rule 7): all numbers above are from the VM.
