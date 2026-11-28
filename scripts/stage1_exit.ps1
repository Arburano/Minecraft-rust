# T1-15 / T1-25: stage 1 exit measurement on the owner's laptop.
#   powershell -File scripts\stage1_exit.ps1                         # walk at 12 chunks: 3 x 30 min on the Vega 10, then 1 x 30 min on the GTX 1660 Ti
#   powershell -File scripts\stage1_exit.ps1 -Mode fly               # the T1-15 fly route at 16 chunks (stress case)
#   powershell -File scripts\stage1_exit.ps1 -Seconds 300 -Runs 1    # a quick 5-minute check first
# Plugged in, other programs closed, the game window left in front (it pauses on focus loss only outside this mode).
# Results: %APPDATA%\voxel-engine\bench\summary-*.txt, frames-*.csv and spikes-*.csv
# (every frame over 33 ms with the time of each phase); paste the summaries back and commit the spikes CSVs.
param([int]$Seconds = 1800, [int]$Runs = 3, [ValidateSet("walk", "fly")][string]$Mode = "walk")
cargo build --release -p client
if ($LASTEXITCODE -ne 0) { exit 1 }
for ($i = 1; $i -le $Runs; $i++) {
    Write-Host "Vega 10, $Mode route, run $i of $Runs ($Seconds s + 30 s warm-up)"
    & target\release\client.exe --offline-name dev_01 --gpu vega --bench-mode $Mode --bench-route $Seconds
}
Write-Host "GTX 1660 Ti, $Mode route, additional data"
& target\release\client.exe --offline-name dev_01 --gpu 1660 --bench-mode $Mode --bench-route $Seconds
