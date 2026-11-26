# T2-05. Headless bot clients and the load harness

- **Status:** todo
- **Depends on:** T2-03, T2-04
- **Owner:** agent (cloud) + owner's machine for the 8-core run

## Goal

A `bots` binary: N protocol-level clients without rendering (scripted walking, flying, digging and building, chat-free), joining a server. It reports per-bot traffic, and on the server side tick p50/p95/p99, CPU and memory.

## Acceptance criteria

- [ ] 50 bots run in CI-size containers without errors (smoke test).
- [ ] The harness writes a summary like the T1-15 route (tick percentiles, kbit/s per player, memory).

## Out of scope

The stage 2 exit run itself (T2-09).
