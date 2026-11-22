#!/usr/bin/env bash
# T1-13: time from process start to the first menu frame, 3 runs.
# Build first: cargo build --release -p client
# A "cold" run means the OS file cache does not hold the binary yet: run this
# right after a reboot for the first number; later runs are warm.
set -euo pipefail
bin="${1:-target/release/client}"
for i in 1 2 3; do
  "$bin" --offline-name dev_01 --measure-startup | grep startup
done
