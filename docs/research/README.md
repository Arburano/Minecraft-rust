# Research

Materials from which the PRD and ADRs grew (November 2026).

| File | What it is | Language |
| --- | --- | --- |
| [optimization-search.md](optimization-search.md) | Web research on voxel game optimization approaches, with references | English |
| [optimization-tests.md](optimization-tests.md) | Benchmark results from `research/bench` (VM 2 vCPU, ±30%) | English |
| [optimization-analysis.md](optimization-analysis.md) | Analysis and recommendations (the same as PRD §13) | English |
| [job-scheduler-2026-11-15.md](job-scheduler-2026-11-15.md) | T0-02 job scheduler overhead (VM 4 vCPU, ±30%) | English |
| [review-budget.py](review-budget.py) | Reviewer calculations: memory, VRAM, traffic, login time | Python |

Rules:

- Put new measurements here as separate files `<topic>-<date>.md`, describing the machine, versions and methodology.
- GPU capability dumps (task T0-06) go in `gpu/`.
- If a new measurement changes a target, update `docs/process/performance-budgets.md` and the PRD.
