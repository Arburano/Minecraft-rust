@echo off
rem T0-14 part B benchmark matrix. Run from the repository root in cmd.exe:
rem   research\gpu\run-matrix.cmd DATE "MACHINE STRING"
rem Example:
rem   research\gpu\run-matrix.cmd 2026-11-20 "ASUS TUF FX505DU; Ryzen 7 3750H; 14 GiB; Windows 11 build 28000; NVIDIA 555.97; AMD 27.20.1032.2"
rem Re-check the adapter indices below against the fresh laptop-probe-DATE.json
rem before trusting the file names; indices are per backend and can change.
setlocal EnableExtensions
if "%~1"=="" goto :usage
if "%~2"=="" goto :usage
set "DATE=%~1"
set "M=%~2"
set "OUT=docs\research\gpu"
set "BENCH=target\release\quad-bench.exe"
set "PROBE=target\release\gpu-probe.exe"
set "VEGA_DX12=1"
set "NV_DX12=0"
set "NV_GL=0"
if not exist "%BENCH%" goto :nobuild
if not exist "%PROBE%" goto :nobuild

"%PROBE%" > "%OUT%\laptop-probe-%DATE%.json"
if errorlevel 1 goto :fail

for %%p in (b c) do (
  for %%r in (1 2 3) do (
    "%BENCH%" --backend dx12 --adapter %VEGA_DX12% --path %%p --machine "%M%" --environment bare-metal > "%OUT%\vega10-dx12-%%p-%DATE%-run%%r.json"
    if errorlevel 1 goto :fail
    "%BENCH%" --backend dx12 --adapter %NV_DX12% --path %%p --machine "%M%" --environment bare-metal > "%OUT%\gtx1660ti-dx12-%%p-%DATE%-run%%r.json"
    if errorlevel 1 goto :fail
    "%BENCH%" --backend gl --adapter %NV_GL% --path %%p --machine "%M%" --environment bare-metal > "%OUT%\gtx1660ti-gl-%%p-%DATE%-run%%r.json"
    if errorlevel 1 goto :fail
  )
)

"%BENCH%" --backend dx12 --adapter %VEGA_DX12% --path b --no-timestamps --machine "%M%" --environment bare-metal > "%OUT%\vega10-dx12-b-notimestamps-%DATE%.json"
if errorlevel 1 goto :fail
"%BENCH%" --backend dx12 --adapter %NV_DX12% --path b --no-timestamps --machine "%M%" --environment bare-metal > "%OUT%\gtx1660ti-dx12-b-notimestamps-%DATE%.json"
if errorlevel 1 goto :fail

set "WGPU_BACKEND=dx12"
set "QUAD_BENCH_ADAPTER_NAME=Vega"
cargo test -p render --bin quad-bench gpu_paths_render_identical_nonblank_images -- --ignored > "%OUT%\imagetest-vega10-dx12-%DATE%.txt" 2>&1
if errorlevel 1 goto :fail
set "QUAD_BENCH_ADAPTER_NAME=1660"
cargo test -p render --bin quad-bench gpu_paths_render_identical_nonblank_images -- --ignored > "%OUT%\imagetest-gtx1660ti-dx12-%DATE%.txt" 2>&1
if errorlevel 1 goto :fail
set "WGPU_BACKEND=gl"
cargo test -p render --bin quad-bench gpu_paths_render_identical_nonblank_images -- --ignored > "%OUT%\imagetest-gtx1660ti-gl-%DATE%.txt" 2>&1
if errorlevel 1 goto :fail

echo Matrix complete. Verify every JSON has "build_profile": "release" and "environment": "bare-metal".
exit /b 0

:nobuild
echo Missing release binaries. Run: cargo build --release -p render --bins 1>&2
exit /b 1
:fail
echo A run failed. Keep the error text in a .txt next to the partial output; do not commit empty JSON. 1>&2
exit /b 1
:usage
echo Usage: research\gpu\run-matrix.cmd DATE "MACHINE STRING" 1>&2
exit /b 1