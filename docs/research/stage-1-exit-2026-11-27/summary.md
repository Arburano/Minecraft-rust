# T1-15. Stage 1 exit measurement — laptop results (2026-11-27)

## Machine and conditions

- **Laptop:** ASUS TUF FX505DU (bare metal, Windows 11, AC power plugged in)
- **iGPU:** AMD Radeon(TM) RX Vega 10 Graphics (driver `27.20.1032.2`, `Dx12`)
- **dGPU:** NVIDIA GeForce GTX 1660 Ti (driver `32.0.15.5597`, `Dx12`)
- **Commit:** `cff0936` (release build `cargo build --release -p client`)
- **Command:** `powershell -ExecutionPolicy Bypass -File scripts\stage1_exit.ps1`
  - 3 × 1800 s runs on Vega 10 + 1 × 1800 s run on GTX 1660 Ti (each preceded by a 30 s warm-up).
  - Profile: `High (16 chunks)`, path `PullingIndexed`, scripted 1000-block square fly route (`9983` blocks flown at ≈ `5.5` blocks/s).

## Full-run results (1800 s each)

| Run | Start to menu | Avg FPS (target ≥ 60) | 1% low FPS (target ≥ 45) | Frame time p50 / p99 / max | Frames > 33 ms (target 0) | RAM avg / peak (target ≤ 1536 MiB) | Raw summary |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Vega 10 #1 | 1311 ms | **270.4** | **74.0** | 3.42 / 7.93 / 116.47 ms | **161** / 486,697 | 321 / **345 MiB** | [summary-1795799922.txt](summary-1795799922.txt) |
| Vega 10 #2 | 1571 ms | **295.8** | **103.0** | 3.22 / 6.04 / 114.94 ms | **36** / 532,388 | 308 / **324 MiB** | [summary-1795801756.txt](summary-1795801756.txt) |
| Vega 10 #3 | 1481 ms | **257.9** | **50.9** | 3.37 / 16.64 / 119.60 ms | **189** / 464,278 | 247 / **326 MiB** | [summary-1795803588.txt](summary-1795803588.txt) |
| GTX 1660 Ti | 1670 ms | **325.6** | **93.8** | 2.91 / 6.90 / 94.55 ms | **98** / 586,111 | 325 / **346 MiB** | [summary-1795805421.txt](summary-1795805421.txt) |

A preceding 300 s quick check (`-Seconds 300 -Runs 1`) showed consistent results: Vega 10 avg 250.9 FPS, 1% low 71.4, RAM peak 339 MiB, 14 frames > 33 ms, startup 2839 ms; GTX 1660 Ti avg 253.0 FPS, 1% low 67.6, RAM peak 347 MiB, 49 frames > 33 ms, startup 2059 ms.

## Assessment against Stage 1 exit criteria

- **Start to menu (≤ 5 s):** **Met** — 1.31–1.67 s across all 4 runs.
- **FPS on iGPU (≥ 60, 1% low ≥ 45):** **Met** (even on `High`, 16 chunks) — Vega 10 average 257.9–295.8 FPS, 1% low 50.9–103.0 FPS.
- **Client RAM (≤ 1.5 GB exit / ≤ 1 GiB internal):** **Met** (moves from *Hypothesis* to *Measured*) — peak 324–346 MiB, average 247–325 MiB.
- **Frame spikes (0 frames > 33 ms / 30 min):** **Deviation** — 36–189 frames > 33 ms on Vega 10 (max 114.94–119.60 ms) and 98 frames > 33 ms on GTX 1660 Ti (max 94.55 ms). Note that the bench route flies continuously at ≈ 5.5 blocks/s over ungenerated terrain (`High`, 16 chunks), which maximizes chunk generation/meshing churn.
