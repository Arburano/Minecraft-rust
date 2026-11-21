# T0-15. Stabilize the flaky QUIC spike test

- **Status:** done
- **Depends on:** T0-11
- **Owner:** agent (cloud)

## Goal

`net::quic_spike::lossless_full_transfer` (`crates/net/tests/quic_spike.rs`) fails intermittently on an arrival-order assertion: once in CI of PR #16 and once locally during T0-09 verification ([T0-09 report](../../task-reports/t0-09-sim-region-prototype.md)). A flaky test blocks unrelated PRs.

## Result

Root cause: the test asserted the client's reader-task completion order, which on a lossless localhost link is decided by task scheduling, not by stream priority (failure rate 1% quiet, 14% under load). The server now opens streams and sets priorities in distance order deterministically, and the test checks that send order via stream indices. 600/600 consecutive passes (quiet + loaded). Details: [report](../../task-reports/t0-15-net-flaky-test.md).

## Steps

1. Reproduce: run the test in a loop (≥ 200 runs, also with all cores busy) and record the failure rate and the failing assertion.
2. Find the cause: is the asserted order actually guaranteed by the protocol (`docs/architecture/networking-protocol.md`: priority streams for columns, datagrams for deltas)? If the order is **not** guaranteed, the test asserts the wrong thing; if it is, the code has a bug.
3. Fix the cause (test or code). Do **not** add retries, sleeps or `#[ignore]`.
4. Verify: ≥ 500 consecutive passes, including under CPU load. Report numbers and the machine.

## Acceptance criteria

- [x] Root cause stated in the report; fix applied without retries/ignore.
- [x] ≥ 500 consecutive passes (quiet + loaded), CI green.
- [x] Report `task-reports/t0-15-net-flaky-test.md`.
