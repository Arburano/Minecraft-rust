# Empirical tests of the optimization claims (CPU-only Linux VM)

Bench workspace: `/home/claude/bench` (members: `meshing`, `palette`, `wasmbench`, `light`, `zstdbench`). Raw outputs: `/home/claude/bench/results/*.txt`.
Run with `cargo build --release && ./target/release/<member>`. Timing uses `std::time::Instant` loops with `black_box` (no criterion), release profile `opt-level=3, lto=thin`, no `target-cpu=native`.

## Machine
- Intel Xeon (Sapphire Rapids family, model 207) @ 2.10 GHz, **KVM guest, only 2 vCPUs** (1 thread/core), 7 GiB RAM, L2 4 MiB/2 instances. AVX2/AVX-512 present but not targeted by the build.
- rustc/cargo 1.97.0, already installed (no rustup needed).
- **Noise warning:** this is a shared VM. Repeated runs of the same benchmark differed by 20-70% (e.g. wasm empty call 17-30 ns, 1000 individual calls 18-35 us). Treat every number as +-30%, and order-of-magnitude conclusions only. "Scaling to N cores" was only testable up to 2 real cores (4 and 8 threads are oversubscribed).
- Crate versions (from Cargo.lock): binary-greedy-meshing **0.6.4** (research file said 0.3.6; 0.6.4 is the latest and has `fast_mesh`/`mesh_lod`), rayon 1.12.0, noise 0.9.0, wasmtime **50.0.1** (51.0.0-rc.1 exists only as prerelease), wat 1.263.0, anyhow 1.0.104, zstd 0.14.4 (zstd-sys 2.1.3 = zstd 1.5.7).

## Verdict vs targets (summary)
| Target | Result | Verdict |
|---|---|---|
| mesh <= 0.2 ms/chunk | `mesh()` (BTreeSet) median 1.1 ms; `fast_mesh()` (masks given) median 0.27 ms, p95 0.46 ms, on this 2.1 GHz VM | **Not met here**; `fast_mesh` is within ~1.4x of target on a slow VM core. Not demonstrable either way on a desktop CPU. `mesh()` is 4x slower, so maintain opaque/transparent masks per chunk. |
| mod hook <= 1 us/call | Core wasm host->guest call 17-35 ns (typed func); wasmtime **component** call ~0.40 us | **Met** for both (core ~30x, component ~2.5x margin), if calls are not made 1000s of times per hook; batch for bulk data |
| client RAM <= 2 GB at 12 chunks | Voxel data 15000 sections: ~17 MiB palette-compressed (est. mix), 117 MiB plain, 95 MiB worst case; meshes a few MiB | **Chunk data is nowhere near the limit** (see caveats; textures, GPU mirror, entities, engine not measured) |
| server tick <= 25 ms @ 500 players | Not testable (no server). Only building-block costs measured | **Not tested** |

## 1. Chunk meshing (binary-greedy-meshing 0.6.4, 62^3 padded to 64^3)
Method: 256 chunks (16x16 grid, world y 61..123, so each chunk straddles the surface), generated from 4-octave Perlin heightmap (base 80, amp 38) + 3D-Perlin caves (threshold 0.55) + water below y=78 + sparse leaf blobs. Block ids: air/stone/dirt/grass/sand + transparent {5 water, 6 leaves, 7 glass}.
**Composition deviates from the 5% brief**: 59.4% air, 32.5% opaque, **8.1% transparent of total volume (20% of non-air)**, i.e. a harder case than 5%. Time = `mesher.clear()` + mesh call, 5 timed passes over 256 chunks (1280 samples), single thread. Naive mesher = per-voxel, 6 faces, emits face if neighbour is air or a different transparent block, no merging, lookup-table transparency test, 1 u64 per face.

| Mesher | median | p95 | quads/chunk | bytes @8 B/quad |
|---|---|---|---|---|
| `Mesher::mesh` (BTreeSet transparents) | 1083-1178 us | 1540-1990 us | 4962 | 39 KB |
| `Mesher::fast_mesh` (precomputed masks) | 269-278 us | 457-556 us | 4962 | 39 KB |
| naive per-face cull | 898-1373 us (BTreeSet version 1373, table version 898) | 1425-2238 us | 11762 | 92 KB |
(two runs shown; the second run used the table-lookup naive mesher and is the fair comparison.)

