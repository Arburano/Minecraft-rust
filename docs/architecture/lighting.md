# Lighting

Crate: `light`. ADR: 0011.

## Data

- Two 4-bit channels: block light and sky light. `Nibbles` = `Single(u8)` for uniform sections (all sky 15 / all dark 0) or 2,048 bytes.
- Light is stored separately from blocks, under the same `SectionPos` key.

## Who computes it

- The **client** computes light for rendering (the server does not send light — less traffic).
- The **server** computes light only where it affects gameplay (mob spawn, plant growth), lazily, with the same code.
- Same code + same inputs → same result. Test: client light hash = server light hash (T1).

## Algorithm

- Add: a BFS queue from the source, level −1 per step, taking block transparency into account.
- Remove: reverse BFS — clear everything lit by the removed source, collect the boundary with other sources, then refill from the boundary.
- Sky: start from the column's height map, then the same BFS under overhangs and into caves.
- Initial lighting of generated chunks is done in 2×2 column groups so that borders are computed once.
- Light tasks work on a section snapshot (3×3×3 around), and the region applies the result on the next tick. Light reaches ≤ 15 blocks, so with a region buffer of ≥ 1 section it does not leave the region.

## Limits

- A per-tick limit on light updates. The excess (TNT, mass building) is deferred to later ticks; the client temporarily shows the old light.
- Targets: incremental add ≤ 100 µs p95 per edit, removal ≤ 200 µs p95 (T1).

Measured (full recompute, VM): one section 25–45 µs; 3×3×3 sections with 8 torches — 1.4–2.2 ms; sky 3×3×3 — 0.55 ms; 10–35 ns per queue step.

## Tests

- Torch on a section boundary: the incremental result is bit-for-bit equal to a full recompute.
- Torch removal and a broken block under the sky: bit-for-bit equality with a full recompute.
- Property test: random edits, compared with a naive full BFS.
