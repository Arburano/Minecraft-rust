# T4-03. Moving grids: storage and physics

- **Status:** todo
- **Depends on:** T4-01, T4-02
- **Owner:** agent (cloud)

## Goal

[ADR-0019](../../docs/adr/0019-moving-block-structures.md), [moving-structures.md](../../docs/architecture/moving-structures.md): grids with their own sections and pose, simulated by Rapier in the region tick.

## Deliverables

- `Grid` (sections, pose `f64` + quaternion, velocity, mass properties) and the region grid index; `rapier3d` in `[workspace.dependencies]` behind the `moving_grids` feature.
- Block `[physics]` data and tags; mass, center of mass and inertia summed incrementally, with a reference implementation.
- Colliders: per-section box merge, rebuilt per edited section; terrain colliders near grids cached by section version.
- Forces: gravity, drag, buoyancy, `lift` blocks; a native force API for tests (the WIT API comes with T4-04).
- Sleeping, per-region limits (`data/grids/limits.toml`), freezing over budget; saving in a grid store.
- Region ownership and handoff; swept bounds count in the partition.

## Acceptance criteria

- [ ] Benchmarks: 50 grids × 1,000 blocks ≤ 2 ms physics per region tick; one section collider rebuild ≤ 50 µs; no allocations per tick at steady state (state the machine).
- [ ] Deterministic golden poses after N ticks.
