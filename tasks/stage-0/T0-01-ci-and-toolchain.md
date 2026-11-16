# T0-01. CI green and reproducible

- **Status:** done
- **Depends on:** —
- **Owner:** human + agent
- **PR:** closed in [#3](https://github.com/Arburano/Minecraft-rust/pull/3) (status bookkeeping; the CI work itself landed earlier)
- **Evidence:** [CI run #3 on `main`](https://github.com/Arburano/Minecraft-rust/actions/runs/43750180060) (commit `4257ec8`) — green; reproduced locally on 2026-11-15 (fmt, clippy with `-D warnings`, test, `bench --no-run`, all pass; sandbox toolchain stable 1.100, the pinned 1.97 was not downloadable there — `rust-toolchain.toml` unchanged).

## Goal

Make sure GitHub Actions passes on `main`: fmt, clippy with `-D warnings`, test, `bench --no-run`. The toolchain is pinned in `rust-toolchain.toml` (1.97).

## Specs

- [docs/process/testing-and-benchmarks.md](../../docs/process/testing-and-benchmarks.md)
- [docs/process/coding-standards.md](../../docs/process/coding-standards.md)

## Deliverables

- First CI run is green.
- CI badge in README.

## Acceptance criteria

- [x] Workflow `.github/workflows/ci.yml` passes on a clean clone.
- [x] `cargo test --workspace` gives the same result locally and in CI.

## Out of scope

Do not add new checks that are not in coding-standards.md.
