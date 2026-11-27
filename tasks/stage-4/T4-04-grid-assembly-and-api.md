# T4-04. Grid assembly, disassembly, splitting and the grids mod API

- **Status:** todo
- **Depends on:** T4-03 (WIT part: stage 3 mod API)
- **Owner:** agent (cloud)

## Goal

Players' constructions become grids and back; mods drive them.

## Deliverables

- Assembly from a seed block (flood fill with mod glue rules), disassembly with snapping to 24 orientations, splitting on disconnection.
- `engine:api/grids` WIT interface linked only with the capability: assemble, disassemble, apply-force/torque (batched), grid-state, block access in grid coordinates, impact/split/sleep events.
- Fragile blocks breaking on impact.

## Acceptance criteria

- [ ] Assembly → disassembly round trip leaves the world identical (test).
- [ ] A test mod lifts and turns a grid through the API.
