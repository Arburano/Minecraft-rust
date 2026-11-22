# In-game launcher menu and updater

Crates: `client` (the in-game menu), `updater` (separate binary). ADR: 0010.

Decision: there is no separate launcher (Tauri etc.). The game's main menu acts as the launcher, and a tiny updater updates files and starts the game. Target: from click to menu ≤ 5 s.

## Launch process

```text
shortcut ─► updater
          ├─ check the version manifest (signed with ed25519)
          ├─ update available? download binary deltas (zstd --patch-from), verify SHA-256
          ├─ atomically replace the version directory; on error, roll back to the previous one
          └─ start client with the --launched-by-updater flag
client
          ├─ the window opens immediately (before assets load), menu on egui over wgpu
          ├─ in the background: login check (refresh token), assets, shader cache
          └─ "Play" → pick server / world → mod sync → login
```

Why the updater is separate: a running binary cannot reliably replace itself (especially on Windows), and a crash in the middle of an update must not leave the game broken.

## Updater

- Minimal size: no GUI framework (console window or the OS's native progress bar), no async runtime if possible.
- On-disk layout:

```text
<install>/updater(.exe)
<install>/versions/<version>/client(.exe), ...
<install>/current -> versions/<version>      (or a current.txt file on Windows)
<data>/cache/assets/   <data>/cache/mods/   <data>/cache/shaders/   <data>/cache/lod/
```

- Channels: stable / beta. Rollback: the previous version directory is kept until the next successful launch.
- The updater also updates itself: the new updater version is placed alongside and swapped in on the next launch.

## Menu (in the client)

Screens: login (Microsoft), server list, single-player, mod profiles, settings (quality profile, render distance ≥ 8), mod catalog, about.

Menu responsibilities (formerly the launcher's responsibilities):

1. Login and session refresh — [auth-and-assets.md](auth-and-assets.md).
2. Mojang assets and conversion — same place.
3. Mod sync for the server: the server sends a manifest (mods, versions, SHA-256, signatures); the menu downloads what is missing into a content-addressed cache and verifies signatures.
4. Crash reports — only with the player's consent.

## Fast start (≤ 5 s to menu)

- The window and menu are shown before the world loads; heavy assets load lazily (PRD §14, No. 3, No. 9).
- Compiled shader/pipeline cache of wgpu on disk.
- Assets are converted once after download, then read via mmap (`rkyv`).
- Registries are loaded fully (kilobytes); models/sounds on demand with a placeholder so there is no freeze.

## Tests

- Network drop in the middle of an update → the game starts on the old version.
- Forged manifest (wrong signature) → the update is rejected.
- Cold start to menu on the reference laptop ≤ 5 s.

## Implementation (T1-13)

- **`client` binary:**
  - winit 0.30 window;
  - wgpu surface; the adapter comes from `render::selection::select_for_surface`, which uses the same attempt order as T1-05 over one instance with every backend, so a Vulkan with no adapters falls back to DX12/GL;
  - egui 0.37 (with `default_fonts`, credited in `THIRD_PARTY.md`) through egui-wgpu.
  
  The menu appears before anything else loads; a world loads only after "Play".
- **Screens:**
  - title: the offline name field (required while Microsoft login is not available, T0-10), single-player, multiplayer (disabled until stage 2), settings, about, quit;
  - world list: play, delete with confirmation, create with a name and a seed (a number is used as is, text is hashed, empty is random);
  - settings: quality profile, FOV, mouse sensitivity, graphics API, GPU class, GPU name filter, autosave interval;
  - about: the `docs/legal.md` rule 4 notice, which is also on the title screen.
- **Settings** live in `<data>/settings.toml`. The data folder is per user: `%APPDATA%\voxel-engine`, `~/Library/Application Support/voxel-engine` or `$XDG_DATA_HOME/voxel-engine`, or `--data-dir`. A damaged file falls back to the defaults with a message.
- **Worlds** live in `<data>/worlds/<folder>/`. `world.toml` holds the name, seed, generator mode and player state; the T1-02 store (`world.redb` + `region/`) holds the edited columns.
- **`client::session::Session`** is the single-player world without a window:
  - streaming through the T1-10 loader, with saved columns read from the store instead of generated;
  - a 20 TPS player tick (T1-12 physics) with render interpolation;
  - dig and place;
  - deferred relight, capped at 2 sections per frame;
  - saving: edited columns when they are unloaded, every `autosave_seconds`, and on quit. Unedited columns are regenerated, which is deterministic. Saved columns are relit on load, because the light computed from blocks is exact.
- **In game:**
  - WASD, Space (jump / swim up), Ctrl or Shift (sprint), mouse look with the cursor grabbed;
  - left click digs, right click places the selected block;
  - 1–9 and the mouse wheel pick from the hotbar; F3 shows the debug overlay;
  - Esc pauses ("Back to game", "Save and quit to title"), and losing focus pauses too.
  
  Rendering is the T1-11 terrain renderer with cave culling and the profile's distance and fog.
- **Tools:**
  - `--measure-startup` prints the time from process start to the first menu frame and exits (`scripts/cold_start.sh` / `.ps1` run it 3 times);
  - `--autoplay <world>` opens a world directly;
  - `--screenshot file.bmp:N` saves frame N (in a world, once it is playable) and exits. The last two are used for checks in the cloud under Xvfb with software Vulkan.

