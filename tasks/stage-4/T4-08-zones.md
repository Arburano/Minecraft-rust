# T4-08. Zones

- **Status:** done (cloud; report: [t4-08-zones](../../task-reports/t4-08-zones.md))
- **Depends on:** T4-01
- **Owner:** agent (cloud)

## Goal

The `zones` capability: named world areas with rules (PRD §14 No. 11), for the RPG and role-play builds.

## Deliverables

- Zone definitions (data and mod API), lookup by position, rules for building, PvP, entry and spawn tables; stored per world.

## Acceptance criteria

- [x] Zone lookup cost per entity per tick measured (VM: 20 ns at 1,000 zones, [zones.md](../../docs/modding/zones.md#cost)); rules enforced server-side (tests): build and entry on the real world (`server/tests/zones.rs`); PvP and spawn tables as server-side queries, tested in `world::zones`, waiting for the combat and spawn systems.
