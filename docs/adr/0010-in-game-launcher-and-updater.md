# 0010. In-game launcher menu + separate updater

Status: Accepted (2026-11-15, owner decision)

## Context

Target — from click to game in seconds, in the same window. A separate launcher (Tauri) is a second application, a second UI stack, and window switching. A running binary cannot reliably replace itself.

## Decision

There is no separate launcher. The client's main menu (egui on wgpu) handles sign-in, assets, mod sync, and server selection. A tiny separate `updater` verifies the signed manifest, installs binary deltas into a new version folder, atomically switches to it, rolls back on error, and launches the client.

## Consequences

- One UI stack; the game window opens immediately.
- The updater stays minimal and rarely changes.
- The menu must be ready before the world: assets and the world load after the menu is shown.

## Alternatives considered

- Tauri launcher — see context.
- Client self-update without an updater — risk of a broken installation.

## References

PRD §7, §14 No. 9; docs/architecture/client-menu-and-updater.md.
