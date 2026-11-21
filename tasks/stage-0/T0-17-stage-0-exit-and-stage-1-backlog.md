# T0-17. Stage 0 exit review and stage 1 backlog

- **Status:** in-progress (part A done 2026-11-21: [exit review](../../docs/research/stage-0-exit-2026-11-21.md); waiting for the owner's exit decision before part B)
- **Depends on:** T0-15, T0-16 (T0-10 may still be waiting for external answers — see below)
- **Owner:** agent (cloud) prepares, owner approves

## Goal

Check the stage 0 exit criterion (`docs/process/roadmap.md`: "all numbers from performance-budgets confirmed or adjusted; ADRs recorded") and turn stage 1 into concrete tasks.

## Part A — exit review

1. Go through **every** row of `docs/process/performance-budgets.md` and classify it: Measured (bare metal / VM), Target for a later stage, or still Hypothesis. As of 2026-11-21 these are still Hypothesis with no stage 0 measurement: client RAM at 12 chunks, network CPU per 10,000 packets/s, node RAM for 500 players, cold start of a mod with cache. For each: measure now with a small benchmark if cheap (e.g. mod cold start with `modhost`), or propose to the owner to move it to the stage where it becomes measurable (and record that in the row). Do not change targets silently.
2. Check that every decision taken in stage 0 is recorded: ADRs, open questions 11–14, carry-over list. If a stage 0 result contradicts an accepted ADR, write a new ADR with status "Proposed" (AGENTS.md rule 6).
3. Write `docs/research/stage-0-exit-2026-MM-DD.md`: table of budget rows with status and evidence links, open items, and the recommendation "exit stage 0: yes/no".
4. T0-10 (lawyer, Microsoft API access) does not block engine work: open questions 4 and 5 have interim owner decisions (2026-11-21). List it as an open external dependency, not as a blocker.

## Part B — stage 1 backlog (after the owner agrees to exit)

Create `tasks/stage-1/T1-NN-<slug>.md` files with the same template (goal, specs, deliverables, acceptance criteria with numbers, out of scope) and a `## Stage 1` table in `tasks/README.md` with dependencies and a recommended order / parallel waves. Sources: PRD §9 "Stage 1", `docs/process/roadmap.md` (including "What was added from the review"), and the "Carry-over for stage 1" list. Expected work areas (split further where needed):

- ECS core and tick phases on the client; task pool integration (T0-02).
- World storage and saving (sections, light, columns on disk; format version + golden files).
- Terrain generator pipeline (beyond the T0-12 test corpus) and initial lighting in 2×2 column groups; light integrated with the world (import of stored light, per-tick limits).
- Loading queue with priorities (direction of movement, height, speed).
- Renderer: real mesh upload, culling, texture arrays, quality profiles, backend/adapter selection with fallback (carry-over), B vs C decision (open question 13), CPU recording statistic (open question 14).
- Player physics, digging and placing blocks.
- Assets: loader with local cache + fallback asset set; Mojang asset import on the player's machine, and reading a developer's own licensed installation for local tests (open question 5, legal.md rule 2; never in repo/CI/builds).
- Offline mode for client and server (no Microsoft login; legal.md rule 7, auth-and-assets.md); Microsoft login itself once T0-10 is answered.
- Menu, cold start ≤ 5 s, lazy asset loading.
- CI with benchmarks and Tracy tracing.
- Stage 1 exit measurement harness: 60 FPS on iGPU at 12 chunks (1% low ≥ 45), RAM ≤ 1.5 GB, 30 min without spikes > 33 ms (on the Vega 10 per open question 12).

Then set "Current stage: 1" in `docs/process/roadmap.md`.

## Acceptance criteria

- [ ] Exit review document with every budget row classified ([done](../../docs/research/stage-0-exit-2026-11-21.md)); owner decision recorded (pending).
- [ ] Stage 1 task files and table created; every carry-over item is covered by a task.
- [ ] Roadmap current stage updated; report `task-reports/t0-17-stage-0-exit-and-stage-1-backlog.md`.
