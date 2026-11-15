# research/

Research code that backed the decisions in the ADRs. Not part of the main workspace.

## bench/

A separate Cargo workspace with the benchmarks from the "before code" stage (run on a VM with 2 vCPU Xeon 2.1 GHz, noise ±30%):

| Crate | What it measures | Result |
| --- | --- | --- |
| `meshing` | `binary-greedy-meshing` 0.6.4 on a 62³ unit, naive mesher, rayon | `docs/research/optimization-tests.md` §1 |
| `palette` | palette sections 16³: size, get/set | §2 |
| `wasmbench` | wasmtime 50: core and component calls, batches, epoch, fuel | §3 |
| `light` | BFS light, 16³ and 3×3×3 | §4 |
| `zstdbench` | zstd on synthetic columns (data is unrealistic) | §5 |

```bash
cd research/bench && cargo run --release -p meshing
```

Raw outputs are in `bench/results/`. The `/home/claude/...` paths in the `docs/research/` documents are where the benchmarks were run during the research; in the repository they correspond to `research/bench/` and `docs/research/`.

Working-code crates (`crates/world`, `crates/mesh`, `crates/light`) may take code from here as a starting point (tasks T0-03, T0-04, T0-08), but they do not depend on this workspace.
