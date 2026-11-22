# T1-13. Client application, main menu and single-player

- **Status:** done (cold start measured on laptop, in-game screenshot verified 2026-11-22)
- **Depends on:** T1-07, T1-08, T1-10, T1-11
- **Owner:** agent (cloud) + laptop confirmation

## Goal

The `client` binary becomes a game: window, main menu (the in-game launcher), create/load a single-player world (with T1-02 saving), settings (quality profile, backend override from T1-05, offline name), "About" screen with the legal notice. Cold start to menu ≤ 5 s with lazy asset loading.

## Specs

- [client-menu-and-updater.md](../../docs/architecture/client-menu-and-updater.md)
- [legal.md](../../docs/legal.md) — rule 4 (About text)
- [auth-and-assets.md](../../docs/architecture/auth-and-assets.md)

## Deliverables

- Window + render loop + menu screens; single-player world in-process (embedded world, no network).
- Autosave via T1-02; settings persisted.
- Cold-start measurement script.

## Acceptance criteria

- [x] Cold start to menu ≤ 5 s on the owner's laptop (measured: **1.64 s** cold, **0.85–0.96 s** warm on ASUS FX505DU, 3 runs).
- [x] Create → play → quit → load returns the same world (test `crates/client/tests/session.rs`).
- [x] About screen shows the required notice; no "Minecraft" in UI or crate names. The notice is on the title and About screens; "Minecraft" appears only inside the notice itself, which rule 4 requires.

## Laptop run (owner)

```powershell
cargo build --release -p client
powershell -File scripts\cold_start.ps1      # right after a reboot for the cold number; paste the 3 lines
cargo run --release -p client -- --offline-name dev_01
```

Play for a few minutes:
- walk, sprint, jump, swim, dig, place, Esc → save and quit, then load the world again;
- with F3 on, note the FPS at the 12-chunk profile, and how movement feels (PRD §14).

## Out of scope

Updater binary (stage 3), multiplayer menu (stage 2–3).
