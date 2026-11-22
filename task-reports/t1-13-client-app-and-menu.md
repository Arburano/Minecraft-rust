# T1-13 — Client application, main menu and single-player

Status: the cloud part is done. Still pending: the owner's cold-start run on the laptop, and the playtest (movement feel, FPS).

## What was done

**The launcher question:** the owner asked whether to build the launcher on Tauri. ADR-0010 (accepted 2026-11-15) already decided against a separate launcher: the game's own menu (egui on wgpu) is the launcher, and only a tiny updater is separate (stage 3). This task follows ADR-0010.

**`client` library (testable without a window):**

- `settings.rs` — `Settings`: offline name, quality profile, `GpuPreference`, mouse sensitivity, FOV, autosave interval, worker threads. Stored in `<data>/settings.toml`, written atomically; a damaged file falls back to the defaults with a message. The per-user data folder is found per OS.
- `worlds.rs` — world folders with `world.toml` (name, seed, generator, timestamps, player state): create (unique folder names), list (most recent first), delete, seed from text.
- `session.rs` — `Session`, a single-player world without a window:
  - streaming through the loader, with saved columns read from the T1-02 store instead of generated;
  - spawning on the ground, and a 20 TPS physics tick with interpolation;
  - look, target, dig, place;
  - deferred relight, 2 sections per frame;
  - render updates (batches and light);
  - saving: edited columns when unloaded, on autosave and on quit, plus `world.toml`.
- `loading.rs`: `tick_with_source` (saved columns instead of jobs), `track_unloads` and `take_unloaded`.

**`render`:** `selection::select_for_surface`. It applies the T1-05 attempt order to one instance with every backend, filtered by what can present to the window.

**`client` binary:**

- `app.rs` — the winit 0.30 application:
  - wgpu surface (sRGB, vsync), the egui-wgpu overlay, a depth buffer;
  - a fixed 20 TPS step with interpolation;
  - input: WASD, Space, Ctrl or Shift, mouse look with the grabbed cursor, left/right click, 1–9, wheel, F3, Esc;
  - pause on Esc and on focus loss; save on quit and on window close;
  - a debug overlay, and BMP screenshots.
- `menu.rs`:
  - title (offline name field, single-player, multiplayer disabled until stage 2, settings, about, quit);
  - world list (play, delete with confirmation, create with name and seed);
  - settings and about (legal notice);
  - pause screen; HUD with crosshair, hotbar and debug text.
- `view.rs` — the T1-11 terrain renderer fed from the session, with fallback textures, cave culling and the camera from the profile's FOV, distance and fog.
- `main.rs` — options `--data-dir`, `--mods-dir`, `--measure-startup`, `--autoplay`, `--screenshot file:N`.

**Scripts:** `scripts/cold_start.sh` and `scripts/cold_start.ps1` (3 runs of `--measure-startup`).

**Tests:**

- `tests/session.rs`:
  - create a world, wait until playable, dig two blocks (falling into the holes), walk and jump, place two blocks, quit;
  - reopen, then check that the player position, every edited block and every section of the edited columns are equal.
  
  The test fails if saving is disabled (checked).
- Unit tests for settings and worlds.

**Docs:** `client-menu-and-updater.md` (implementation section), `performance-budgets.md` (cold start, VM), `open-questions.md` No. 15 (the game's name), `THIRD_PARTY.md` (egui's bundled fonts), the T1-13 task file, `tasks/README.md`.

**Also in this PR** (owner decisions from the same message):

- ADR-0013 accepted, with the new task T1-18 for its data side;
- the T1-16 follow-ups recorded as accepted.

### Numbers

Cloud VM (4 vCPU Intel Xeon 2.1 GHz), Xvfb with software Vulkan (llvmpipe):

| What | Result |
| --- | --- |
| Process start → first menu frame, first run | 2,347 ms (cold shader and driver caches) |
| Process start → first menu frame, later runs | 103–122 ms |
| Screenshots | title screen; in game, world radius 12, frame 120 after the world became playable |

FPS on llvmpipe is not representative; the laptop run will give real numbers.

## What went wrong

- The first menu showed boxes instead of letters: egui 0.37 has no built-in fonts unless the `default_fonts` feature is on. Enabled it and credited the fonts.
- The first window under Xvfb failed: the container lacked `libxkbcommon-x11`. It was installed in the session only; players' systems have it.
- Smaller API drift with egui 0.37 and wgpu 30: `run_ui`, texture deltas as lists, `Queue::present`, `CurrentSurfaceTexture`. Fixed while building.
- Clippy: `field_reassign_with_default` in a test.

## Issues encountered

- **Name (open question 15):** the game has no name. The window shows "Voxel Engine (working title)" and the data folder is `voxel-engine`.
- **Not done here:**
  - the visual block highlight under the crosshair;
  - held-button repeat for digging;
  - per-face textures and a grass top (T1-14 brings real textures from the player's own assets);
  - water as a separate translucent pass;
  - sounds;
  - a pipeline cache on disk (wgpu's driver cache makes later starts fast already);
  - the Microsoft login screen (waits for T0-10).
- **Saving:** done on the main thread (the spec allows a background writer later). Only edited columns are saved; light is recomputed on load, which is exact and costs the same as for generated columns.
- **Render distance:** changing the quality profile applies to the radius of the next world opened (the loader is created with it) and to fog and culling at once.
- The rayon pool (light, deep sky changes) and the engine job pool run side by side (noted in T1-16).
- All numbers are from the VM (rule 7). The laptop run is pending: cold start ×3, FPS with F3, and movement feel.
