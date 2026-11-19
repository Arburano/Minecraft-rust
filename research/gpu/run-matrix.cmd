@echo off
rem T0-14 part B benchmark matrix. Run from the repository root in cmd.exe:
rem   research\gpu\run-matrix.cmd DATE "MACHINE STRING"
rem Example:
rem   research\gpu\run-matrix.cmd 2026-11-20 "ASUS TUF FX505DU; Ryzen 7 3750H; 14 GiB; Windows 11 build 28000; NVIDIA 555.97; AMD 27.20.1032.2"
rem Indices below match the 2026-11-19 probe AFTER the NVIDIA Control Panel
rem program setting "Integrated graphics" was applied to gpu-probe.exe and
rem quad-bench.exe (Vega 10 first on DX12/Vulkan, Vega 10 alone on GL).
rem Without that setting the order is reversed (GTX 1660 Ti = 0) and GL shows
rem the GTX 1660 Ti instead; re-check the fresh laptop-probe-DATE.json and
rem adjust before trusting the file names.
rem The image test runs from target\debug\deps\quad_bench-*.exe; give that exe
rem the same integrated-GPU preference (Windows Settings > Display > Graphics,
rem or HKCU\Software\Microsoft\DirectX\UserGpuPreferences "GpuPreference=1;")
rem or the Vega GL image test will not find its adapter.
setlocal EnableExtensions
if "%~1"=="" goto :usage
if "%~2"=="" goto :usage
set "DATE=%~1"
set "M=%~2"
set "OUT=docs\research\gpu"
set "BENCH=target\release\quad-bench.exe"
set "PROBE=target\release\gpu-probe.exe"
set "VEGA_DX12=0"
set "NV_DX12=1"
set "VEGA_GL=0"
set "VEGA_VK=0"
set "NV_VK=1"
rem The AMD switchable-graphics layer breaks vkEnumeratePhysicalDevices on this
rem laptop (INCOMPLETE, zero devices); disabling it exposes both GPUs.
set "VK_LOADER_LAYERS_DISABLE=VK_LAYER_AMD_switchable_graphics"
if not exist "%BENCH%" goto :nobuild
if not exist "%PROBE%" goto :nobuild

rem gpu-probe may crash at exit (exit 139) AFTER writing complete JSON when GL
rem is on the Vega 10 (AMD driver teardown, see probe-crash-note-2026-11-19.txt),
rem so check the output size instead of the exit code.
"%PROBE%" > "%OUT%\laptop-probe-%DATE%.json"
for %%F in ("%OUT%\laptop-probe-%DATE%.json") do if %%~zF==0 goto :fail

for %%p in (b c) do (
  for %%r in (1 2 3) do (
    "%BENCH%" --backend dx12 --adapter %VEGA_DX12% --path %%p --machine "%M%" --environment bare-metal > "%OUT%\vega10-dx12-%%p-%DATE%-run%%r.json"
    if errorlevel 1 goto :fail
    "%BENCH%" --backend dx12 --adapter %NV_DX12% --path %%p --machine "%M%" --environment bare-metal > "%OUT%\gtx1660ti-dx12-%%p-%DATE%-run%%r.json"
    if errorlevel 1 goto :fail
    "%BENCH%" --backend gl --adapter %VEGA_GL% --path %%p --machine "%M%" --environment bare-metal > "%OUT%\vega10-gl-%%p-%DATE%-run%%r.json"
    if errorlevel 1 goto :fail
    rem GTX 1660 Ti on GL only while the program setting above is removed:
    rem "%BENCH%" --backend gl --adapter 0 --path %%p --machine "%M%" --environment bare-metal > "%OUT%\gtx1660ti-gl-%%p-%DATE%-run%%r.json"
    "%BENCH%" --backend vulkan --adapter %VEGA_VK% --path %%p --machine "%M%" --environment bare-metal > "%OUT%\vega10-vulkan-%%p-%DATE%-run%%r.json"
    if errorlevel 1 goto :fail
    "%BENCH%" --backend vulkan --adapter %NV_VK% --path %%p --machine "%M%" --environment bare-metal > "%OUT%\gtx1660ti-vulkan-%%p-%DATE%-run%%r.json"
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
set "QUAD_BENCH_ADAPTER_NAME=Vega"
cargo test -p render --bin quad-bench gpu_paths_render_identical_nonblank_images -- --ignored > "%OUT%\imagetest-vega10-gl-%DATE%.txt" 2>&1
if errorlevel 1 goto :fail
rem GTX 1660 Ti GL image test only while the program setting is removed:
rem set "QUAD_BENCH_ADAPTER_NAME=1660"
rem cargo test -p render --bin quad-bench gpu_paths_render_identical_nonblank_images -- --ignored > "%OUT%\imagetest-gtx1660ti-gl-%DATE%.txt" 2>&1
set "WGPU_BACKEND=vulkan"
set "QUAD_BENCH_ADAPTER_NAME=Vega"
cargo test -p render --bin quad-bench gpu_paths_render_identical_nonblank_images -- --ignored > "%OUT%\imagetest-vega10-vulkan-%DATE%.txt" 2>&1
if errorlevel 1 goto :fail
set "QUAD_BENCH_ADAPTER_NAME=1660"
cargo test -p render --bin quad-bench gpu_paths_render_identical_nonblank_images -- --ignored > "%OUT%\imagetest-gtx1660ti-vulkan-%DATE%.txt" 2>&1
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