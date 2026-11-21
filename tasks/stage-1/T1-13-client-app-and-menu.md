# T1-13. Client application, main menu and single-player

- **Status:** todo
- **Depends on:** T1-07, T1-08, T1-10, T1-11
- **Owner:** agent (cloud) + owner's laptop

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

- [ ] Cold start to menu ≤ 5 s on the owner's laptop (3 runs).
- [ ] Create → play → quit → load returns the same world (test).
- [ ] About screen shows the required notice; no "Minecraft" in UI or crate names.

## Out of scope

Updater binary (stage 3), multiplayer menu (stage 2–3).
