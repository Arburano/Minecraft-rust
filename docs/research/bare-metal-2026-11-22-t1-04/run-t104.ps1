$outDir = "C:\Users\aboba\Documents\Qoder\2026-11-18\89794042\Minecraft-rust\docs\research\bare-metal-2026-11-22-t1-04"
$cargo = "$env:USERPROFILE\.cargo\bin\cargo.exe"
$repo = "C:\Users\aboba\Documents\Qoder\2026-11-18\89794042\Minecraft-rust"

$powerJob = Start-Job -ScriptBlock {
    param($dir)
    $logFile = "$dir\power-log.txt"
    while ($true) {
        $p = Get-CimInstance Win32_PerfFormattedData_Counters_ProcessorInformation | Where-Object Name -eq '_Total'
        $b = Get-CimInstance Win32_Battery
        $time = Get-Date -Format "HH:mm:ss"
        "$time freq=$($p.ProcessorFrequency) perf=$($p.PercentProcessorPerformance)% battery=$($b.BatteryStatus) charge=$($b.EstimatedChargeRemaining)%" | Out-File -Append -FilePath $logFile -Encoding utf8
        Start-Sleep -Seconds 20
    }
} -ArgumentList $outDir

try {
    for ($run = 1; $run -le 3; $run++) {
        Write-Host "Starting world run $run..."
        & $cargo bench --manifest-path "$repo\Cargo.toml" -p world --bench section *>&1 | Out-File -FilePath "$outDir\world-run$run.txt" -Encoding utf8
        Write-Host "Finished world run $run."

        Write-Host "Starting mesh run $run..."
        & $cargo bench --manifest-path "$repo\Cargo.toml" -p mesh --bench mesh_chunk *>&1 | Out-File -FilePath "$outDir\mesh-run$run.txt" -Encoding utf8
        Write-Host "Finished mesh run $run."
    }
} finally {
    Stop-Job $powerJob
    Receive-Job $powerJob
    Remove-Job $powerJob
}
Write-Host "ALL BENCHMARKS COMPLETED!"
