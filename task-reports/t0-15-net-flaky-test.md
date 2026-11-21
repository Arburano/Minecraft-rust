# T0-15: stabilize the flaky QUIC spike test

## What was done

- **Reproduced** `net::quic_spike::lossless_full_transfer` failing on its order assertion ("3x3 member late"). On a cloud VM, 4 vCPU Xeon 2.1 GHz: **1/100** runs failed on a quiet machine and **14/100** with all 4 vCPUs busy (4 busy-loop processes).
- **Root cause: the test asserted something the protocol does not guarantee.** It required all 9 nearest columns to be among the first 20 entries of `ClientReport::columns`. That list is in the order the **client's reader tasks finished**. Temporary instrumentation showed:
  - The server side behaved correctly. In failing runs every nearest-3×3 stream had its data written within ~15 µs and was acknowledged by the transport within ~2.3–3.0 ms.
  - The reordering happened on the client. On a lossless localhost link the link is not the bottleneck, so stream priorities have nothing to reorder; all 16 in-flight streams arrive almost together, and the multi-threaded runtime decides which reader task completes first. Under CPU load a 3×3 column ended up 20th–64th.
  - A latent server-side variant of the same problem: each column stream was opened and given its priority **inside** its spawned task. Stream IDs and priorities therefore followed task scheduling rather than distance rank.
- **Fix:**
  - `crates/net/src/spike/server.rs`: the stream is opened and its priority set in the send loop, before the writer task is spawned. Stream IDs and priorities now follow the distance rank exactly.
  - `crates/net/src/spike/client.rs`: `ClientReport` gains `column_streams`, the QUIC stream index of each received column. The docs of `columns` now say its order reflects reader-task scheduling.
  - `crates/net/tests/quic_spike.rs`: the lossless test asserts the property the sender actually guarantees. Sorting the received columns by stream index must give exactly `columns_by_distance`. Byte integrity (625 columns, 625 × 8 KiB, payload verified per column), zero lost deltas and `time_to_first_3x3` are unchanged. Prioritised delivery **under a real bottleneck** stays covered by `nearest_columns_under_loss_and_deltas_not_blocked` (3×3 ≤ 1 s at 2% loss, RTT 100 ms). No retries, sleeps or `#[ignore]` were added.
- **Verification** on the same VM:
  - 300/300 runs passed on a quiet machine and 300/300 with all vCPUs busy, so **600 consecutive passes** against 1% / 14% failures before.
  - The whole `quic_spike` test binary passed 20/20.
  - Mutation check: reversing the server's send order makes the new assertion fail ("server must open column streams in distance order").
  - `cargo fmt --check` and `cargo clippy --workspace --all-targets -D warnings` are clean; `cargo test --workspace` is green.

## What went wrong

- The first diagnostic hypothesis (server task scheduling reorders stream opening) was only partly right. The instrumentation showed the observed failures came from the client's completion order, so the test assertion itself was changed, not only the server.

## Issues encountered

- The order in which the client's reader tasks finish is not a protocol property. Any future metric that needs "arrival order" should timestamp at the transport level, or be measured on a shaped link where bandwidth is the bottleneck. The bare-metal shaped-link re-run is already a stage 1 carry-over item.
- These are VM numbers; the failure rates before the fix depend on the machine's load.
