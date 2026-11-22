$ErrorActionPreference = "Stop"

$outDir = "C:\Users\aboba\Documents\Qoder\2026-11-18\89794042\Minecraft-rust\docs\research\bare-metal-2026-11-22-wave2"
if (!(Test-Path $outDir)) {
    New-Item -ItemType Directory -Path $outDir -Force | Out-Null
}

$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"
$powerLog = "$outDir\power-log.txt"
"timestamp,battery_status,charge_percent,cpu_perf_pct" | Out-File -FilePath $powerLog -Encoding utf8

function Log-Power {
    try {
        $batt = Get-CimInstance -ClassName Win32_Battery | Select-Object -First 1
        $perf = (Get-Counter '\Processor Information(_Total)\% Processor Performance' -ErrorAction SilentlyContinue).CounterSamples[0].CookedValue
        $time = (Get-Date).ToString("yyyy-MM-dd HH:mm:ss")
        "$time,$($batt.BatteryStatus),$($batt.EstimatedChargeRemaining),$perf" | Out-File -FilePath $powerLog -Append -Encoding utf8
    } catch {
        # ignore counter hiccups
    }
}

Log-Power
Write-Host "Starting Wave 2 bare-metal benchmark suite on ASUS TUF FX505DU..."

# --- T1-10: Flythrough (3 runs) ---
Write-Host "Running T1-10 flythrough (3 runs)..."
for ($i = 1; $i -le 3; $i++) {
    Log-Power
    Write-Host "  Flythrough run $i..."
    & $cargo run --release -p client --example flythrough -- 12 7 20 | Out-File -FilePath "$outDir\flythrough-run$i.txt" -Encoding utf8
}

# --- T1-12: Edit latency (3 runs) ---
Write-Host "Running T1-12 edit latency (3 runs)..."
for ($i = 1; $i -le 3; $i++) {
    Log-Power
    Write-Host "  Edit latency run $i..."
    & $cargo run --release -p client --example edit_latency -- 100 | Out-File -FilePath "$outDir\edit_latency-run$i.txt" -Encoding utf8
}

# --- T1-11: Terrain renderer on Vega 10 (DX12) - B, Bi, C (3 runs each) ---
Write-Host "Running T1-11 Vega 10 DX12 (B, Bi, C - 3 runs each)..."
$env:ADAPTER_NAME = "vega"
$env:WGPU_BACKEND = "dx12"
$env:FRAMES = "300"

foreach ($path in @("B", "Bi", "C")) {
    for ($i = 1; $i -le 3; $i++) {
        Log-Power
        Write-Host "  Vega DX12 $path run $i..."
        $bmpName = "$outDir\vega-$($path.ToLower())-run$i.bmp"
        & $cargo run --release -p client --example screenshot -- 12 $bmpName $path | Out-File -FilePath "$outDir\vega-dx12-$($path.ToLower())-run$i.txt" -Encoding utf8
    }
}

# --- T1-11: Terrain renderer on GTX 1660 Ti (DX12) - B, Bi, C (1 run each for comparison) ---
Write-Host "Running T1-11 GTX 1660 Ti DX12 (B, Bi, C)..."
$env:ADAPTER_NAME = "1660"
$env:WGPU_BACKEND = "dx12"
$env:FRAMES = "300"

foreach ($path in @("B", "Bi", "C")) {
    Log-Power
    Write-Host "  GTX 1660 Ti DX12 $path..."
    $bmpName = "$outDir\gtx1660-$($path.ToLower()).bmp"
    & $cargo run --release -p client --example screenshot -- 12 $bmpName $path | Out-File -FilePath "$outDir\gtx1660-dx12-$($path.ToLower()).txt" -Encoding utf8
}

# --- T1-11: Vulkan runs on Vega 10 and GTX 1660 Ti ---
Write-Host "Running T1-11 Vulkan runs..."
$env:WGPU_BACKEND = "vulkan"
$env:VK_LOADER_LAYERS_DISABLE = "VK_LAYER_AMD_switchable_graphics"
$env:FRAMES = "300"

$env:ADAPTER_NAME = "vega"
Log-Power
Write-Host "  Vega Vulkan B..."
& $cargo run --release -p client --example screenshot -- 12 "$outDir\vega-vulkan-b.bmp" B | Out-File -FilePath "$outDir\vega-vulkan-b.txt" -Encoding utf8

$env:ADAPTER_NAME = "1660"
Log-Power
Write-Host "  GTX 1660 Ti Vulkan B..."
& $cargo run --release -p client --example screenshot -- 12 "$outDir\gtx1660-vulkan-b.bmp" B | Out-File -FilePath "$outDir\gtx1660-vulkan-b.txt" -Encoding utf8

Log-Power
Write-Host "All Wave 2 benchmark runs completed successfully!"
