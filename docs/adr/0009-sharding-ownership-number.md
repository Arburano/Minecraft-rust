# 0009. Sharding with ownership number

Status: Accepted (2026-11-15)

## Context

5 000+ players need several nodes. Entity handoff between nodes must not dupe or lose items. Core NATS delivers "at most once"; classic 2PC hangs when the coordinator fails.

## Decision

Spatial cells belong to nodes; the client connects to a gateway. Each entity has an ownership number that increases on handoff; receiving is idempotent, and commands carrying an old number are discarded; delivery is "at least once" (JetStream or a direct channel with acknowledgment). Shadow copies in the overlap zone are for display only; actions across the boundary are a command to the owner.

## Consequences

- No dupes on retries and failures (verified by 10 000 automated tests).
- Actions across the boundary incur network latency between nodes.
- The owner's source of truth is open question No. 3.

## Alternatives considered

- 2PC — see context.
- A MultiPaper-like chunk master — beta, and does not describe entity movement.

## References

docs/architecture/sharding.md.
