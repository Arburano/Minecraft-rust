# T0-03: world palette section

## What was done

Implemented `world::Section` (ADR-0004, `docs/architecture/world-storage.md`) in
`crates/world/src/section.rs`, re-exported from `crates/world/src/lib.rs`:

- Three representations chosen by distinct-state count: `Single` (uniform),
  `Paletted` (2..=256 states, 1–8 bit indices that never cross a `u64` word),
  `Direct` (flat `Box<[BlockStateId; 4096]>` above 256 states).
- `get`, `set` (palette growth and 1→8 bit→direct repacking), `unpack_into`
  (word-by-word), `compact` (minimal repack, PostTick/save only).
- Intrinsic `non_air` counter maintained on every edit; `all_air()` from it;
  `all_opaque(is_opaque)` takes the registry opacity predicate (the engine does
  not know block semantics).
- `ReverseIndex`: a compact open-addressing hash table (one `u32` per slot packing
  key + palette index + occupancy bit, multiply-shift hash, load factor ≤ 0.5) for
  palettes > 16, keeping `set` O(1).
- `storage_bytes()` reproduces the spec size model; the whole `Section` is 56 bytes.

Files touched: `crates/world/src/section.rs` (new), `crates/world/src/lib.rs`,
`crates/world/Cargo.toml` (dev-deps `criterion`, `proptest`; `[[bench]] section`),
`crates/world/benches/section.rs` (new), `crates/world/tests/section_proptest.rs`
(new), `docs/architecture/world-storage.md`, `docs/process/performance-budgets.md`,
task file + `tasks/README.md`.

Tests (all green, `RUSTFLAGS="-D warnings"`, rustc 1.97.1):

- 15 unit tests, including exact spec sizes (56 / 1640 / 2124 / 4664 / 8248 bytes),
  growth through every width to direct, compact shrinking, counter tracking, and a
  regression test for palette index 255 / key 0 in the reverse index.
- 4 property tests against a reference `[u16; 4096]`: **10 000** random edit
  sequences (`section_matches_reference`), 10 000 edits on populated sections,
  2 000 compact-preserves-content, 2 000 `from_blocks` round-trips. Every index,
  `unpack_into`, and the `non_air` counter are checked per case.
- Full workspace `fmt --check`, `clippy --all-targets`, `test`, `bench --no-run` green.

Benchmarks — `cargo bench -p world --bench section` (criterion, 100 samples,
1 s warm-up, 2–3 s measurement). **Machine: cloud VM, Intel Xeon, 4 vCPU, Linux
(KVM guest); VM numbers, noise ±30% or more, not bare metal.** Per-op = group time
/ 4096:

- `get`: ≈ 3.4 ns paletted (all widths), 0.5 ns single, 0.7 ns direct — meets the
  ≈ 3.8 ns reference.
- `unpack_into`: ≈ 2.9–4.1 µs paletted, < 0.1 µs single/direct — meets ≤ 5 µs.
- `set` (existing palette entry, no growth): ≈ 6 ns typical (single 2–3.5 ns,
  1–5 bit ≈ 6 ns, direct ≈ 3–4 ns). Some 6–8-bit readings were 13–15 ns; see below.

## What went wrong

1. **std `HashMap` reverse index was too slow.** First implementation used
   `Option<Box<HashMap<BlockStateId, u8>>>`; `set` at 8 bit measured ~14.6 ns
   (SipHash + hashbrown probe overhead), over the ≤ 10 ns target. Replaced with the
   custom `ReverseIndex`.
2. **Separate key/value arrays caused a 4K-aliasing catastrophe.** The first
   `ReverseIndex` kept `keys: Box<[BlockStateId]>` and `vals: Box<[u8]>` separately.
   Random `set` on an 8-bit section (packed data is exactly 4 KiB) measured
   **~267 ns/op** — reproducible, isolated, and absent at 7 bit (3648 B data).
   Diagnosis: sequential `set` was 4.5 ns, constant-value random `set` 11.8 ns,
   `reverse.get` alone 1.9 ns; only random-index + random-value combined stalled.
   A probe-count simulation showed the hash is perfect (1 probe), ruling out
   clustering. Fix: interleave key+index into one `Box<[u32]>` (one load per probe,
   small contiguous table). 8-bit `set` dropped from ~267 ns to ~13 ns.
3. **Empty-slot sentinel collided with valid data.** Packing the index into the
   high byte with `0xFF` as empty made palette index 255 (and key 0 at index 255)
   indistinguishable from an empty slot. The 10 000-case proptest did not catch it
   (exactly-256-distinct palettes are rare with random values), so it was found by
   inspection. Fixed with a dedicated occupancy bit (bit 16); added the
   `reverse_index_handles_index_255_and_key_zero` regression test.
4. **Clippy.** `box_collection` on the boxed map (resolved by the custom index) and
   several `needless_range_loop` in tests (rewritten as iterators).

## Issues encountered

- **Residual 4K-aliasing on this VM (follow-up: bare-metal numbers).** After the
  interleaving fix, `set` is ≈ 6 ns for most widths but certain widths still read
  13–15 ns. The affected widths *shift* when the benchmark's allocation layout
  changes (5/6/8-bit with auxiliary vectors, 6/8-bit with inline generation), which
  identifies address-dependent 4K-aliasing between the data stores and index loads
  on this VM rather than an algorithmic cost (the hash is O(1), 1 probe). The
  ≤ 10 ns target is met for most widths on the VM; the aliasing-affected readings
  are within the documented VM noise and need confirmation on bare metal with a
  fixed frequency (per `docs/process/testing-and-benchmarks.md`). Recorded as
  "Measured (VM …)" in `performance-budgets.md`, not as a confirmed desktop number.
  **Bare-metal follow-up (T0-16, 2026-11-21):** on the owner's laptop `set` reads
  12.7–29.4 ns across widths — the ≤ 10 ns target is missed at *every* width, not
  only the aliasing-affected ones, though the relative 6–8-bit elevation persists.
  The budget row now reads "Measured deviation" (target kept); see the
  [T0-16 summary](../docs/research/bare-metal-2026-11-21/summary.md).
- **`Direct` size differs from the research prototype.** The spec enum mandates a
  flat `Box<[BlockStateId; 4096]>` (8192 B data, 8248 B with the struct), while the
  research prototype's 6616 B was a 12-bit packed direct. Implemented the flat
  variant per the enum and updated the world-storage size table to match (the
  acceptance ±10% is against the flat 8192 B figure). No ADR change: ADR-0004 does
  not fix the direct encoding.
- **Opacity is registry data.** The spec lists an `all_opaque` flag "updated on
  set", but opacity comes from mods/datapacks (`opaque`), which the storage layer
  cannot know. Resolved by maintaining the intrinsic `non_air`/`all_air` on `set`
  and making `all_opaque` take the caller's predicate; documented in world-storage.
  The mesher (T0-04) supplies the registry predicate and the true opaque/transparent
  masks.