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
| [T0-06](stage-0/T0-06-render-capability-probe.md) | GPU capability probe and vertex pulling prototype | T0-04 | agent writes, human runs on hardware | todo |
| [T0-07](stage-0/T0-07-wit-and-modhost-spike.md) | WIT 0.1 and mod host spike | T0-01 | agent | todo |
| [T0-08](stage-0/T0-08-light-incremental-bfs.md) | Incremental light add/remove | T0-03 | agent | todo |
| [T0-09](stage-0/T0-09-sim-region-prototype.md) | Server regions prototype | T0-02 | agent | todo |
| [T0-10](stage-0/T0-10-human-legal-and-azure.md) | Lawyer and Minecraft API access | — | human (not an agent) | todo |
| [T0-11](stage-0/T0-11-net-quic-spike.md) | QUIC transport spike | T0-01 | agent | todo |
| [T0-12](stage-0/T0-12-worldgen-test-corpus.md) | Deterministic test terrain | T0-03 | agent | todo |

Recommended order for a single agent: T0-01 → T0-03 → T0-12 → T0-04 → T0-05 → T0-08 → T0-02 → T0-09 → T0-07 → T0-11 → T0-06.

## Next stages

Stage 1 tasks are created at the end of stage 0, once the benchmark numbers are confirmed (stage 0 exit criterion).
