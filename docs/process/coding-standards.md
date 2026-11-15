# Coding standards

## Language

- Everything in the repository — code, identifiers, comments in code, commit messages, documentation in `docs/`, tasks — is written in English. Names of crates, types and functions in documents are as in the code.

## Rust

- Edition 2024, toolchain from `rust-toolchain.toml`. `cargo fmt` and `cargo clippy --workspace --all-targets` without warnings (CI has `-D warnings`).
- `unsafe` is forbidden at the workspace level. Exception: a module with `#![allow(unsafe_code)]`, a `// SAFETY:` comment next to each block and a reference to an ADR. Candidates: quad packing, SIMD, FFI.
- Errors: `thiserror` in libraries, `anyhow` only in binaries and tests. No `unwrap()` in library code outside tests (the `unwrap_used` lint); `expect("reason")` — only for invariants that cannot be violated.
- A panic in the engine is a bug. The server does not panic on input from the network or from a mod.
- Public items have `///` documentation (the `missing_docs` lint).

## Performance

- The hot loop (tick, meshing, lighting, render frame) — no allocations: pools, arenas, `Vec::clear()` instead of a new `Vec`.
- No `Mutex`/`RwLock` around the world or a region. Ownership, channels, snapshots. A lock is acceptable for rare global structures (the registry at load time).
- Every optimization that complicates the code comes with a before and after benchmark in the PR description.
- Do not add SIMD, `unsafe`, mimalloc, PGO without a measured gain (thresholds — `docs/research/optimization-analysis.md` §13.11).

## Dependencies

- Versions — only in `[workspace.dependencies]` of the root `Cargo.toml`. A crate pulls in `foo = { workspace = true }`.
- A new dependency: check the license (MIT/Apache-2.0/BSD/Zlib — ok; GPL — no), maintenance activity, tree size. A heavy dependency (> 50 transitive crates) — justify in the PR.
- `bevy_ecs` and `wgpu` are updated on schedule, not on the fly (risk from PRD §10).

## Architectural boundaries

- The direction of dependencies between crates is as in [`../architecture/overview.md`](../architecture/overview.md). A violation — through an ADR.
- Engine code does not know specific blocks (`stone`, `zombie`). Everything specific is in `mods/core`.
- Mechanics are not copied from decompiled Mojang code ([`../legal.md`](../legal.md)).
