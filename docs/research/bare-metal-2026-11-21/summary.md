# Bare-metal confirmation of VM-only benchmarks (T0-16, 2026-11-21)

Stage 0 exit criterion: the "Measured (VM …)" rows with ±30% noise must be
confirmed on real hardware (`docs/process/testing-and-benchmarks.md`).
This document records the bare-metal runs of `cargo bench -p world`,
`cargo bench -p mesh` and `cargo bench -p light`, three runs each.

## Machine and protocol

- ASUS TUF Gaming FX505DU, AMD Ryzen 7 3750H (4 cores / 8 logical, base
  2.3 GHz, boost up to 4.0 GHz), 13.94 GiB usable RAM, Windows 11 build
  28000. Physical machine, not a VM.
- Rust 1.97.1 (pinned toolchain), release profile: thin LTO, one codegen
  unit, line-tables-only debug info.
- AC power, "High performance" power scheme with the "Best performance"
  overlay (both verified before the runs). The owner closed other programs
  and did not use the laptop during the runs.
- Three consecutive runs per crate, 2026-11-21 19:40–20:09 local, in the
  order world → mesh → light per run. Criterion defaults: 100 samples, 3 s
  warm-up, 5 s measurement. Raw console output of every run is committed
  next to this file (`world-run1.txt` … `light-run3.txt`).
- Continuous power/frequency logging: `power-log.txt` recorded 86 samples
  every 20 seconds during the entire 30-minute run. All 86 samples confirmed
  AC power (`battery=2`), 100% battery charge, and active CPU boost
  (processor performance 131–158%, minimum 131%).
- Corpus: `worldgen::test_corpus`, seed 0 (T0-12). Mesh corpus composition
  printed by every run: air 28.8%, opaque 65.5%, transparent 5.7%;
  599 quads/chunk; render batch 5 410 quads / 42.3 KB. Light corpus:
  13 snapshots, 832 air / 832 surface edit cells, 2 496 edits per kind per
  run. Per-op values = criterion group estimate / 4096.
- Tables below give the criterion point estimate of each run, the median of
  the three runs, and the spread (max − min) / median.

### Attempt 1 demotion and power state context

An initial set of runs taken earlier that day (15:21–15:51) was demoted to
`attempt-1-power-uncertain/` because the machine was suspected of having run
on battery part of the time and latched at base clock (2300 MHz, no boost,
≈ 1.5× slower) afterwards (`spot-check-base-clock.txt`).
To eliminate all doubt, the entire 9-run suite was re-executed under confirmed
AC power with continuous frequency logging. The numbers reported below are
from this clean, verified rerun (attempt 2).

## world (`crates/world/benches/section.rs`)

### Synthetic width sweep (criterion estimates, µs per 4096-op group)

