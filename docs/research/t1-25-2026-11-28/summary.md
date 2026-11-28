# T1-25. Frame spike attribution — laptop quick check (2026-11-28)

## Machine and conditions

- **Laptop:** ASUS TUF FX505DU (bare metal, Windows 11, AC power plugged in)
- **iGPU:** AMD Radeon(TM) RX Vega 10 Graphics (driver `27.20.1032.2`, `Dx12`)
- **dGPU:** NVIDIA GeForce GTX 1660 Ti (driver `32.0.15.5597`, `Dx12`)
- **Branch / commit:** `ccr-b8153911-17d0gw` (`adfbfc0`)
- **Tests:** `cargo test -p client` passed (29/29)
- **Commands:**
  - `powershell -ExecutionPolicy Bypass -File scripts\stage1_exit.ps1 -Seconds 300 -Runs 1 -Mode walk`
  - `powershell -ExecutionPolicy Bypass -File scripts\stage1_exit.ps1 -Seconds 300 -Runs 1 -Mode fly`

## Results (300 s each after 30 s warm-up)

| Run | Start to menu | Avg FPS | 1% low FPS | p50 / p99 / max (ms) | Frames > 33 ms | RAM avg / peak | Top spike causes (longest phase) | Files |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Vega 10 `walk` (12 ch.) | 2206 ms | **311.3** | **115.9** | 3.02 / 6.33 / 56.23 | **3** / 93,399 | 287 / 300 MiB | `ui` 1 (56.2 ms), `encode` 1 (37.5 ms), `outside` 1 (39.7 ms) | [summary](summary-1795848773.txt), [spikes](spikes-1795848773.csv) |
| GTX 1660 Ti `walk` (12 ch.) | 1488 ms | **44.1** | **24.4** | 33.10 / 34.70 / 135.87 | **6973** / 13,238 | 284 / 295 MiB | `present` 6912 (avg 33.5 ms), `upload_light` 56 (avg 40.7 ms), `submit` 3, `outside` 2 | [summary](summary-1795849106.txt), [spikes](spikes-1795849106.csv) |
| Vega 10 `fly` (16 ch.) | 2701 ms | **97.1** | **29.9** | 9.35 / 20.06 / 120.87 | **143** / 29,128 | 313 / 324 MiB | `upload_light` 73 (avg 39.6 ms), `acquire` 46 (avg 38.3 ms), `generation` 7 (avg 58.8 ms), `submit` 6, `prepare` 4 | [summary](summary-1795849443.txt), [spikes](spikes-1795849443.csv) |
| GTX 1660 Ti `fly` (16 ch.) | 2897 ms | **30.8** | **17.1** | 33.32 / 48.34 / 127.09 | **7863** / 9,239 | 319 / 333 MiB | `present` 7757 (avg 33.4 ms), `upload_light` 103 (avg 55.1 ms), `prepare` 1, `encode` 1, `outside` 1 | [summary](summary-1795849777.txt), [spikes](spikes-1795849777.csv) |

## Key findings

1. **`upload_light` is the main internal engine spike cause** during chunk streaming: 73 spikes (avg 39.6 ms) on Vega 10 `fly`, 56 spikes (avg 40.7 ms) on GTX 1660 Ti `walk`, and 103 spikes (avg 55.1 ms) on GTX 1660 Ti `fly`.
2. **Vega 10 `walk` (12 chunks)** had only 3 spikes over 300 s (`ui`, `encode`, `outside`), with `saved_reads` = 0.00 ms and `upload_light` = 0.00 ms across spikes.
3. **GTX 1660 Ti `present` throttling:** `p50` sat at ~33.1–33.3 ms in both `walk` and `fly` with almost all spikes attributed to `present` (avg 33.4–33.5 ms), pointing to Optimus / DXGI swapchain presentation pacing rather than scene rendering cost.
