# T4-06. Grid rendering

- **Status:** todo
- **Depends on:** T4-03, T1-22
- **Owner:** agent (cloud) + laptop

## Goal

Grids are drawn where they are: meshed in local space, drawn with per-object transforms.

## Deliverables

- Per-object model matrices in the render graph (ADR-0015); grid batches; block light per grid, sky light from the world at the grid position.
- Interpolated poses on the client; far grids as one mesh (optional).

## Acceptance criteria

- [ ] 50 grids of 1,000 blocks on the laptop: frame-time cost recorded; no visible seams or jitter (owner review).
