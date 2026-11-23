# T2-06. Regional multithreading on the real server

- **Status:** todo
- **Depends on:** T2-02, T0-09 (prototype)
- **Owner:** agent (cloud)

## Goal

Bring the T0-09 region prototype ([server-regions.md](../../docs/architecture/server-regions.md), ADR-0008) to the real world and lift its relaxations:

- regions own real columns and sections (vertical extent included) and their players;
- they merge and split with the ≥ 1-section buffer;
- they tick in parallel, exchanging messages N → N+1;
- the network and storage threads hand over to region owners without locks around the world (AGENTS.md rule 3).

## Acceptance criteria

- [ ] 1,000-entity merge/split property test: none lost or duplicated; the same inputs and partitioning give the same state hash.
- [ ] 200 bots in 8 regions: tick p99 ≤ 25 ms on the laptop (8 cores); the maximum for 1 region is measured (single-region capacity, server-regions.md).

## Out of scope

Shards (stage 6), pinning (stage 6).
