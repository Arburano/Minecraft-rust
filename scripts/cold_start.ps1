# T1-13: time from process start to the first menu frame, 3 runs (Windows).
# Build first: cargo build --release -p client
# The first run after a reboot is the cold start; later runs are warm.
param([string]$Bin = "target\release\client.exe")
for ($i = 1; $i -le 3; $i++) {
    & $Bin --offline-name dev_01 --measure-startup | Select-String "startup"
}
