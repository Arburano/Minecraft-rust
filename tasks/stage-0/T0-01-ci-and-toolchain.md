# T0-01. CI green and reproducible

- **Status:** todo
- **Depends on:** —
- **Owner:** human + agent

## Goal

Make sure GitHub Actions passes on `main`: fmt, clippy with `-D warnings`, test, `bench --no-run`. The toolchain is pinned in `rust-toolchain.toml` (1.97).

## Specs

- [docs/process/testing-and-benchmarks.md](../../docs/process/testing-and-benchmarks.md)
- [docs/process/coding-standards.md](../../docs/process/coding-standards.md)

## Deliverables

- First CI run is green.
- CI badge in README.

## Acceptance criteria

- [ ] Workflow `.github/workflows/ci.yml` passes on a clean clone.
- [ ] `cargo test --workspace` gives the same result locally and in CI.

## Out of scope

Do not add new checks that are not in coding-standards.md.
