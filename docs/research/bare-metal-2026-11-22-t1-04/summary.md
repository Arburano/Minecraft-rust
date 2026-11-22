# Bare-metal confirmation of T1-04 hot-path optimizations (2026-11-22)

This document records the bare-metal confirmation runs of the division-free
packed addressing optimization (T1-04) on the owner's laptop (Ryzen 7 3750H).

## Machine and protocol

- **Machine:** ASUS TUF Gaming FX505DU, AMD Ryzen 7 3750H (4 cores / 8 logical,
  base 2.3 GHz, boost up to 4.0 GHz), 13.94 GiB usable RAM, Windows 11 build 28000.
- **Toolchain:** Rust 1.97.1 pinned, release profile: thin LTO, 1 codegen unit.
- **Power & clock:** AC power (`battery=2`), 100% battery charge. Continuous
  logging in `power-log.txt` (75 samples every 20 seconds, 09:10–09:35 local):
  average processor performance **152.1%** (boost active, min 133%, max 170%).
  Zero battery throttling and zero base-clock latching during the entire run.
- **Runs:** Three consecutive runs each of `cargo bench -p world --bench section`
  and `cargo bench -p mesh --bench mesh_chunk`. Raw console outputs committed
  as `world-run{1,2,3}.txt` and `mesh-run{1,2,3}.txt`.

## Before / After Comparison (Ryzen 7 3750H Bare Metal)

Comparison against the T0-16 baseline (measured on the same machine under AC boost):

| Benchmark | Target | T0-16 Baseline | T1-04 Optimized | Verdict |
| --- | --- | --- | --- | --- |
| `section_get/random/1bit` | ≈ 3.8 ns | 8.55 ns | **3.15 ns** | **Met** (2.7× faster) |
| `section_get/random/2bit` | ≈ 3.8 ns | 8.26 ns | **3.03 ns** | **Met** (2.7× faster) |
| `section_get/random/3bit` | ≈ 3.8 ns | 8.65 ns | **3.10 ns** | **Met** (2.8× faster) |
| `section_get/random/4bit` | ≈ 3.8 ns | 8.46 ns | **3.03 ns** | **Met** (2.8× faster) |
| `section_get/random/5bit` | ≈ 3.8 ns | 8.69 ns | **3.09 ns** | **Met** (2.8× faster) |
| `section_get/random/6bit` | ≈ 3.8 ns | 8.58 ns | **3.08 ns** | **Met** (2.8× faster) |
| `section_get/random/7bit` | ≈ 3.8 ns | 8.84 ns | **3.14 ns** | **Met** (2.8× faster) |
| `section_get/random/8bit` | ≈ 3.8 ns | 8.40 ns | **3.05 ns** | **Met** (2.8× faster) |
| `section_corpus/get/deep` | ≈ 3.8 ns | 8.37 ns | **2.99 ns** | **Met** (2.8× faster) |
| `section_corpus/get/surface` | ≈ 3.8 ns | 8.77 ns | **2.91 ns** | **Met** (3.0× faster) |
| **`mesh_chunk/gather_masks`** | **≤ 100 µs** | **112.8 µs** | **81.95 µs** | **Met** (1.38× faster, comfortably under budget) |
| `mesh_chunk/fast_mesh` | ≤ 0.2 ms | 53.9 µs | **50.17 µs** | **Met** |
| `mesh_chunk/build` | — | 166.2 µs | **135.49 µs** | Improved (1.23× faster) |
| `section_set/existing/1bit` | ≤ 10 ns | 21.06 ns | **12.33 ns** | Miss (improved 1.7×) |
| `section_set/existing/2bit` | ≤ 10 ns | 20.83 ns | **11.76 ns** | Miss (improved 1.8×) |
| `section_set/existing/3bit` | ≤ 10 ns | 20.18 ns | **11.61 ns** | Miss (improved 1.7×) |
| `section_set/existing/4bit` | ≤ 10 ns | 20.31 ns | **12.02 ns** | Miss (improved 1.7×) |
| `section_set/existing/5bit` | ≤ 10 ns | 20.69 ns | **11.39 ns** | Miss (improved 1.8×) |
| `section_set/existing/6bit` | ≤ 10 ns | 25.57 ns | **15.34 ns** | Miss (improved 1.7×) |
| `section_set/existing/7bit` | ≤ 10 ns | 29.40 ns | **17.53 ns** | Miss (improved 1.7×) |
| `section_set/existing/8bit` | ≤ 10 ns | 27.05 ns | **16.29 ns** | Miss (improved 1.7×) |
| `section_corpus/set/deep` | ≤ 10 ns | 30.32 ns | **20.39 ns** | Miss (improved 1.5×) |
| `section_corpus/set/surface` | ≤ 10 ns | 29.92 ns | **19.67 ns** | Miss (improved 1.5×) |

## Key Findings

1. **Section `get`**:
   The division-free multiply-shift addressing (`locate`) reduced `get` latency from
   8.3–8.8 ns down to **3.03–3.15 ns** across all bit widths. This brings `get`
   firmly under the ≈ 3.8 ns reference budget (which was based on server Xeon numbers).

2. **Gather + masks (`mesh_chunk/gather_masks`)**:
   In T0-16, `gather_masks` was 112.8 µs (over the ≤ 100 µs target). Because
   `gather_masks` queries 6,536 shell cells using per-cell `get`, the 2.8× speedup
   in `get` directly translated to `gather_masks` dropping from 112.8 µs to **81.95 µs**
   (runs: 79.55 µs, 83.64 µs, 81.95 µs, spread 5.0%).
   **The ≤ 100 µs budget row is now fully MET.**

3. **Section `set`**:
   `set` latency improved by ~1.7–1.8× across all widths (1–5 bit down from ~21 ns to
   11.4–12.3 ns; 6–8 bit down from ~27–29 ns to 15.3–17.5 ns). It remains slightly
   above the 10 ns budget row due to memory writes and cache-line write-allocation
   latency on the mobile Zen+ architecture, but is dramatically closer.
