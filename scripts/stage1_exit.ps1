# T1-15: stage 1 exit measurement on the owner's laptop.
#   powershell -File scripts\stage1_exit.ps1                 # 3 x 30 min on the Vega 10, then 1 x 30 min on the GTX 1660 Ti
#   powershell -File scripts\stage1_exit.ps1 -Seconds 300    # a quick 5-minute check first
# Plugged in, other programs closed, the game window left in front (it pauses on focus loss only outside this mode).
# Results: %APPDATA%\voxel-engine\bench\summary-*.txt and frames-*.csv; paste the summaries back.
param([int]$Seconds = 1800, [int]$Runs = 3)
cargo build --release -p client
if ($LASTEXITCODE -ne 0) { exit 1 }
for ($i = 1; $i -le $Runs; $i++) {
    Write-Host "Vega 10, run $i of $Runs ($Seconds s + 30 s warm-up)"
    & target\release\client.exe --offline-name dev_01 --gpu vega --bench-route $Seconds
}
Write-Host "GTX 1660 Ti, additional data"
& target\release\client.exe --offline-name dev_01 --gpu 1660 --bench-route $Seconds
