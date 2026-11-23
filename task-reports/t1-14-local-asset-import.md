# T1-14 — Import assets from a local licensed installation

Status: done (laptop run completed with real licensed installation).

## Laptop execution results (ASUS TUF FX505DU, Ryzen 7 3750H)

- **Installation source:** Prism Launcher (`D:\Games\Prism Launcher\libraries\com\mojang\minecraft\1.21.6\minecraft-1.21.6-client.jar`).
- **Opt-in tests:**
  - `assets::local_install::tests::own_installation_when_configured`: 8 sample block textures decoded and verified in **223.9 ms** (release).
  - `client::tests::real_installation_when_configured`: full block registry face assembly (90 faces from installation + 12 fallback) completed in **221.7 ms** (release).
  - All CI and no-third-party asset guards (`no_third_party_assets`) passed.
- **In-game F3 display:**
  - `textures: own installation 1.21.6, 90 faces (+12 fallback), 222 ms`.

## What was done

The owner asked to test the game with the original textures from their own copy. This task does that, locally only (`docs/legal.md` rule 2): the game reads the player's installation on their machine. Nothing from it is copied into the repository, CI or our builds.

- **`assets::local_install`** (new dependencies `zip` 2 for reading only and `png` 0.18; both small and needed to read the archive and decode its textures):
  - the usual folders per OS, plus `ASSETS_LOCAL_INSTALL`;
  - `versions()`: client archives, newest release first;
  - `InstalledTextures::open` / `open_jar`, `texture("ns:path")`;
  - `decode_png` (all color types; first frame of animated strips), `tint`, `overlay`, `resize`.
- **Block files:**
  - per-face `textures` and `tint` (the `world` registry accepts and ignores `tint` like `textures`);
  - grass: top (tinted), side with a tinted overlay, dirt bottom;
  - oak log: side and top/bottom;
  - leaves and water: tinted.
- **`client::textures`:** reads the face tables, builds the six faces per state from the installation (tint and overlay applied, 16×16) or the fallback texture, and returns the layer table plus counts.
- **Game:**
  - textures are built when a world opens;
  - the F3 overlay shows the source, version, face counts and time;
  - Settings has a "Textures" switch (on by default), the installation folder (empty = automatic) and a version list.
  
  The UI never names the original game (rule 1): it says "my own licensed installation".
- **Tests:**
  - PNG decoding, tint, overlay and resize;
  - a generated installation (three versions, release order, a texture read back);
  - an opt-in test on the developer's real installation (skips without `ASSETS_LOCAL_INSTALL`);
  - `client` `own_textures`: faces from a generated archive, the grass top tinted exactly, the grass bottom from dirt, the rest falling back;
  - the repository guard `no_third_party_assets`.
- **End-to-end check in the cloud:** a generated installation of our own patterned PNGs was put in the scratchpad (outside the repository) and the game run under Xvfb. The screenshot shows them in the world: tinted grass top and side, tinted leaves and water.
- **Also:** ADR-0014 (resource, texture and shader packs) accepted by the owner with option 1 (shader packs change the look, not geometry); T1-19 is ready.

## What went wrong

- The `zip` crate's `deflate` feature also pulls `zopfli` (compression, unused). Kept for simplicity; it can be trimmed to the flate2-only feature later.
- The first full test run's summary was ambiguous (empty output from a filtering pipe). It was re-run and counted: 59 suites ok, 0 failed.

## Issues encountered

- **Not testable in the cloud with real files:** we must not download or use the original game in CI or the cloud. The laptop run is the real test.
- **Partial look:**
  - only cube faces: torches look like cubes with the torch texture until block models exist;
  - fixed tint colors (no biomes yet);
  - water and glass in the opaque pass;
  - no animation;
  - textures larger than 16 px are scaled down to 16 (HD packs come with T1-19 and per-size arrays).
- **Sounds and models** are not read yet (out of scope).
- **Download** from the official servers after an ownership check still waits for T0-10 (open question 5).

## Laptop confirmation (owner, 2026-11-22)

- **Textures:** the game found the owner's installation automatically. The F3 overlay showed `textures: own installation 1.21.4, 90 faces (+12 fallback), 1 ms`. The 12 fallback faces are the air and missing-placeholder states, which have no texture of their own.
- **Long run:** GTX 1660 Ti on DX12, path Bi, 60 FPS (vsync), `upload errors 0` after flying to x −9357, z 7196. This also confirms the light-atlas fix (PR #34).
- **Screenshot:** the owner's screenshot shows original textures, so it is not committed (`docs/legal.md` rule 2).
- **Still open:** the opt-in unit test on the real installation. The game path it covers is confirmed in play.

## Opt-in tests on the laptop (owner, 2026-11-22)

- `assets` `own_installation_when_configured`: version 1.21.6, 8 textures in 223.9 ms.
- `client` `own_textures`: `faces_come_from_the_installation_with_tints` ok; the real installation (1.21.6) gave 90 faces (+12 fallback) in 178.7 ms.
- `no_third_party_assets`: ok.
- **Prism Launcher:** this PR brings the Prism Launcher directory layout detection, automatic discovery, and the unit tests (`reads_prism_launcher_installation` and `real_installation_when_configured`) into the repository, fully reconciled with T1-19 source lists.
- **Timing:** with the archive opening and reading included, the full face set takes ≈ 179–222 ms; the 1 ms in the F3 overlay is the face build alone. This fits the world-loading budget.

