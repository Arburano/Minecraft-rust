# T0-01. CI green and reproducible

- **Status:** done
- **Depends on:** —
- **Owner:** human + agent
- **PR:** [#2](https://github.com/Arburano/Minecraft-rust/pull/2)

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

## Verification

All four CI steps pass with the pinned toolchain (rustc 1.97.1, Linux x86_64), both on a
fresh `git clone` without a build cache and in GitHub Actions
([run 43750957998](https://github.com/Arburano/Minecraft-rust/actions/runs/43750957998),
conclusion: success): `fmt --all --check`, `clippy --workspace --all-targets` with
`-D warnings`, `test --workspace` (6 passed, 0 failed: 3 in `engine-core`, 3 in `world`),
`bench --workspace --no-run`. No workflow or code changes were needed; the CI badge was
already in the README.

## Out of scope

Do not add new checks that are not in coding-standards.md.
