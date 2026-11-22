# T1-04: optimize section get/set and gather + masks

Status: cloud part done (analysis, optimization, VM measurements). The acceptance needs the confirmation run on the owner's laptop (Ryzen 7 3750H), where the misses were measured.

## What was done

**Analysis (before changing code).**
- Every paletted `get`/`set` computed `per = 64 / bits` and then `i / per` and `i % per`: two integer divisions by run-time values on each access.
- Hardware division latency differs strongly between CPU families. That matches the T0-16 picture: bare-metal Zen+ `get` 8.3–8.8 ns against VM Xeon 3.4 ns, and `set` 12.7–29.4 ns.
- Gather + masks reads the 6,536-cell padding shell with per-cell `get` (T0-04), so it inherits the same cost. That is consistent with gather + masks being 112.8 µs on bare metal against 89 µs on the VM.

**Change (`crates/world/src/section.rs`).**
- `locate(bits, i)` returns the word and shift from a `PER_WORD` table (`64 / bits`) and a multiply-shift reciprocal: `(i * ceil(2^20 / per)) >> 20`. The product stays below 2^32 for `i < 4096`.
- `get_raw` and `set_raw` use it.
- A new unit test checks `locate` against `/` and `%` for every width 1..=8 and every index 0..4096 (exhaustive).
- The section proptests (10,000-case reference comparison and others) and the world/mesh tests pass unchanged.

**Before/after** — cloud **VM**, 4 vCPU Xeon 2.1 GHz. Criterion `--baseline before`, 4,096 random indices per iteration; the "after" ranges are 2 runs × 30 samples.

| Bench | Before (per op) | After (per op) |
| --- | --- | --- |
| `section_get` paletted, 1–8 bit | 4.3–4.6 ns | 2.5–3.3 ns |
| `section_get` single | 0.7 ns | 0.8 ns (no change) |
| `section_get` direct | 0.9 ns | **2.1 ns (regression)** |
| `section_set` existing, 1–5 bit | 7.8–8.3 ns | 5.4–8.1 ns |
| `section_set` existing, 6–8 bit | 15.5–19.8 ns | 7.0–8.3 ns |
| `section_corpus/get` deep / surface | 4.5 / 4.4 ns | 2.6–2.9 / 2.7 ns |
| `section_corpus/set` surface | 15.4 ns | 8.0–9.7 ns |
| `mesh_chunk/gather_masks` | 95.7 µs | 92–106 µs (no stable change on the VM) |

## What went wrong

- The first version multiplied with a 24-bit shift and overflowed `u32` for 8-bit sections; it was caught immediately by the debug-mode tests. The shift is now 20, which is still exact for every index (exhaustive test).
- **`get` on direct sections** (more than 256 distinct states) got slower in the microbenchmark: 0.9 → 2.1 ns, reproduced 4 times. Re-measuring the old code reproduced the baseline (3.7 µs per 4,096). These did not help: `#[inline]` instead of `#[inline(always)]` on `locate`, `#[inline(always)]` on `get`, reordering the match arms.
  - Most likely the larger paletted path changed how the compiler optimizes the benchmark loop (unswitching or vectorizing the direct arm), not the direct access itself.
  - Kept: direct sections are rare, 2.1 ns is still far under the ≈ 3.8 ns `get` reference, and the corpus `get`/`set` benches all improved.

## Issues encountered

- **Laptop confirmation pending.** The expected gain is larger on Zen+ than on this VM, especially for gather + masks through the shell `get` path. For the owner's local agent: run `cargo bench -p world --bench section` and `cargo bench -p mesh --bench mesh_chunk` 3 times as in T0-16, compare with the T0-16 summary, and update the `get`, `set` and gather + masks rows.
- If gather + masks is still over 100 µs on the laptop, the next step is to read the shell faces with word-level unpacking of the needed rows instead of per-cell `get` (T0-04 chose per-cell `get` for the shell because whole-section unpacking was 3× slower).
- **Budget rows** are left as "Measured deviation" until the laptop numbers exist (AGENTS.md rule 7: no VM number replaces a bare-metal row).
