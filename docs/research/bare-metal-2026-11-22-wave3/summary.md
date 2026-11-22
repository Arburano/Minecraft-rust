# Bare-Metal Benchmark Summary — Wave 3 (T1-13 & T1-16)

- **Date:** 2026-11-22 18:35–18:38 local time
- **Host:** ASUS TUF Gaming FX505DU
  - **CPU:** AMD Ryzen 7 3750H (4C/8T, base 2.3 GHz, boost up to 4.0 GHz)
  - **Integrated GPU:** AMD Radeon(TM) RX Vega 10 Graphics
  - **Discrete GPU:** NVIDIA GeForce GTX 1660 Ti Mobile
  - **OS:** Windows 11 Home 24H2
  - **Rust:** 1.97.1 pinned
- **Power conditions:** Plugged into AC power (`PowerLineStatus = Online`, battery charging, CPU boost ~151%).

---

## 1. T1-13 — Client Application, Menu & Single-Player

### Cold Start to Menu (`scripts/cold_start.ps1`)
Protocol: 3 runs measuring time from process start to the first rendered menu frame via `--measure-startup`.

| Run | Time | Note |
|---|---|---|
| Run 1 (cold, first run after build) | **1,640 ms** (1.64 s) | Cold shader/driver initialization |
| Run 2 | **913 ms** (0.91 s) | Warm driver cache |
| Run 3 | **853 ms** (0.85 s) | Warm driver cache |
| Verification repetition | 1,015 ms / 883 ms / 956 ms | Consistent < 1 s warm startup |

- **Acceptance criterion (Cold start ≤ 5 s):** **MET** (1.64 s cold, > 3× faster than budget).

### Interactive & In-Game Autonomous Test
- Command: `client.exe --offline-name dev_01 --autoplay LaptopTest --screenshot docs/research/t1-13-laptop-ingame.bmp:120`
- Result: Window launched, wgpu presented to surface using default **Path Bi (`PullingIndexed`)**, terrain generated and rendered in real time, 120 frames rendered, and saved screenshot `docs/research/t1-13-laptop-ingame.png`.
- Integration test: `cargo test -p client --test session` ➔ `create_play_quit_load_returns_the_same_world ... ok`.

---

## 2. T1-16 — Faster World-Path Lighting Confirmation

### Before vs After T1-16 Comparison on Bare Metal (Ryzen 7 3750H)
3 runs of `cargo run --release -p client --example edit_latency -- 100` (100 edits per kind):

| Edit kind | Phase | Before T1-16 (T1-12) | After T1-16 (Wave 3) | Improvement |
|---|---|---|---|---|
| **Break surface block**<br>*(direct sky change)* | set_block + remesh | p50 0.55–0.61 ms, p95 ≤ 1.26 ms | p50 0.49–0.57 ms, p95 ≤ 1.16 ms | ~1.1× |
| | relight | p50 **11.8–13.3 ms**, p95 ≤ 18.5 ms | p50 **0.39–0.45 ms**, p95 ≤ 0.65 ms | **~28× FASTER** |
| | batch re-copy | p50 16.9–17.9 µs | p50 6.5–12.0 µs | ~1.8× |
| | **total** | p50 **12.4–14.0 ms**, p95 ≤ 19.3 ms | p50 **0.92–1.02 ms**, p95 ≤ 1.71 ms | **~13× FASTER** |
| **Place block in air**<br>*(direct sky change)* | set_block + remesh | p50 0.53–0.54 ms, p95 ≤ 1.05 ms | p50 0.48–0.58 ms, p95 ≤ 1.14 ms | ~1.0× |
| | relight | p50 **11.8–12.8 ms**, p95 ≤ 16.1 ms | p50 **0.36–0.43 ms**, p95 ≤ 0.91 ms | **~30× FASTER** |
| | batch re-copy | p50 12.9–14.5 µs | p50 4.4–5.4 µs | ~2.5× |
| | **total** | p50 **12.5–13.4 ms**, p95 ≤ 16.7 ms | p50 **0.84–1.01 ms**, p95 ≤ 2.08 ms | **~13× FASTER** |
| **Torch underground**<br>*(local relight)* | set_block + remesh | p50 0.43–0.52 ms, p95 ≤ 1.14 ms | p50 0.44–0.59 ms, p95 ≤ 1.08 ms | ~1.0× |
| | relight | p50 **0.78–0.95 ms**, p95 ≤ 1.47 ms | p50 **0.29–0.42 ms**, p95 ≤ 0.53 ms | **~2.5× FASTER** |
| | batch re-copy | p50 11.4–13.3 µs | p50 8.4–24.8 µs | ~1.0× |
| | **total** | p50 **1.24–1.50 ms**, p95 ≤ 2.58 ms | p50 **0.75–1.06 ms**, p95 ≤ 1.63 ms | **~1.6× FASTER** |

### World Light Criterion Benchmark (`cargo bench -p light --bench world_light`)
- `world edit torch place`: p50 **332 µs**, p95 **524 µs** (was ~700–900 µs).
- `world edit torch restore`: p50 **337 µs**, p95 **515 µs**.
- `world edit stone_on_surface place`: p50 **317 µs**, p95 **488 µs** (was 10,000–13,000 µs!).
- `world edit stone_on_surface restore`: p50 **319 µs**, p95 **470 µs**.
- `world edit roof_over_shaft` (rare deep 60-block shaft): p50 **10.87 ms**, p95 **13.72 ms** (down from 25–36 ms).
- `initial_light_one_column`: p50 **7.9 ms** (Criterion: 8.38 ms).

### Acceptance Criteria Verdict
- **World-path edit p95 measured before/after on the laptop**: **MET**. Torch relight p95 improved from ~1.0–1.5 ms down to **0.44–0.53 ms**; stone on surface relight p95 improved from 16–18 ms down to **0.47–0.65 ms**!
- **Sky-changing edits frame spike eliminated**: **MET**. Total latency for surface block edits is now **0.84–1.02 ms p50** (p95 ≤ 2.08 ms), completing 100% within a single 60 FPS frame (16.6 ms).
