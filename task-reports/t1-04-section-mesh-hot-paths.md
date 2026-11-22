# T1-04: optimize section get/set and gather + masks

Status: done (2026-11-22: optimization complete, laptop bare-metal confirmation complete).

## What was done

**Analysis (before changing code).**
- Every paletted `get`/`set` computed `per = 64 / bits` and then `i / per` and `i % per`: two integer divisions by run-time values on each access.
- Hardware division latency differs strongly between CPU families. That matched the T0-16 picture: bare-metal Zen+ `get` 8.3–8.8 ns against VM Xeon 3.4 ns, and `set` 12.7–29.4 ns.
- Gather + masks reads the 6,536-cell padding shell with per-cell `get` (T0-04), so it inherited the same cost, resulting in 112.8 µs on bare metal.

**Change (`crates/world/src/section.rs`).**
- `locate(bits, i)` returns the word and shift from a `PER_WORD` table (`64 / bits`) and a multiply-shift reciprocal: `(i * ceil(2^20 / per)) >> 20`. The product stays below 2^32 for `i < 4096`.
- `get_raw` and `set_raw` use it.
- A new unit test checks `locate` against `/` and `%` for every width 1..=8 and every index 0..4096 (exhaustive).
- The section proptests (10,000-case reference comparison and others) and the world/mesh tests pass unchanged.

**Bare-Metal Confirmation on Owner's Laptop (Ryzen 7 3750H, 2026-11-22).**
Three runs under confirmed AC power with continuous power logging (`power-log.txt`, 75 samples, average boost 152.1%):

| Bench | Target | T0-16 Baseline (Bare Metal) | T1-04 (Bare Metal) | Verdict |
| --- | --- | --- | --- | --- |
| `section_get` paletted 1–8 bit | ≈ 3.8 ns | 8.3–8.8 ns | **3.03–3.15 ns** | **Met** (2.8× speedup, under target) |
| `section_corpus/get` deep / surface | ≈ 3.8 ns | 8.37 / 8.77 ns | **2.99 / 2.91 ns** | **Met** |
| `mesh_chunk/gather_masks` | ≤ 100 µs p50 | 112.8 µs | **81.95 µs** | **Met** (1.38× speedup, under budget) |
| `mesh_chunk/fast_mesh` | ≤ 0.2 ms p50 | 53.9 µs | **50.17 µs** | **Met** |
| `section_set` existing 1–5 bit | ≤ 10 ns | 20.2–21.1 ns | **11.39–12.33 ns** | Deviation (1.8× speedup, near target) |
| `section_set` existing 6–8 bit | ≤ 10 ns | 25.6–29.4 ns | **15.34–17.53 ns** | Deviation (1.7× speedup) |
| `section_corpus/set` surface | ≤ 10 ns | 29.92 ns | **19.67 ns** | Deviation (1.5× speedup) |

Full report and raw logs committed in [`docs/research/bare-metal-2026-11-22-t1-04/summary.md`](../docs/research/bare-metal-2026-11-22-t1-04/summary.md).

## What went wrong

- The first version multiplied with a 24-bit shift and overflowed `u32` for 8-bit sections; it was caught immediately by the debug-mode tests. The shift is now 20, which is still exact for every index (exhaustive test).
- Direct `get` in the microbenchmark is 1.14 ns on bare metal (vs 0.9 ns pre-optimization), but direct sections are rare and 1.14 ns is far below the ≈ 3.8 ns reference target.

## Issues encountered

- **`section_set` remains slightly above the ≤ 10 ns budget** on Ryzen 7 3750H (11.4–17.5 ns), but is improved by ~1.7–1.8×. The remaining gap is memory write-allocation latency on mobile Zen+.
- `performance-budgets.md` updated: `get` and `gather_masks` moved from "Measured deviation" to "Measured" (targets met). `set` remains as "Measured deviation" with updated numbers and the ≤ 10 ns target kept.
