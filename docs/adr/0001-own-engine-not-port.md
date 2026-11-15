# 0001. Own engine, not a port of Minecraft

Status: Accepted (2026-11-15)

## Context

The goal is a game that can be fully reworked by mods, running at 60 FPS on a weak PC and with thousands of players. Rewriting Minecraft "as is" in Rust would carry over the single-threaded tick, the entity class hierarchies, and a protocol that changes with every version. Compatibility with Java mods is impossible in principle: they modify the game's Java classes.

## Decision

We are building our own voxel engine. It is compatible with Minecraft assets (loaded by the player), but not with the vanilla server protocol or with Java mods. All gameplay is the `core` mod built on the public API. Mechanics are implemented from our own design, without Mojang code.

## Consequences

- Engine optimizations apply to all mods; the API is validated against our own content.
- The scope is years (PRD §9); strict exit criteria for stages are required.
- Vanilla clients cannot join our server; our client cannot join a vanilla server.

## Alternatives considered

- Porting the Minecraft client to Rust — carries over the architectural bottlenecks, and is legally risky.
- Fabric + optimization mods (Sodium, Lithium, C2ME, Distant Horizons) — realistic right now, but the ceiling for performance and for the modding model is low. Remains a parallel track for the community.
- Azalea/Valence as a foundation — they tie us to the Minecraft protocol; we take them as code references.

## References

PRD §1–2.