- Building the opaque/transparent masks from scratch costs median 341-366 us (p95 ~550-636 us), more than `fast_mesh` itself; `fast_mesh` only pays off if the masks are maintained incrementally on block edits (not tested).
- Empty chunk: `mesh` 205 us, `fast_mesh` 52 us. All-stone padded chunk: `mesh` 1.4 ms (!), `fast_mesh` 52 us; skip meshing fully-solid / empty chunks cheaply.
- Greedy merging cuts quads by 2.4x vs naive (4962 vs 11762), but `mesh()` (BTreeSet lookups) was NOT faster than my table-based naive mesher (1.1-1.2 ms vs 0.9 ms); only `fast_mesh` wins, ~3.2x faster than naive.
- rayon over 256 chunks (per-worker Mesher via `map_init`, best of 3, 20 rounds):

| mode | 1 thread | 2 threads | 4 (oversub.) | 8 (oversub.) |
|---|---|---|---|---|
| mesh | 916 chunks/s | 1813 (1.98x) | 1814 | 1761 |
| fast_mesh | 3133 chunks/s | 5768 (1.84x) | 6243 | 5943 |
Scaling is linear to the 2 physical cores; no data above 2 cores. Wall time per chunk at 2 threads: 552 us (`mesh`), 173 us (`fast_mesh`).
- Research claim 50-200 us (Ryzen 3800X) and ~600 us with transparency (Xeon E5-1650 v3) is **plausible but not reproduced**: our VM core is slower than a 3800X, and with 8% transparent voxels `mesh()` took ~1.1 ms vs the README's ~0.6 ms figure.
- Not tested: AO, vertex pulling, GPU upload.

## 2. Palette-compressed 16^3 section (`palette/`)
Implementation: `enum {Single(u16), Pal{bits 1..8, Vec<u16> palette, Vec<u64> data}, Direct 12-bit}`; entries never straddle a u64 word (like Minecraft 1.16+; 64/bits entries per word). Growth repacks (1->2->...->8 bit -> direct 12-bit). Round-trip verified for all cases. Byte counts include the 56-byte enum stack size and the palette, excluding allocator overhead.

| Section | distinct | bits | bytes | % of plain 8192 B |
|---|---|---|---|---|
| air | 1 | single | 56 | 0.7% |
| stone | 1 | single | 56 | 0.7% |
| surface mixed (synthetic, 8 ids) | 8 | 3 | 1640 | 20% |
| underground + caves + ores (10 ids) | 10 | 4 | 2124 | 26% |
| 256 distinct | 256 | 8 | 4664 | 57% |
| worst: 4096 distinct | 4096 | 12 direct | 6616 | 81% |
Random-palette sizes for 2/4/8/16/32/64/128/256 entries: 572/1088/1640/2136/2856/3464/3960/4664 B.
Note: non-straddling packing wastes bits for 3, 5, 6, 7, 9-bit widths (e.g. 12-bit direct is 5 entries/64-bit word = 6560 B data, not 6144).

Get/set, ns per op, median/p95 of 51 runs x 4096 ops (random index order unless noted):
| | get | set (value already in palette) |
|---|---|---|
| plain `[u16;4096]` | 0.99 / 1.24 (seq 0.53) | 0.74 / 0.96 |
| Single | 0.84 / 1.43 | 2.13 / 2.79 |
| 3-bit surface mixed | 3.75 / 4.84 (seq 3.91) | 6.94 / 8.24 |
| 4-bit underground | 3.79 / 4.43 | 6.37 / 8.27 |
| 8-bit (256 palette) | 3.68 / 4.29 | **66.7 / 93.3** (linear palette scan, O(256); needs a hash/reverse index) |
| 12-bit direct | 2.81 / 3.72 | 5.14 / 5.81 |
- Palette get is ~3.8x slower than plain (3.8 vs 1.0 ns) but still ~4 ns; set 6-7 ns with a 16-entry palette.
- Growth from Single to 300 distinct values inserted one by one: 288 us total (~960 ns/set amortised, including every repack).
- Client RAM estimate (25x25 columns x 24 sections = 15000 sections): plain u16 everywhere = **117.2 MiB**; assumed per-column mix (8 air, 1.5 surface, 12 underground, 2.5 pure-stone sections; this mix is my assumption, not measured) = **17.0 MiB**; worst case all-12-bit = 94.6 MiB; all 8-bit = 66.7 MiB.
- Add-ons by arithmetic, not benchmarked: light 4 KB/section uncompressed (nibbles block+sky) = 58.6 MiB for 15000 sections; CPU-side mesh output: 39 KB per surface chunk ~= 1.7 MB for the 25x25 surface layer, even 5x for caves/overhangs stays < 10 MiB. So **chunk data is ~25-250 MiB vs 2 GB budget**; the remaining budget must cover textures, GPU staging, entities, UI, allocator slack (none of these measured).