| Benchmark | Run 1 | Run 2 | Run 3 | Median | Spread | Per-op (median) | VM per-op |
| --- | --- | --- | --- | --- | --- | --- | --- |
| section_get/random/single | 3.924 | 3.763 | 3.938 | 3.924 | 4.4% | 0.96 ns | 0.5 ns |
| section_get/random/1bit | 35.390 | 35.006 | 35.012 | 35.012 | 1.1% | 8.55 ns | ≈ 3.4 ns |
| section_get/random/2bit | 34.066 | 33.825 | 33.565 | 33.825 | 1.5% | 8.26 ns | ≈ 3.4 ns |
| section_get/random/3bit | 35.983 | 35.448 | 34.464 | 35.448 | 4.3% | 8.65 ns | ≈ 3.4 ns |
| section_get/random/4bit | 37.211 | 34.495 | 34.665 | 34.665 | 7.8% | 8.46 ns | ≈ 3.4 ns |
| section_get/random/5bit | 35.654 | 35.592 | 34.631 | 35.592 | 2.9% | 8.69 ns | ≈ 3.4 ns |
| section_get/random/6bit | 38.806 | 35.121 | 35.141 | 35.141 | 10.5% | 8.58 ns | ≈ 3.4 ns |
| section_get/random/7bit | 38.330 | 36.213 | 34.305 | 36.213 | 11.1% | 8.84 ns | ≈ 3.4 ns |
| section_get/random/8bit | 34.400 | 35.281 | 33.522 | 34.400 | 5.1% | 8.40 ns | ≈ 3.4 ns |
| section_get/random/direct | 4.939 | 4.918 | 4.721 | 4.918 | 4.4% | 1.20 ns | 0.7 ns |
| section_set/existing/single | 53.081 | 51.809 | 50.646 | 51.809 | 4.7% | 12.65 ns | 2–3.5 ns |
| section_set/existing/1bit | 86.398 | 85.362 | 86.280 | 86.280 | 1.2% | 21.06 ns | ≈ 6 ns |
| section_set/existing/2bit | 90.476 | 85.307 | 84.885 | 85.307 | 6.6% | 20.83 ns | ≈ 6 ns |
| section_set/existing/3bit | 101.170 | 82.645 | 82.338 | 82.645 | 22.8% | 20.18 ns | ≈ 6 ns |
| section_set/existing/4bit | 83.192 | 82.087 | 86.947 | 83.192 | 5.8% | 20.31 ns | ≈ 6 ns |
| section_set/existing/5bit | 87.539 | 84.734 | 80.833 | 84.734 | 7.9% | 20.69 ns | ≈ 6 ns |
| section_set/existing/6bit | 107.310 | 104.740 | 104.160 | 104.740 | 3.0% | 25.57 ns | 13–15 ns (VM aliasing) |
| section_set/existing/7bit | 124.200 | 115.690 | 120.430 | 120.430 | 7.1% | 29.40 ns | ≈ 6 ns |
| section_set/existing/8bit | 112.780 | 110.780 | 110.510 | 110.780 | 2.0% | 27.05 ns | 13–15 ns (VM aliasing) |
| section_set/existing/direct | 67.786 | 64.489 | 62.850 | 64.489 | 7.7% | 15.74 ns | 3–4 ns |
| section_unpack_into/section/single | 0.149 | 0.144 | 0.143 | 0.144 | 3.9% | 144 ns/section | < 100 ns |
| section_unpack_into/section/3bit | 2.872 | 2.740 | 2.832 | 2.832 | 4.7% | 2.83 µs/section | ≈ 2.9–4.1 µs |
| section_unpack_into/section/4bit | 2.876 | 2.943 | 2.902 | 2.902 | 2.3% | 2.90 µs/section | ≈ 2.9–4.1 µs |
| section_unpack_into/section/8bit | 3.017 | 2.892 | 2.968 | 2.968 | 4.2% | 2.97 µs/section | ≈ 2.9–4.1 µs |
| section_unpack_into/section/direct | 0.150 | 0.159 | 0.156 | 0.156 | 5.6% | 156 ns/section | < 100 ns |

### T0-12 corpus group (seed 0, column (0,0))

| Benchmark | Run 1 | Run 2 | Run 3 | Median | Spread | Per-op (median) | VM per-op (T0-12) |
| --- | --- | --- | --- | --- | --- | --- | --- |
| section_corpus/get/deep | 33.325 | 34.300 | 35.233 | 34.300 | 5.6% | 8.37 ns | 3.98 ns |
| section_corpus/get/surface | 35.917 | 34.653 | 36.297 | 35.917 | 4.6% | 8.77 ns | 3.97 ns |
| section_corpus/get/canopy | 3.738 | 3.932 | 3.957 | 3.932 | 5.6% | 0.96 ns | 0.61 ns |
| section_corpus/set/deep | 125.400 | 118.600 | 124.210 | 124.210 | 5.5% | 30.32 ns | 12.4 ns |
| section_corpus/set/surface | 123.420 | 122.540 | 120.270 | 122.540 | 2.6% | 29.92 ns | 13.0 ns |
| section_corpus/set/canopy | 51.065 | 50.184 | 49.328 | 50.184 | 3.5% | 12.25 ns | 4.9 ns |
| section_corpus/unpack/deep | 3.911 | 3.785 | 4.002 | 3.911 | 5.6% | 3.91 µs/section | 3.28 µs |
| section_corpus/unpack/surface | 2.791 | 2.870 | 2.853 | 2.853 | 2.8% | 2.85 µs/section | 3.31 µs |
| section_corpus/unpack/canopy | 0.146 | 0.143 | 0.150 | 0.146 | 5.0% | 146 ns/section | 85 ns |

(µs per 4096-op group unless the per-op column says otherwise.)

### Budget verdicts (world)

