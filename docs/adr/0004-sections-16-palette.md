# 0004. 16³ sections with a palette

Status: Accepted (2026-11-15)

## Context

Client memory ≤ 2 GB, server ≤ 16 GB for 500 players. A flat u16 per block — 8 KB per section; a test showed 1.6–2.1 KB for typical terrain with a palette and 56 B for uniform sections.

## Decision

The storage unit is a 16³ section: `Single`, `Paletted` (1–8 bits, indices without crossing u64 boundaries, reverse hash for palettes > 16), or `Direct`. Light is stored separately, as nibbles, with a `Single` variant. Storage and networking use the same palette format + zstd.

## Consequences

- 15 000 sections ≈ 17 MiB (assumed mix) instead of 117 MiB.
- `get` ≈ 3.8 ns instead of 1 ns — offset by word-by-word unpacking for meshing.
- Property tests for palette growth/shrinking are needed.

## Alternatives considered

- Flat array — 4–7× more memory.
- Octrees/SVO — slow editing, more complex meshing.
- 32³ sections — worse granularity for light, networking, and uniform zones.

## References

docs/architecture/world-storage.md; docs/research/optimization-tests.md.