## 3. Wasmtime host<->guest overhead (`wasmbench/`, wasmtime 50.0.1)
Guest: hand-written WAT core module (cranelift JIT, default `Config` + epoch/fuel toggles). Typed funcs (`get_typed_func`). Median/p95 of 21 reps x 100k calls.
- **Core module empty call**: 16.5-30 ns (two runs: 29.4/21.7/24.1 baseline; epoch 16.5-30; fuel 16.6-30; epoch+fuel 23.6-29). `add(i32,i32)->i32` 22-33 ns. Ordering/warm-up and VM noise exceed the epoch/fuel differences on per-call cost; **no stable per-call penalty from epoch or fuel was measurable** above ~5-10 ns. Per-call `set_fuel` + `set_epoch_deadline` added < ~3 ns in the cleaner runs.
- **Target <= 1 us per hook call: met** (~30x margin) for core modules.
- **1000 block updates as 1000 separate calls** (4 i32 args each, guest writes u16): 18.2-35 us per 1000 = **18-35 ns per update** (run-to-run noise).
- **1000 updates as one batched call** (host `memory.write` 16 KB + 1 call): 0.92-2.9 us per 1000 = **0.9-2.9 ns per update**, i.e. ~10-20x cheaper than individual calls. (Batch buffer = 4 x i32 per update.)
- Guest->host import call (guest loops calling a trivial host fn): 3.7-4.9 ns per call.
- Instantiate: new Store + `InstancePre::instantiate` 4.2 us median (tiny module); compile of the tiny module 3.5-4.9 ms.
- Epoch interruption verified functional: ticker thread incrementing epoch after 20 ms interrupted `spin(i32::MAX)` after 20.6 ms with a trap.

Compute-heavy guest (median of 7 reps; relative to baseline):
| guest | baseline | epoch | fuel | epoch+fuel |
|---|---|---|---|---|
| tight loop, 300M iterations (run1 / run2) | 570 / 596 ms | -0.2% / +2.8% | +0.9% / -1.7% | +2.5% / +0.6% |
| call-heavy `fib(35)` (run1 / run2) | 86.7 / 72.1 ms | 0.0% / +9.3% | **+52.9% / +92.4%** | +52.7% / +76.3% |
- Epoch: 0-9% (docs claim ~10%): **confirmed as cheap**, in the noise for a tight loop. Fuel: ~0-2% on a tight loop but **+50-90% on a call-heavy guest** (fuel accounting at function entries/instruction blocks): the research claim that fuel costs more than epoch is **confirmed**; there was no number before, now ~1.5-1.9x on recursion. (Tight loop has few counted points; real mod code lies in between.)

**Component Model**: tooling note: `cargo-component`/`wasm-tools` were **not installed or tried**; instead I hand-wrote a component in WAT (core module + `canon lift`, `list<u32>` with `memory`/`realloc` options), compiled via the `wat` crate and ran it through `wasmtime::component::{Component, Linker}`. The first attempt failed on legacy WAT syntax (`(memory $i "memory")`); fixed with `(memory (core memory $i "memory"))`. A Rust-guest toolchain (wasm32 target + wit-bindgen) was **not tested**, so lifted-type overhead for strings/records is unmeasured.
| Component (WAT guest) | empty call | 1000 individual calls (4 x u32) | 1 call, `list<u32>` of 4000 (16 KB, clone + lower) |
|---|---|---|---|
| baseline | 398 ns | 523 us (523 ns/update) | 3.6 us (3.6 ns/update) |
| epoch | 450 ns (run noise) | 503 us | 2.1 us |
| epoch+fuel | 402 ns | 423 us | 3.8 us |
- A component call costs ~0.40 us, i.e. **~15-20x a core call** (canon ABI/may_leave/etc. machinery), still **under the 1 us target** but with only 2.5x margin on this slow VM. 1000 per-update calls cost ~0.5 ms vs ~3 us batched (~150x): **batching is mandatory** at the component boundary; with core modules batching is a 10-20x gain.
- Per-hook budget arithmetic (estimate only): 500 players x 10 hook calls/tick = 5000 calls = ~0.15 ms (core) or ~2 ms (component) of the 25 ms tick.

