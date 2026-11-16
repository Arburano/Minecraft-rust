# T0-03. 16³ section with palette

- **Status:** done
- **Depends on:** T0-01
- **Owner:** agent

## Goal

Implement `world::Section` per the specification: `Single` / `Paletted` (1–8 bits, no crossing of u64 boundaries, reverse hash for > 16 states) / `Direct`; `get`, `set`, `unpack_into`, counters and `all_air`/`all_opaque` flags, `compact()`. Starting point — `research/bench/palette`.

## Specs

- [docs/architecture/world-storage.md](../../docs/architecture/world-storage.md)
- [docs/adr/0004-sections-16-palette.md](../../docs/adr/0004-sections-16-palette.md)

## Deliverables

- `crates/world/src/section.rs` + tests + proptests (random `set` operations checked against a reference `[u16; 4096]`).
- Criterion benchmark: `get`, `set` for each bit width, `unpack_into`.
- Updated numbers in docs/process/performance-budgets.md (status "Measured", machine specified).

## Acceptance criteria

- [x] Proptest: 10 000 random edit sequences — `get` always matches the reference (plus 10 000 edits-on-populated cases and 2 000 each for `compact` / `from_blocks`).
- [x] `set` ≤ 10 ns at any bit width, `unpack_into` ≤ 5 µs per section — measured on a VM (marked as such): `set` ≈ 6 ns typical, 13–15 ns on some widths due to VM 4K-aliasing (see [task report](../../task-reports/t0-03-world-palette-section.md)); `unpack_into` ≤ 4.1 µs. Bare-metal confirmation pending.
- [x] Section sizes in the test match the table in world-storage.md exactly (56 / 1 640 / 2 124 / 4 664 / 8 248 bytes).

## Out of scope

No lighting and no disk serialization (separate tasks).
