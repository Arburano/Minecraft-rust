@echo off
rem T0-13 interruption-latency evidence (owner decision No. 11).
rem Run from the repository root in cmd.exe:
rem   research\modhost\run-latency.cmd DATE "MACHINE STRING"
rem Example:
rem   research\modhost\run-latency.cmd 2026-11-19 "ASUS TUF FX505DU; Ryzen 7 3750H; 14 GiB; Windows 11 build 28000"
rem Requires the pinned Rust toolchain, the wasm32-wasip2 target, and prebuilt:
rem   cargo test -p modhost --release --test interruption_latency --test component --no-run
rem   cargo build --release -p modhost --example cpu_load
rem   cargo test -p modhost --test component --no-run
rem Quiet runs: close heavy applications first; no generated load, background
rem activity is not controlled. Loaded runs: cpu_load.exe spins on every logical
rem CPU, the machine will lag until taskkill. Each latency run performs 1,000
rem interruptions with a 100 ms hook budget (see tests/interruption_latency.rs).
setlocal EnableExtensions
if "%~1"=="" goto :usage
if "%~2"=="" goto :usage
set "DATE=%~1"
set "M=%~2"
set "OUT=docs\research\modhost"
set "ROOT=%CD%"
if not exist "%OUT%" mkdir "%OUT%"
echo machine: %M%> "%OUT%\runs-%DATE%.txt"
rustc --version>> "%OUT%\runs-%DATE%.txt"
cargo --version>> "%OUT%\runs-%DATE%.txt"

rem --- quiet latency: 3 runs x 1,000 interruptions ---
for %%r in (1 2 3) do (
  set "MODHOST_LATENCY_SAMPLES=%ROOT%\%OUT%\latency-quiet-%DATE%-run%%r-samples.txt"
  cargo test -p modhost --release --test interruption_latency -- --ignored --nocapture > "%OUT%\latency-quiet-%DATE%-run%%r.txt" 2>&1
  if errorlevel 1 goto :fail
)

rem --- loaded latency: 3 runs x 1,000 interruptions, all cores busy ---
start "" /B target\release\examples\cpu_load.exe
timeout /t 3 /nobreak > nul
for %%r in (1 2 3) do (
  set "MODHOST_LATENCY_SAMPLES=%ROOT%\%OUT%\latency-loaded-%DATE%-run%%r-samples.txt"
  cargo test -p modhost --release --test interruption_latency -- --ignored --nocapture > "%OUT%\latency-loaded-%DATE%-run%%r.txt" 2>&1
  if errorlevel 1 goto :failkill
)
taskkill /F /IM cpu_load.exe > nul 2>&1

rem --- functional CI test: 20 consecutive runs under load (debug, as CI) ---
start "" /B target\release\examples\cpu_load.exe
timeout /t 3 /nobreak > nul
for /L %%i in (1,1,20) do (
  echo === functional run %%i ===>> "%OUT%\functional-loaded-%DATE%.txt"
  cargo test -p modhost --test component infinite_loop_is_interrupted_and_tick_continues -- --nocapture >> "%OUT%\functional-loaded-%DATE%.txt" 2>&1
  if errorlevel 1 goto :failkill
)
taskkill /F /IM cpu_load.exe > nul 2>&1
echo all runs passed>> "%OUT%\runs-%DATE%.txt"
echo done
exit /b 0

:failkill
taskkill /F /IM cpu_load.exe > nul 2>&1
:fail
echo FAILED; see the output files in %OUT%
exit /b 1

:usage
echo usage: research\modhost\run-latency.cmd DATE "MACHINE STRING"
exit /b 1