# T0-12: worldgen test corpus

## What was done

Implemented the deterministic test terrain in `crates/worldgen/src/test_corpus.rs`
(re-exported from `worldgen`):

- `test_corpus(seed, &CorpusArea) -> Vec<TestColumn>` and
  `test_column(seed, x, z, y_range) -> TestColumn`. `CorpusArea` is in section
  coordinates (end-exclusive); a `TestColumn` is a stack of `world::Section`.
- Terrain: 4-octave Perlin heightmap (base 36, amplitude 16), sea level 44,
  3D-noise caves, deterministic 2x2x2 ore clusters in stone (coal ~1%, iron ~1%
  below y 24), trees (24 candidates per 32x32 cell). Parameters follow the
  `research/bench/meshing` generator as adapted by the T0-04 bench corpus.
  New in T0-12: ores and seed-derived noise (splitmix64 domain separation).
- `TestColumn::hash()`: FNV-1a 64 over coordinates and every block ID in a fixed
  order; representation-independent (verified against `Section::compact`).
- Determinism by construction: each column is a pure function of
  `(seed, x, z, y_range)`. Trees never leave their 32x32 placement cell (trunks
  at cell offset 3..=28, leaves reach 2 blocks), and a 16-wide column lies
  inside one cell, so no column needs neighbor data.
- `composition(sections)` helper and the corpus-local `is_transparent`
  predicate (water/leaves/glass, ids 5..=7); ids 8/9 are coal/iron ore.

Benchmark wiring (acceptance criterion 2):

- `mesh` benches `mesh_chunk` (T0-04) and `render_batch` (T0-05) now cut their
  sections from `test_corpus(0, x: -1..9, y: -1..5, z: -1..9)`; the ~130-line
  inline generator and the `noise` dev-dependency were removed from `mesh`.
- `world` bench (T0-03) gained the `section_corpus` group: get/set/unpack on
  real terrain sections (deep 2-bit, surface 3-bit, canopy single) from
  `test_column(0, 0, 0, -1..5)`. The synthetic width sweep is unchanged.
  `worldgen` is a dev-only dependency of `world` (dev-dependency cycle, allowed
  by cargo; `worldgen` itself depends on `world` normally).
- T0-08 (`light`) is still a skeleton; its benchmark must use this corpus when
  implemented, as already planned in `tasks/README.md` (wave 2 waits for T0-12).

Tests (all green, `RUSTFLAGS="-D warnings"`):

- 6 unit tests: composition and block-id coverage (transparent share of
  non-air asserted in 4..15%, measured 8.0%), seed sensitivity,
  representation-independent hash, single-column vs larger-area equality,
  empty area/y range, section accessor.
- `tests/determinism.rs`: 25 columns, reverse order, and 4 rotated orders on
  1/2/3/4/8 threads (std threads) — all per-column hashes identical.
- `tests/corpus_proptest.rs`: 32 random seeds/areas, corpus vs per-column
  regeneration in HashMap order — identical hashes.
- Full workspace `fmt --check`, `clippy --all-targets`, `test`,
  `bench --workspace --no-run` green.

Benchmarks — one criterion run each (default 100 samples). **Machine: cloud VM,
Intel Xeon Platinum, 4 vCPU, 8 GiB RAM, Linux (KVM guest); VM numbers, noise
±30%, not bare metal.** Toolchain: stable 1.98.1 (the pinned 1.97 could not be
downloaded in this sandbox; `rust-toolchain.toml` unchanged, per AGENTS.md).

`cargo bench -p mesh --bench mesh_chunk` on the new corpus (seed 0):

- Composition over the 128x128x64 grid: air 28.8%, opaque 65.5%, transparent
  5.7% of all blocks = **8.0% of non-air** (the ~8% target).
- `gather_masks` 88.9 µs, `fast_mesh` 49.0 µs, `build` 138.1 µs, `remesh`
  51.8 µs, `mask_update` 3.71 ns, `mask_rebuild` 49.5 µs; 599 quads/chunk.
- `render_batch`: 5 410 quads / 42.3 KB; `edit_subrange` 146 ns, `pack_all`
  1.11 µs, `edit_recopy_all` 1.24 µs.
- All budget rows still met (gather ≤ 100 µs, fast_mesh ≤ 0.2 ms, mask update
  ≤ 20 ns, batch copy ≤ 50 µs). Updated in `performance-budgets.md`.

`cargo bench -p world --bench section -- section_corpus` (per-op = time / 4096):

- `get`: 3.98 ns deep, 3.97 ns surface, 0.61 ns canopy (single).
- `set` (existing palette states): 12.4 ns deep, 13.0 ns surface, 4.9 ns canopy.
- `unpack_into`: 3.28 µs deep, 3.31 µs surface, 0.085 µs canopy.

Files touched: `crates/worldgen/` (module, tests, manifest),
`crates/world/Cargo.toml` + `benches/section.rs`,
`crates/mesh/Cargo.toml` + `benches/mesh_chunk.rs`,
`docs/architecture/worldgen.md` (new "Test corpus" section),
`docs/process/testing-and-benchmarks.md`, `docs/process/performance-budgets.md`,
task file + `tasks/README.md`.

## What went wrong

1. **Toolchain mirror failures (environment, not code).** The pinned 1.97
   toolchain and the clippy/rustfmt/wasm32-wasip2 components for stable returned
   404 from the configured Tsinghua mirror, and the installed toolchain manifest
   pins mirror URLs so `RUSTUP_DIST_SERVER` overrides do not help. Installed the
   components manually from `static.rust-lang.org` tarballs into the stable
   toolchain. Used `RUSTUP_TOOLCHAIN=stable` for all commands; the pinned file
   was not touched.
2. Two small compile errors on the first pass (ambiguous integer type in the
   leaf-blob loops, a borrow conflict in the test's `rotate_left`); fixed
   immediately.
3. The first full `cargo test --workspace` failed in `modhost`'s component
   tests because the stable toolchain lacked the `wasm32-wasip2` std; after
   installing it, all tests pass. Unrelated to this task's changes.

## Issues encountered

- **Gather moved from ≈ 60 µs to ≈ 89 µs (VM).** Not a code regression: the
  corpus changed. Ores turn deep sections from uniform stone into 2-bit paletted
  sections, so gather unpacks real palettes instead of filling a constant, and
  the seed-derived noise shifts the terrain. Still within the ≤ 100 µs budget;
  the before/after numbers are not comparable and both are VM-only. The
  bare-metal re-run (wave 3 in `tasks/README.md`) covers this.
- **`section_corpus/set` reads 12–13 ns on 2–3-bit terrain sections**, above the
  ≤ 10 ns target, while the synthetic sweep at the same widths reads ≈ 6 ns.
  This matches the address-dependent 4K-aliasing behavior documented in the
  T0-03 report (readings shift with allocation layout). Recorded with the same
  VM caveat; bare-metal confirmation is already a stage-0 carry-over.
- **T0-08 benchmark does not exist yet** (`light` is a skeleton, task todo).
  The acceptance criterion "Benchmarks T0-03/04/08 use this corpus" is satisfied
  for T0-03/04/05 now; T0-08 is bound to this corpus by the wave plan in
  `tasks/README.md` and by the note in `docs/architecture/worldgen.md`.
- The old T0-04/T0-05 corpus used fixed noise seeds 1/2; the new corpus derives
  noise from the seed, so absolute terrain (and quad counts) differ slightly
  even at the same parameters. All comparisons across the switch are marked as
  corpus changes, not optimizations.