| Budget row | Target | Bare metal (median of 3) | Verdict |
| --- | --- | --- | --- |
| `get` in a section | ≈ 3.8 ns (reference) | 8.3–8.8 ns paletted; 0.96 ns single; 1.20 ns direct; corpus 8.4–8.8 ns deep/surface | **Miss** (≈ 2.2–2.3× the reference on paletted) |
| `set` in a section, any bit width | ≤ 10 ns | 12.7–29.4 ns synthetic; 12.3–30.3 ns corpus | **Miss** (every width) |
| `unpack_into` per section | ≤ 5 µs | 2.8–3.0 µs paletted; 0.14–0.16 µs single/direct; corpus ≤ 3.91 µs | **Met** |

Observations:

- The paletted `get` cost is flat across widths (8.3–8.8 ns): the random
  4096-index walk is memory-latency bound, and the mobile Zen+ part is
  ≈ 2.2–2.3× slower per random access than the server Xeon the VM numbers came
  from. This is a uniform shift, not a width-specific spike.
- `set` shows the same uniform shift (≈ 20–21 ns at 1–5 bit vs ≈ 6 ns on the
  VM) plus a relative 6–8-bit elevation (25.6–29.4 ns) that resembles the
  4K-aliasing pattern documented in T0-03 — but on bare metal even the
  unaffected widths are over the ≤ 10 ns target, so aliasing alone does not
  explain the miss.
- The T0-03 follow-up hope that bare metal would read ≤ 10 ns at all widths
  is **not** confirmed. The corpus `set` was already over target on the VM
  (12–13 ns) and is worse here (30 ns on 2–3-bit terrain sections).

## mesh (`crates/mesh/benches/mesh_chunk.rs`)

| Benchmark | Run 1 | Run 2 | Run 3 | Median | Spread | VM (T0-12) |
| --- | --- | --- | --- | --- | --- | --- |
| mesh_chunk/gather_masks | 112.830 µs | 113.110 µs | 111.950 µs | 112.830 µs | 1.0% | 88.9 µs |
| mesh_chunk/fast_mesh | 52.743 µs | 53.937 µs | 56.196 µs | 53.937 µs | 6.4% | 49.0 µs |
| mesh_chunk/build | 166.160 µs | 165.950 µs | 172.470 µs | 166.160 µs | 3.9% | 138.1 µs |
| mesh_chunk/remesh | 58.496 µs | 63.358 µs | 62.464 µs | 62.464 µs | 7.8% | 51.8 µs |
| mesh_chunk/mask_update | 4.5 ns | 4.4 ns | 4.4 ns | 4.4 ns | 1.2% | 3.71 ns |
| mesh_chunk/mask_rebuild | 48.843 µs | 49.933 µs | 49.121 µs | 49.121 µs | 2.2% | 49.5 µs |
| render_batch/edit_subrange | 163.8 ns | 176.1 ns | 166.7 ns | 166.7 ns | 7.3% | 146 ns |
| render_batch/pack_all | 1.463 µs | 1.523 µs | 1.596 µs | 1.523 µs | 8.7% | 1.11 µs |
| render_batch/edit_recopy_all | 1.668 µs | 1.709 µs | 1.649 µs | 1.668 µs | 3.6% | 1.24 µs |

### Budget verdicts (mesh)

| Budget row | Target | Bare metal (median of 3) | Verdict |
| --- | --- | --- | --- |
| Gather + masks of a 32³ mesh chunk | ≤ 100 µs p50 | 112.8 µs (runs 112.0–113.1; all three over) | **Miss** (≈ 13% over; spot-check at base clock was 170.6 µs) |
| `fast_mesh` of a 32³ mesh chunk | ≤ 0.2 ms p50 | 53.9 µs | **Met** (3.7× margin) |
| Mask update per edit | ≤ 20 ns | 4.4 ns | **Met** |
| Render batch copy after an edit | ≤ 50 µs | 1.67 µs full re-copy; 0.17 µs sub-range | **Met** (≈ 30× margin; T0-05 recommendation to keep the full re-copy stands) |

`gather_masks` unpacks eight core sections word by word plus 6 536 shell
cells with per-cell `get`; both paths are memory-latency sensitive, consistent
with the world `get`/`set` shift. Criterion reports means, not p50; the
per-iteration distribution is not captured, but all three run estimates and
the boost-state spot-check (113.3 µs) are over 100 µs, so the p50 miss is not
an artifact of the statistic.

## light (`crates/light/benches/light.rs`)

Per-edit percentiles (µs, `Instant` timing, range over the 3 runs;
2 496 edits per kind per run):

