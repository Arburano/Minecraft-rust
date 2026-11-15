# 0007. QUIC and own protocol

Status: Accepted (2026-11-15)

## Context

The Minecraft protocol runs over TCP (head-of-line blocking), changes with every version, and knows nothing about mods, LOD, and shards.

## Decision

One QUIC connection (`quinn`): datagrams for intents and entity deltas, prioritized unidirectional streams for columns and LOD, bidirectional streams for control and events. Messages are enums in `protocol`, serialized with `bitcode`, with the protocol version in the handshake.

## Consequences

- Loss of a packet carrying a chunk does not block entity movement.
- QUIC congestion control also applies to datagrams — explicit priorities are needed.
- There is no compatibility with vanilla clients (ADR-0001).

## Alternatives considered

- TCP + our own multiplexing — head-of-line blocking.
- Bare UDP with our own reliability — years of work on what QUIC provides ready-made.
- The Minecraft protocol — see context.

## References

docs/architecture/networking-protocol.md.
