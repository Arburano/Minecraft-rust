# T0-04. Gather, incremental masks and fast_mesh for the 32³ mesh chunk

## What was done

Implemented the `mesh` crate per `docs/architecture/meshing-and-rendering.md` and ADR-0005:

- `SectionNeighborhood` — the 4×4×4 sections touched by a 32³ mesh chunk and its one-block padding (`from_fn` by absolute `SectionPos`, `from_array`, accessor by local `-1..=2` coordinates).
- `MeshChunkBuilder` — reusable per-worker pipeline: `gather` (34³ padded ZXY buffer + masks), `build` / `build_into` (load path), `set_block` + `remesh_into` (edit path, no allocations after warm-up). Empty and solid chunks skip the mesher via section flags (`non_air` counters, `all_opaque`); a solid core with air padding still meshes its boundary faces.
- Gather: the 8 core sections are unpacked word by word (`Section::unpack_into`); uniform sections are filled without unpacking with whole bit-range mask updates; the thin padding shell is read per cell (see below).
- `MaskCache` — opaque/transparent column masks (one u64 per padded (y,x) column), `rebuild` from scratch and `on_block_set` incremental single-bit update.
- `mesh::affected_mesh_chunks` — dirty set after an edit: the chunk itself plus up to 3 face neighbors on a border (up to 4 total; diagonal AO neighbors are a later task).
- `mesh::reference` — naive per-face reference mesher and greedy-quad coverage decoding via the crate's own `Face::vertices_packed` geometry; comparisons use covered unit faces, not quad counts.
- `binary-greedy-meshing` 0.6.4 added to the crate (version already pinned in `[workspace.dependencies]`); `noise` as a dev-dependency for the benchmark corpus (also already a workspace dependency).

Files: `crates/mesh/src/lib.rs`, `crates/mesh/src/reference.rs`, `crates/mesh/tests/mesh_proptest.rs`, `crates/mesh/benches/mesh_chunk.rs`, `crates/mesh/Cargo.toml`; docs `meshing-and-rendering.md`, `performance-budgets.md`.

Tests: 9 unit tests (gather correctness incl. padding corners, uniform vs paletted gather, empty/solid skip, solid-core-with-air-padding boundary, incremental masks vs rebuild, dirty-set interior/face/edge/corner and negative coordinates, single-block mesh vs reference) + 2 proptests × 512 cases: random 34³ fields (4 shapes, random transparency assignment) — greedy output covers exactly the reference faces; random edit sequences — masks equal a full rebuild and the re-mesh still matches the reference.

Benchmark (VM 2 vCPU, ±30% noise, criterion, seeded Perlin terrain corpus 4×2×4 mesh chunks, composition 29.8% air / 62.0% opaque / 8.1% transparent, ≈ 535 quads/chunk):

| Operation | Before (hypothesis / 62³ research) | After (VM, 32³) | Budget |
| --- | --- | --- | --- |
| gather + masks | ≤ 100 µs (hypothesis) | ≈ 60 µs | ≤ 100 µs p50 ✓ |
| `fast_mesh` | 0.27 ms at 62³ | ≈ 42 µs | ≤ 0.2 ms p50 ✓ |
| full build (gather + masks + mesh) | — | ≈ 99 µs | — |
| re-mesh after an edit | — | ≈ 30 µs | — |
| mask update per edit | ≤ 20 ns (hypothesis) | ≈ 2.6 ns | ≤ 20 ns ✓ |
| mask rebuild from scratch | 0.35 ms at 62³ | ≈ 41 µs | — |

All numbers are VM measurements; bare-metal confirmation pending (same caveat as T0-03).

## What went wrong

- First gather implementation mapped section-local coordinates to padded coordinates without subtracting the intersection start, so shell sections landed at the wrong buffer offset. Caught by the `gather_matches_source_field` unit test at the (-1,-1,-1) padding corner; fixed by making the padded origin relative to the intersection box.
- The first version unpacked all 64 sections word by word: gather + masks measured ≈ 188 µs, over the ≤ 100 µs budget. The 56 shell sections each contribute only 1..=256 padding cells, so decoding 4 096 cells per section was wasted work. Switched the shell to per-cell `get` (6 536 cells total): ≈ 60 µs, a 3.1× improvement. This deviates from the spec's "word-by-word, not per-block get()" wording; the spec was updated with the measured justification (word-by-word remains for the 8 core sections, 83% of the cells).
- Benchmark smoke run panicked: the mask-update bench used chunk-local y = 40 (out of 0..=31). Fixed to local coordinates.
- Clippy: `too_many_arguments` (grouped the intersection bounds into a `SectionBox` struct), `needless_range_loop`, `manual_range_patterns`, `type_complexity` in the bench, and criterion 0.8's deprecated `black_box` (switched to `std::hint::black_box`).

## Issues encountered

- The `binary-greedy-meshing` quad layout differs per face direction (the x/y/z fields swap roles); the reference decoder therefore uses the crate's own `Face::vertices_packed` corner geometry and a bounding-box expansion instead of hand-decoding each face. Verified against the naive mesher on 1 024 proptest cases.
- "Solid chunk" skip is conservative: it requires all 64 neighborhood sections to be fully opaque. A solid core surrounded by opaque neighbors in only some directions still meshes (correct, slightly more work); per-face skip flags can be added later if profiling asks for it.
- The corpus transparent share is 8.1% (target "8%"): tuned via sea level and tree count; composition is printed by the bench on every run.
- VM numbers only; the usual ±30% noise caveat applies. No GPU/render-batch work (T0-05) and no diagonal-AO dirty neighbors yet.