# T1-15. Stage 1 exit measurement

- **Status:** todo
- **Depends on:** T1-01 … T1-14
- **Owner:** agent writes the harness, owner's laptop runs it

## Goal

Measure the stage 1 exit criterion and the remaining client hypothesis: 60 FPS at 12 chunks on the iGPU (Vega 10, open question 12) with 1% low ≥ 45, RAM ≤ 1.5 GB without LOD (internal budget 1 GiB — the "Client RAM" hypothesis), 30 minutes of walking without frame spikes > 33 ms, start to menu ≤ 5 s.

## Specs

- [roadmap](../../docs/process/roadmap.md) — stage 1 exit
- [performance-budgets.md](../../docs/process/performance-budgets.md)
- [stage 0 exit review](../../docs/research/stage-0-exit-2026-11-21.md)

## Deliverables

- Scripted walk/fly route through a fixed-seed world; frame-time log, RAM samples, spike detector.
- Run instructions for the owner's laptop (like T0-14); results in `docs/research/stage-1-exit-YYYY-MM-DD/`.
- GTX 1660 Ti run as additional data; GTX 1050 targets stay unverified (open question 12).

## Acceptance criteria

- [ ] 3 runs on the Vega 10 with all exit numbers reported; misses go to the owner (AGENTS.md).
- [ ] Client RAM row moves from Hypothesis to Measured.
- [ ] Stage 1 exit decision recorded by the owner.

## Out of scope

Network, LOD, mods.
