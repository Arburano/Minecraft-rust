# Attempt 1 (demoted): power state uncertain

These nine runs (2026-11-21 15:21–15:51) and the base-clock spot-check were
the first measurement attempt. A pre-run check reported AC power
(Win32_Battery BatteryStatus=2, charge 40%), and the numbers are internally
consistent and match a later verified AC+boost spot-check (get/2bit
34.25/35.26 µs). However, the owner reported that the laptop ran on battery
at some point during the session; after the protocol the CPU latched at
base clock (2300 MHz, perf ≈ 100%, no boost) and the spot-checks in that
state read ≈ 1.5× worse (`spot-check-base-clock.txt`). Boost recovered once
AC power was confirmed again (perf ≈ 155%).

Because the power state during the protocol cannot be guaranteed, these
runs are kept as evidence but are **not** the official confirmation values.
The official runs are the top-level `world-run{1,2,3}.txt`,
`mesh-run{1,2,3}.txt`, `light-run{1,2,3}.txt` of the parent folder, taken
with verified AC power and a continuous frequency/battery log
(`power-log.txt`).