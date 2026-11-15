# 0002. bevy_ecs without the rest of Bevy

Status: Accepted (2026-11-15)

## Context

We need an ECS with automatic parallel scheduling of systems based on declared access. Full Bevy pulls in a general-purpose renderer that is not optimized for millions of voxels, and breaks its API every 3–4 months.

## Decision

We use only the `bevy_ecs` crate (version pinned in the workspace). Rendering, windowing, assets, UI — our own or separate crates (wgpu, winit, egui). Each server threading region is a separate `World`.

## Consequences

- A proven system scheduler, SoA archetypes.
- `bevy_ecs` upgrades are planned, every six months, in a separate PR.
- A thin abstraction layer over `World` in `sim`, so version migrations touch a single place.

## Alternatives considered

- Full Bevy — see context.
- Our own ECS — months of work with no benefit.
- `hecs`/`flecs` — hecs has no built-in parallel scheduler with declared access, and flecs is C.

## References

PRD §2.
