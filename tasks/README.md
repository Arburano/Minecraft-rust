# Tasks

The backlog is split by stage of the roadmap ([docs/process/roadmap.md](../docs/process/roadmap.md)). Each task is a separate file with its goal, links to specifications, deliverables and acceptance criteria.

## How to work with tasks

1. Take a task with status `todo` whose dependencies are all `done`.
2. Change the status to `in-progress` in the task file and in the table below, in the same PR as the code (or in a separate first commit on the branch).
3. A task is closed when all acceptance criteria are checked and CI is green. Status → `done`; the file links to the PR.
4. Found new work? Add a file `T<stage>-<NN>-<slug>.md` using the same template; do not expand someone else's task.
5. If a task contradicts a specification or ADR, stop and record the question in [docs/open-questions.md](../docs/open-questions.md); do not resolve it silently.

## Stage 0

| Task | Name | Depends on | Owner | Status |
| --- | --- | --- | --- | --- |
| [T0-01](stage-0/T0-01-ci-and-toolchain.md) | CI green and reproducible | — | human + agent | done |
| [T0-02](stage-0/T0-02-core-job-scheduler.md) | Job scheduler with priorities and cancellation | T0-01 | agent | done |
| [T0-03](stage-0/T0-03-world-palette-section.md) | 16³ section with palette | T0-01 | agent | done |
| [T0-04](stage-0/T0-04-mesh-chunk-32.md) | Gather, incremental masks and fast_mesh for the 32³ mesh chunk | T0-03 | agent | done |
| [T0-05](stage-0/T0-05-mesh-render-batches.md) | Render batches of 2×2×2 mesh chunks | T0-04 | agent | done |
| [T0-06](stage-0/T0-06-render-capability-probe.md) | GPU capability probe and vertex pulling prototype | T0-04 | agent writes, human runs on hardware | done |
| [T0-07](stage-0/T0-07-wit-and-modhost-spike.md) | WIT 0.1 and mod host spike | T0-01 | agent | done |
| [T0-08](stage-0/T0-08-light-incremental-bfs.md) | Incremental light add/remove | T0-03 | agent | done |
| [T0-09](stage-0/T0-09-sim-region-prototype.md) | Server regions prototype | T0-02 | agent | done |
| [T0-10](stage-0/T0-10-human-legal-and-azure.md) | Lawyer and Minecraft API access | — | human (not an agent) | in-progress (waiting for answers) |
| [T0-11](stage-0/T0-11-net-quic-spike.md) | QUIC transport spike | T0-01 | agent | done |
| [T0-12](stage-0/T0-12-worldgen-test-corpus.md) | Deterministic test terrain | T0-03 | agent | done |
| [T0-13](stage-0/T0-13-modhost-interruption-latency.md) | Mod interruption latency: split criterion, diagnose overrun | T0-07 (open question 11 answered) | agent | done |
| [T0-15](stage-0/T0-15-net-flaky-test.md) | Stabilize the flaky QUIC spike test | T0-11 | agent (cloud) | done |
| [T0-16](stage-0/T0-16-bare-metal-benchmarks.md) | Bare-metal confirmation of VM-only benchmarks | T0-03/04/05/08/12 | agent on the owner's laptop | todo |
| [T0-17](stage-0/T0-17-stage-0-exit-and-stage-1-backlog.md) | Stage 0 exit review and stage 1 backlog | T0-15, T0-16 | agent prepares, owner approves | todo |
| [T0-14](stage-0/T0-14-gpu-hardware-evidence.md) | GPU hardware evidence for T0-06 (Vega 10 + GTX 1660 Ti) | T0-06 implementation | agent + human runs on the laptop | done |

Recommended order for a single agent: T0-01 → T0-03 → T0-12 → T0-04 → T0-05 → T0-08 → T0-02 → T0-09 → T0-07 → T0-13 → T0-11 → T0-06 → T0-14.

### Remaining stage 0: order and parallel work (2026-11-21)

Done: T0-01…T0-09, T0-11…T0-15. Open: T0-10 (waiting for external answers, does not block engine work), T0-16, T0-17.

| Wave | Tasks that can run in parallel | Where | Waits for |
| --- | --- | --- | --- |
| 1 | **T0-15** flaky QUIC test — done | cloud | — |
| 1 | **T0-16** bare-metal benchmarks | owner's laptop, local agent | — |
| 2 | **T0-17** stage 0 exit review, then stage 1 backlog | cloud + owner decision | T0-16 |
| — | **T0-10** answers from the lawyer and Microsoft | human | external; interim decision recorded in open questions 4 and 5 |

### Carry-over for stage 1 (from stage 0 results)

Turn these into stage 1 tasks; do not drop them:

- Renderer backend selection with fallback when Vulkan enumerates no adapters (AMD switchable-graphics laptops), adapter choice by type/capability rather than index, and a GL shutdown test on AMD iGPU — T0-14, [meshing-and-rendering](../docs/architecture/meshing-and-rendering.md).
- Re-measure B vs C (indexed B variant, real terrain corpus) and decide the iGPU default — open question 13.
- CPU command recording: decide the statistic (open question 14); single frames reached 1.32 ms on Vega 10 + DX12.
- Bare-metal confirmation of VM-only numbers from T0-03/T0-04/T0-05/T0-08 — now task T0-16 (stage 0).
- Bare-metal re-run of the T0-11 QUIC spike with a shaped link (bandwidth cap + jitter, `tc netem`), and BBR fairness on a shared congested bottleneck — [net-quic-spike-2026-11-19](../docs/research/net-quic-spike-2026-11-19.md).
- GTX 1050 targets remain unverified (open question 12).
