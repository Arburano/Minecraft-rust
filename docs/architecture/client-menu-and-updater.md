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
