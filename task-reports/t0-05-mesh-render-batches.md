# T0-05. Render batches of 2×2×2 mesh chunks

## What was done

Implemented `mesh::RenderBatch` per `docs/architecture/meshing-and-rendering.md` and ADR-0005 (new module `crates/mesh/src/batch.rs`):

- `RenderBatch` — quads of 8 mesh chunks (2×2×2 slots, y-major order `(sy*2+sz)*2+sx`) copied into a 64³ batch with the 0/32 offset per axis. Stored as 8 sub-ranges of 6 per-direction lists in batch coordinates (`set_chunk` re-copies one slot, reusing capacity — the sub-range update variant).
- `PackedBatch` + `RenderBatch::pack_into` — concatenation of all 8 slots into 6 contiguous per-direction ranges, one GPU range/draw per direction (the full re-copy variant). No allocation after warm-up.
- The offset is applied as a XOR of bit 5 of each 6-bit coordinate field: adding 32 modulo 64 is exactly a bit-5 flip, so width/height/id bits are never touched and there are no cross-field carries.
- **Coordinate convention (mod 64):** quad anchors are not always min corners — for Up/Down/Right/Front some fields store a positive face plane or a max corner, which can be 32 chunk-locally and therefore 64 in the batch, overflowing the 6-bit field. Such max-corner fields are always ≥ 1 in true coordinates, so a stored 0 there unambiguously means 64; min-corner fields never wrap. Documented in the module and the spec; the future vertex shader must apply the same unwrap (noted for T0-06).
- `reference::batch_faces` — decoder of batch quads into unit faces with exact batch-local block coordinates 0..=63 (the crate's `Face::vertices_packed` cannot represent 64, so the anchor layout is decoded directly; the layout table is cross-checked against the crate-based `covered_faces` decoder on unshifted chunks in the tests).

Files: `crates/mesh/src/batch.rs` (new), `crates/mesh/src/reference.rs`, `crates/mesh/src/lib.rs`, `crates/mesh/tests/mesh_proptest.rs`, `crates/mesh/benches/mesh_chunk.rs`; docs `meshing-and-rendering.md` (render-batch section + quad format note), `open-questions.md` (No. 8 resolved), `performance-budgets.md` (batch copy row → Measured).

Tests: 5 new unit tests (bit-5 flip keeps w/h/id, slot replacement semantics, pack concatenation order + buffer reuse, boundary planes at 64 decode to block 63, batch decoder == crate decoder at offset 0 and == shifted chunk faces for all 8 slots) + 2 new proptests: `batch_positions_equal_chunk_plus_offset` (512 cases, random valid quads in all 8 slots: decoded anchor = chunk anchor + offset mod 64, w/h/id preserved, packing concatenates) and `batch_faces_equal_shifted_chunk_faces` (256 cases, real mesher output on random 34³ fields in all 8 slots: covered unit faces equal the chunk's faces shifted by the slot offset, exact coordinates). All 14 mesh unit tests + 4 proptests pass; `world` tests and the dependent `render` skeleton still build clean. `cargo clippy -p mesh -p render --all-targets` with `-D warnings` clean.

Benchmark (criterion, 100 samples per bench, cloud VM 4 vCPU Xeon Platinum, numbers ±30% VM noise; same seeded T0-04 terrain corpus, one 64³ batch from the 2×2×2 chunks at the grid origin: 4 976 quads, 38.9 KB; `mesh` bench `mesh_chunk/render_batch`):

| Variant after an edit | Time (VM) | Budget |
| --- | --- | --- |
| B: re-copy the edited slot's sub-ranges (`edit_subrange`) | ≈ 218 ns | — |
| A: full re-copy (`edit_recopy_all` = set_chunk + pack) | ≈ 1.63 µs | ≤ 50 µs ✓ (≈ 30× margin) |
| pack alone (`pack_all`) | ≈ 1.42 µs | — |

**Recommendation (open question No. 8): keep the full re-copy.** It is ~30× under budget; the sub-range variant saves only ~1.4 µs per edited chunk but needs 48 GPU sub-ranges with their own counters (or 48 draws instead of 6). Even an edit dirtying 4 mesh chunks stays at ≈ 6.5 µs of copying. `RenderBatch` keeps the per-slot storage so the sub-range variant remains available if bare-metal or real-world numbers ever demand it. Bare-metal confirmation pending, as with T0-03/T0-04.

## What went wrong

- The spec's claim "the crate's 6 bits (0..63) fit the entire batch" is subtly wrong: anchor planes/max corners reach 32 chunk-locally, so the +32 offset produces 64, which does not fit 6 bits (and a naive integer add would carry into the width field). Resolved with the mod-64 convention above (XOR bit-5 flip + unambiguous unwrap), documented in the spec and module docs; no ADR change needed (still 8-byte quads, 6 ranges, 64³ batches). Caught by reasoning over the crate's `Face::vertices_packed` before writing code, confirmed by the boundary unit test.
- First draft of a unit test assumed `set_chunk` on one slot clears another; fixed the test (slots are independent).
- `prop_assert_eq!` cannot capture variables from format strings expanded through `concat!`; switched to positional format args.
- The pinned 1.97 toolchain and the configured rustup mirror cannot serve components in this sandbox (404); used `RUSTUP_TOOLCHAIN=stable` (1.98.1) per AGENTS.md and installed rustfmt/clippy manually from the official dist server. `rust-toolchain.toml` unchanged.

## Issues encountered

- The vertex shader (T0-06, paths B/C) must unwrap max-corner anchor fields: a stored 0 in a positive face plane or in the width-max corner of Down/Right/Front means 64. The layout table lives in `batch::max_corner_field` and `reference::batch_faces`; the spec's quad-format section now states the convention.
- No GPU upload in this task (out of scope); `PackedBatch` is the upload-ready layout.
- VM numbers only (4 vCPU cloud VM, unlike the 2 vCPU research VM of earlier reports); the ±30% noise caveat applies, but the 30× margin to the 50 µs budget makes the variant decision robust to it.