$ErrorActionPreference = "Stop"

$outDir = "C:\Users\aboba\Documents\Qoder\2026-11-18\89794042\Minecraft-rust\docs\research\bare-metal-2026-11-22-wave3"
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
    } catch {}
}

Log-Power
Write-Host "Starting Wave 3 bare-metal measurements (T1-13 & T1-16)..."

# --- T1-13: Cold start (3 runs) ---
Write-Host "Running T1-13 cold start (3 runs)..."
& powershell -ExecutionPolicy Bypass -File scripts\cold_start.ps1 | Out-File -FilePath "$outDir\cold_start.txt" -Encoding utf8

# --- T1-16: Edit latency (3 runs) ---
Write-Host "Running T1-16 edit latency (3 runs)..."
for ($i = 1; $i -le 3; $i++) {
    Log-Power
    Write-Host "  Edit latency run $i..."
    & $cargo run --release -p client --example edit_latency -- 100 | Out-File -FilePath "$outDir\edit_latency-run$i.txt" -Encoding utf8
}

# --- T1-16: World light benchmark ---
Write-Host "Running T1-16 world light benchmark..."
Log-Power
& $cargo bench -p light --bench world_light -- --warm-up-time 1 --measurement-time 2 | Out-File -FilePath "$outDir\world_light-bench.txt" -Encoding utf8

Log-Power
Write-Host "Wave 3 bare-metal measurements finished successfully!"