| Edit | Path | p50 | p95 | p99 | max | VM p95 (T0-08) |
| --- | --- | --- | --- | --- | --- | --- |
| add_torch | add | 0.8–1.1 | 2.2–2.8 | 3.0–4.3 | 7.8–14.9 | 2.7–3.4 |
| break_block | add | 0.2 | 0.3–0.4 | 0.6–0.9 | 1.5–7.4 | 0.3–0.4 |
| remove_torch | remove | 5.4–6.7 | 11.4–14.2 | 14.2–17.8 | 30.8–83.3 | 12.8–16.8 |
| place_block | remove | 0.8–0.9 | 3.7–4.5 | 5.4–6.4 | 10.0–52.1 | 3.8–5.2 |
| worst dark_torch | add | 16.2–21.4 | 16.3–24.7 | — | 38.3–56.7 | 18.3–24.3 |
| worst dark_torch | remove | 13.4–19.6 | 13.9–20.5 | — | 22.1–51.9 | 12.5–18.0 |
| worst cave_roof | add | 14.3–18.6 | 14.4–19.0 | — | 25.7–33.8 | 19.0–24.5 |
| worst cave_roof | remove | 12.1–16.6 | 12.8–17.2 | — | 30.4–66.2 | 11.3–15.3 |
| full_relight_3x3x3 | — | 862–1045 | — | — | 984–1702 | 910–1230 (p50) |

Criterion means (µs):

| Benchmark | Run 1 | Run 2 | Run 3 | Median | Spread |
| --- | --- | --- | --- | --- | --- |
| light/add_torch | 1.122 | 1.090 | 1.111 | 1.111 | 2.9% |
| light/remove_torch | 5.972 | 6.095 | 6.091 | 6.091 | 2.0% |
| light/break_block | 0.162 | 0.161 | 0.157 | 0.161 | 3.3% |
| light/place_block | 1.288 | 1.274 | 1.337 | 1.288 | 4.9% |
| light/worst_dark_torch_add | 19.215 | 18.823 | 18.741 | 18.823 | 2.5% |
| light/worst_dark_torch_remove | 15.164 | 15.270 | 14.642 | 15.164 | 4.1% |
| light/worst_cave_roof_add | 15.974 | 16.374 | 16.313 | 16.313 | 2.5% |
| light/worst_cave_roof_remove | 14.027 | 13.889 | 13.150 | 13.889 | 6.3% |
| light/full_relight_3x3x3 | 931.880 | 949.120 | 934.700 | 934.700 | 1.8% |

### Budget verdicts (light)

| Budget row | Target | Bare metal | Verdict |
| --- | --- | --- | --- |
| Lighting add per edit | ≤ 100 µs p95 | corpus p95 ≤ 2.8 µs; worst cases p95 ≤ 24.7 µs | **Met** |
| Lighting remove per edit | ≤ 200 µs p95 | corpus p95 ≤ 14.2 µs; worst cases p95 ≤ 20.5 µs | **Met** |

Notes:

- In attempt 2 under clean AC conditions, spreads across all criterion runs
  dropped to 1.8–6.3% (add_torch spread was only 2.9%, compared to 41% in
  the initial run).
- Full recompute of a 3×3×3 snapshot: p50 862–1045 µs, faster than the
  1.4–2.2 ms research-VM figure quoted in the lighting spec.

## Owner decision on the misses (2026-11-21)

Per the task and AGENTS.md, the agent stopped and asked the owner when
targets were missed. Decision: **record the measured deviations, keep the
targets unchanged**, and add a follow-up optimization task to the stage 1
carry-over list (`tasks/README.md`). The affected budget rows are marked
"Measured deviation (bare metal …)" with the VM value kept in brackets.

Interpretation for the follow-up: the misses are concentrated in
random-access, memory-latency-bound paths (`get`, `set`, gather). The
direction is consistent with mobile Zen+ vs the server Xeon the VM numbers
came from, but the size of the gap (≈ 2–3×) is not explained by clock alone
and deserves profiling (cache behavior, 4K aliasing, layout) before any
target is renegotiated.

## Files

- `world-run{1,2,3}.txt`, `mesh-run{1,2,3}.txt`, `light-run{1,2,3}.txt` —
  raw console output of the nine verified protocol runs (attempt 2).
- `power-log.txt` — continuous AC power / battery and CPU frequency log during
  all nine runs (86 samples every 20s, verified AC and boost).
- `attempt-1-power-uncertain/` — demoted attempt 1 runs and `spot-check-base-clock.txt`
  with explanation README.
