# Sharding across nodes

Stage 6. Crates: `server` + future `gateway` and `coordinator` (created in stage 6). ADR: 0009.

## Components

```text
client ══QUIC══► gateway ──► node A (cells 0..k)
                    └──► node B (cells k+1..)
            coordinator: cell → node map, rebalancing
            bus: NATS JetStream (or direct node↔node channels)
            global services: chat, friends, economy, profiles (PostgreSQL)
```

- The **gateway** holds the client connection and routes traffic to the node that owns the player's position. Crossing between nodes is invisible to the client (no reconnect).
- The **coordinator** assigns cells (e.g. 512×512 blocks) to nodes. Start: a static grid; dynamic re-partitioning (a hot cell splits) — later.
- **Overlap zones** of width ≥ the visibility radius: a node sees the edge of its neighbor read-only (shadow copies of entities) so players can see each other across the border.

## Rules

1. A shadow copy is for display only. Any action across the border (a hit, an explosion, a block change in the overlap zone) is a command to the owning node; the result arrives with an inter-node delay.
2. Each entity has an **ownership number** (u64) that grows with every handoff. A command with a stale number is discarded.
3. Handoff: serialize components → message with ownership number N+1 → idempotent receive (a repeat does not create a copy) → acknowledgment → the source stops considering itself the owner. The entity is frozen for 1–2 ticks; interpolation hides this from the player.
4. "At least once" delivery (JetStream or a direct channel with acknowledgment) + idempotency. We do not use core NATS ("at most once") or classic 2PC (hangs when the coordinator crashes).
5. The source of truth about the owner is the event log. What exactly (JetStream or PostgreSQL with a record version) is an open question.

## Handoff target

≤ 1 tick (50 ms) and unnoticeable to the player. Not "< 1 ms".

## Limits of sharding

- 5,000 players / 500 per node = 10 nodes only with an even distribution.
- A hot spot (an event with 2,000 players in one place) is not solved by sharding: an instance and queue policy is needed (a game design level decision).

## Tests (stage 6 exit)

- Two nodes, RTT 100 ms, 1,000 round-trip crossings: 0 duplicates, 0 losses, p99 ≤ 50 ms.
- 200 bots standing on the node border: tick of both ≤ 25 ms.
- Node crash during a handoff: after recovery, no item duplication (10,000 automated tests).
