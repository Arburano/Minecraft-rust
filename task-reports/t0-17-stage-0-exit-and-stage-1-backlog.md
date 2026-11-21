# T0-17: stage 0 exit review and stage 1 backlog

## What was done

- **Part A, exit review** ([stage-0-exit-2026-11-21.md](../docs/research/stage-0-exit-2026-11-21.md)):
  - Every row of `performance-budgets.md` was classified: 14 met (5 of them VM-only), 5 deviations, 3 hypotheses, and the rest later-stage targets.
  - The decisions taken in stage 0 were checked against the ADRs; none contradicts an accepted ADR, so no new ADR is needed.
  - **New measurement:** the "Cold start of a mod with cache" hypothesis was measured as an upper bound without a cache. A new `modhost_cold_start` bench (compile + link + instantiate + `init` of the 127 KB hello component) gives 12.3–13.9 ms. That is a VM measurement (4 vCPU Xeon 2.1 GHz), 3 runs, 20 samples each, against a budget of ≤ 50 ms with a cache.
- **Owner decision:** exit stage 0 accepted as recommended (2026-11-21).
- **Part B, stage 1 backlog:**
  - 15 task files `tasks/stage-1/T1-01…T1-15`, covering PRD §9 stage 1, the roadmap additions (loading queue with priorities, lazy assets, start ≤ 5 s) and every stage 0 carry-over item.
  - A stage 1 table and parallel waves in `tasks/README.md`.
  - Carry-over items for stages 2 and 3: the QUIC shaped-link re-run, the network CPU and node RAM measurements, hook/event optimization and the mod compilation cache.
  - Roadmap and `AGENTS.md` now say the current stage is 1.
- Also in this branch:
  - T0-15 (flaky QUIC test) and the offline-mode / local-assets rules, which missed the merge of PR #19.
  - A merge of the T0-16 branch (PR #20) to have its results as input.

## What went wrong

- PR #19 was merged before its last two commits (T0-15 and the offline-mode rules) were pushed, so they were missing from `main`. Found while starting this task; they are carried in this branch's PR.
- The first draft of the review counted "17 met" rows; a recount gave 14. Fixed before committing.

## Issues encountered

- **Mod cold start** is measured without a cache and on a small component. The real budget, with a cache, needs the disk compilation cache. Precompiled-component deserialization in Wasmtime is an `unsafe` API, so this needs an ADR under AGENTS.md rule 4. It is noted as a stage 3 item.
- **Proposed numbers in T1 tasks** for which there is no budget yet (T1-10: nearest 3×3 ready ≤ 0.5 s, apply batch ≤ 2 ms; T1-01: map access within 2× of section access) need the owner's confirmation when those tasks start.
- **GTX 1050 targets** remain unverified (open question 12).
