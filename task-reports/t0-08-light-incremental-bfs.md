# T0-08: incremental light add/remove

## What was done

Implemented the `light` crate per `docs/architecture/lighting.md` and ADR-0011:

- `Nibbles` (`Single(u8)` or 2,048 packed bytes) and `SectionLight` (block + sky), with compaction of uniform sections.
- `LightNeighborhood`: a 3×3×3 section snapshot (48³ cells) stored as a padded 50³ grid with a sentinel border of opacity 15, so the BFS moves by constant index offsets. Block properties come from the caller as `LightProps { emission, opacity }`; the engine knows no blocks.
- One equation for both channels: `L(c) = max(src(c), max_n L(n) − 1 − opacity(c))`, least fixed point inside the snapshot. `src` is the emission (block) or 15 in the column's exposed sky run (sky). The sky run comes from the heightmap ("open above the snapshot") and the blocks inside the snapshot, and edits move it.
- `full_relight` (BFS from every source), `relight_add` (one BFS from the changed cell; rejects darkening changes with `WouldDecrease`), `relight_remove` / `set_block` (reverse BFS from every cell that lost light or a source, boundary collection, re-seeding of sources, refill). Queues are reused, so an edit does not allocate after warm-up.
- `reference::naive_light`: an independent relaxation (sweeps until stable) that shares no code with the BFS.
- Spec updated with the model, the sky-run rule, exactness (center section exact, outer ring approximate) and the measured numbers. The budget row moved to Measured (VM).

Files: `crates/light/{Cargo.toml, src/lib.rs, src/nibbles.rs, src/neighborhood.rs, src/reference.rs, tests/light.rs, benches/light.rs}`; docs `architecture/lighting.md`, `process/performance-budgets.md`; task file and `tasks/README.md`.

Tests:

- 3 unit tests (nibbles).
- 7 integration tests:
  - torch levels and going around a wall;
  - a torch on section boundaries;
  - torch removal with another source refilling;
  - a broken cave roof under the sky, with leaves attenuating direct sky;
  - `relight_add` rejecting darkening changes;
  - compact section export;
  - the `Section`-based constructor matching the props constructor and the naive reference.
- 1 proptest: 48 cases by default; also run once with `PROPTEST_CASES=400`, all passing. Each case is a random terrain-like snapshot (stone/leaves/water blobs, torches, glowstone and lanterns, random open columns) with 1–23 random edits (half of them in the center section). After every edit the incremental result is compared with a fresh full recompute, and at the start and end of each case with the naive reference.
- Mutation check: two deliberate bugs — weakening the reverse-BFS zeroing condition, and not extending the sky run when its bounding block is removed — each made the proptest fail. The code was restored afterwards.
- `cargo fmt --check`, `cargo clippy --workspace --all-targets` with `-D warnings`, and `cargo test --workspace` are green.

Benchmark (`cargo bench -p light`): cloud VM, 4 vCPU Intel Xeon 2.1 GHz (KVM). These are **VM numbers** with ±30% noise expected. 3 runs. Corpus: `worldgen::test_corpus` seed 0, 13 snapshots centered on sections (1..=4, 2, 1..=4) whose center section (world y 32..48) has both air and diggable cells, with 8 torches per section in random air cells. 64 edit cells per snapshot, 2,496 edits per kind per run. p50/p95 come from per-edit `Instant` timing (range over the 3 runs); Criterion means are from run 1.

| Edit | Path | p50 | p95 | Criterion mean | Budget |
| --- | --- | --- | --- | --- | --- |
| place torch | add | 1.0–1.3 µs | 2.7–3.4 µs | 1.43 µs | ≤ 100 µs p95 ✓ |
| break block | add | 0.1 µs | 0.3–0.4 µs | 0.14 µs | ≤ 100 µs p95 ✓ |
| remove torch | remove | 5.8–7.5 µs | 12.8–16.8 µs | 6.54 µs | ≤ 200 µs p95 ✓ |
| place block | remove | 0.8–1.0 µs | 3.8–5.2 µs | 1.34 µs | ≤ 200 µs p95 ✓ |
| worst: torch in a dark empty snapshot | add / remove | 14.1–17.9 / 10.4–15.8 µs | 18.3–24.3 / 12.5–18.0 µs | 16.3 / 11.2 µs | ✓ |
| worst: cave roof (28×7×28 cave) under open sky | add / remove | 15.4–18.0 / 10.8–13.8 µs | 19.0–24.5 / 11.3–15.3 µs | 15.4 / 13.3 µs | ✓ |
| full recompute 3×3×3, both channels | — | 0.91–1.23 ms | — | 0.95 ms | — (spec: 1.4–2.2 ms on the research VM) |

Single-edit maxima reached 50–124 µs on the VM, which looks like scheduler or cache noise: the worst-case p95 is ~25 µs. Even these maxima are below the remove budget, but one torch-add maximum of 123.7 µs is above the add budget's p95 value. The budget is p95, so it is met.

## What went wrong

- The first benchmark run panicked with a division by zero: 3 of the 16 corpus snapshots have a center section with no air cells (fully submerged or solid). Those snapshots are now skipped and counted in the output.
- Clippy (`-D warnings`) flagged three unnecessary `i32` casts in the benchmark. Fixed.
- The corpus numbers on their own were optimistic: with 8 torches per section most cells are already brightly lit, so a new torch touches few cells. Two synthetic worst cases were added so the budget check does not rest on an easy corpus.
- One expected value in a unit test was wrong (light behind a wall: 9, not 10). The test was corrected; the implementation was right.

## Issues encountered

- **Model choice (no ADR impact):** any block with opacity > 0 (leaves, water) stops *direct* sky. Light still reaches below such blocks sideways and through them with attenuation `1 + opacity`. Attenuation values are registry data from mods; the benchmark uses water/leaves = 1.
- **Snapshot boundary:** only the center section is exact; the outer ring treats cells outside the snapshot as dark. This matches the spec (light reaches ≤ 15 blocks; the region applies the center result). Applying results to a region, initial lighting in 2×2 column groups and per-tick limits are out of scope (region integration).
- The snapshot is rebuilt from sections and lit with `full_relight` (~1 ms). Importing stored `SectionLight` instead, so that edits start from saved light, is not implemented. It is needed when this is integrated with regions (stage 1).
- VM numbers only at the time of writing. Bare-metal confirmation done in T0-16 (2026-11-21, `cargo bench -p light`, 3 runs on the owner's laptop): add/remove budgets met — corpus p95 ≤ 2.8 / ≤ 14.2 µs, synthetic worst cases p95 ≤ 24.7 / ≤ 20.5 µs. See the [T0-16 summary](../docs/research/bare-metal-2026-11-21/summary.md).