## 4. Light flood-fill BFS (`light/`)
Method: scalar BFS over `Vec<u32>` queue with per-cell bounds checks, `u8` light levels 0-15 (torch 14), full relight from scratch (clear + seed + BFS, no cross-section boundary handling). Median/p95 of 201-301 runs. Grid 16^3 (1 section) and 48^3 (3x3x3 neighbourhood, 110,592 cells, all 27 sections relit at once).
| scenario | 16^3 (4096 cells) | 48^3 (3x3x3) |
|---|---|---|
| open air, 1 torch | 24 us / p95 35 us (2610 pops) | 36 us / p95 60 us (3303 pops) |
| open air, 8 torches per section | 46 us / 69 us (3987 pops) | **1415 us / 1600 us** (109,490 pops, 12.9 ns/pop) |
| ~25% opaque, 8 torches/section | 27 us / 50 us | 2181 us / 2361 us (81,000 pops, 26.9 ns/pop) |
| ~43-45% opaque, 8 torches/section | 25 us / 62 us | 2062 us / 2654 us (58,714 pops, 35 ns/pop) |
| skylight (heightmap terrain + roof patches) | 14 us (degenerate: only 23 pops, grid is mostly solid) | 554 us / 606 us (19,945 pops) |
- Cost is ~10-13 ns per queue pop in open air and 27-35 ns per pop with obstacles (branch mispredictions). One section ~25-45 us; 3x3x3 ~1.4-2.2 ms. At ~2 ms per 27-section relight, a server doing 1000 block-light updates per tick on single sections (~40 us) uses ~40 ms of a single thread, so **incremental (add/remove) light updates, not full relights, are required**; incremental was not benchmarked.
- Skylight numbers for 16^3 are not meaningful (test grid mostly solid); trust the 48^3 row only.

## 5. zstd (`zstdbench/`, zstd 0.14.4 / zstd 1.5.7)
Method: 400 synthetic columns 16x16x384 (24 sections) from Perlin terrain (strata, caves, 3 ore types, water). Two serializations: raw u16 LE (196,608 B) and palette-packed per section (avg 20,496 B). Dict trained with `zstd::dict::from_samples` on 200 columns, evaluated on 100 held-out columns; level = 1/3/9; compress/decompress throughput per input bytes, median of 5 passes.
**Caveat: these synthetic columns are far more compressible than real worlds** (smooth strata, sparse noise features, no light data, no block entities, no player builds). The ratios below (140x raw!) are **not** realistic expectations; use only the relative effects (levels, dictionary, speed).
| format | level | ratio | bytes/col | compress | decompress |
|---|---|---|---|---|---|
| raw u16, no dict | 1 / 3 / 9 | 140x / 144x / 184x | 1404 / 1361 / 1066 | 2663 / 2693 / 607 MB/s (74 / 73 / 324 us/col) | 6.5-9.3 GB/s (21-30 us/col) |
| raw u16, 16 KB dict | 1 / 3 / 9 | 100x / 100x / 184x | 1967 / 1975 / 1070 | 2468 / 2462 / 201 MB/s | 6.4-6.8 GB/s |
| palette-packed, no dict | 1 / 3 / 9 | 18.9x / 19.1x / 21.5x | 1091 / 1081 / 957 | 860 / 802 / 125 MB/s (24 / 26 / 165 us/col) | 1.6-1.9 GB/s (11-12 us/col) |
| palette-packed, 16 KB dict | 1 / 3 / 9 | 17.9x / 18.7x / 22.2x | 1149 / 1104 / 930 | 875 / 601 / 100 MB/s | 1.6-1.9 GB/s |
| single raw 16^3 section (8 KB), no dict | 1 / 3 / 9 | 70.7x / 72x / 90.7x | 116 / 114 / 90 | 1910 / 1816 / 189 MB/s (4.3 / 4.5 / 43 us) | 3.4-4.2 GB/s |
| single section, 16 KB dict | 1 / 3 / 9 | 74.6x / 76.5x / 91.4x | 110 / 107 / 90 | 2140 / 2071 / 144 MB/s | 3.4-4.5 GB/s |
- Level 9 gives ~13-30% smaller output than level 3 at 4-6x compress time (6.7x on raw columns), decompression is equally fast. Level 1 and 3 are within 2% of each other in ratio here.
- Dictionary: **helped only on tiny payloads (single 8 KB section: +3-5% ratio)**; on whole columns it was neutral to **harmful** (raw columns: 144x -> 100x at level 3; palette-packed +/-3%). The 110 KB dictionary request produced only a 1.5 KB (raw) or 33 KB (packed) dictionary, so dictionary training with a synthetic corpus was not very informative. Not validated on real worlds.
- A palette-packed column compresses to ~1 KB, i.e. 625 columns ~ 0.7 MB for the whole 12-radius view, unrealistically small because of the synthetic data.

## What was NOT tested / failed
- No GPU: no wgpu, vertex pulling, draw-call, Hi-Z or storage-buffer-limit tests.
- Server tick @ 500 players, quinn networking, bevy_ecs, region threading: not tested.
- Real-terrain data: all terrain is synthetic Perlin, transparent share 8.1% rather than 5%.
- Multi-core scaling above 2 cores (VM has 2 vCPUs).
- Component Model via Rust guest + `cargo-component` / `wit-bindgen`: not attempted; only a hand-written WAT component measured. The WAT component fails on legacy syntax until fixed (documented above).
- criterion was not used; no statistical outlier rejection beyond median/p95.
- Incremental light updates, mask maintenance for `fast_mesh`, palette hash index: not implemented.
