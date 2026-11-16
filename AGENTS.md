# AGENTS.md

Instructions for AI agents (Claude Code, Codex, Cursor, etc.) and humans working in this repository.

## What this project is

A custom voxel engine in Rust in the style of Minecraft: ECS, multithreading by default, native mods on WebAssembly, a server with regions and shards. It is **not** a port of Minecraft and **not** a client for vanilla servers (ADR-0001). All gameplay is the `mods/core` mod on the public API.

Decision priority (PRD §14): when an optimization conflicts with game feel, feel wins. Smoothness without stutters, the Minecraft feel, smart mobs, full render distance of no less than 8 chunks.

## Before any work

1. Read [`docs/README.md`](docs/README.md) and [`docs/architecture/overview.md`](docs/architecture/overview.md).
2. Find your task in [`tasks/`](tasks/README.md). The current stage is **0** ([`docs/process/roadmap.md`](docs/process/roadmap.md)).
3. Read the specifications the task refers to and the relevant ADRs in [`docs/adr/`](docs/adr/README.md).
4. Do not start a task if its dependencies are not `done`.

## Commands

```bash
cargo fmt --all                                 # formatting
cargo clippy --workspace --all-targets          # lints; CI runs with RUSTFLAGS="-D warnings"
cargo test --workspace                          # tests
cargo bench -p <crate>                          # benchmarks (criterion)
cargo bench --workspace --no-run                # check that benchmarks compile (runs in CI)
wasm-tools component wit wit/engine-api.wit     # validate the mod contract
```

The toolchain is pinned in `rust-toolchain.toml`. If rustup cannot download it (closed network), temporarily use `RUSTUP_TOOLCHAIN=stable`, but do not change the file without an ADR.

Benchmarks from the pre-code stage live in a separate workspace `research/bench` and are not part of the main build.

## Structure

```text
crates/        engine crates (see the table in docs/architecture/overview.md)
wit/           engine:api mod contract (WIT)
mods/core/     base gameplay as a mod
docs/          PRD, specifications, ADRs, process, research
tasks/         task backlog by stage
task-reports/  report after each completed task (what was done, what went wrong, issues)
research/      benchmarks that justified decisions
```

## Rules that must not be broken

1. **Legal** ([`docs/legal.md`](docs/legal.md)): no code decompiled from Minecraft; no Mojang assets in the repository, tests, CI or builds. The game and all tests run on fallback assets. Do not write "Minecraft" in crate names, types and UI.
2. **Architectural boundaries**: the direction of dependencies between crates is as in overview.md. The engine does not know specific blocks and mobs — they are in `mods/core`. Region code does not read another region (ADR-0008). Mods only through the WIT contract (ADR-0006).
3. **Performance**: no allocations in the hot loop; no `Mutex`/`RwLock` around the world; optimizations come with a before/after benchmark; numbers live in [`docs/process/performance-budgets.md`](docs/process/performance-budgets.md).
4. **Quality**: `unsafe` is forbidden (exception — a module with `// SAFETY:` and an ADR); no `unwrap()` in libraries; public API is documented; new logic comes with tests, an optimized implementation comes with a reference one for comparison.
5. **Dependencies**: versions only in `[workspace.dependencies]`; a new heavy dependency comes with a justification in the PR.
6. **Decisions**: do not silently revisit accepted ADRs. If you disagree, write a new ADR with status "Proposed" and ask the owner. Product questions go in [`docs/open-questions.md`](docs/open-questions.md); until answered, use the "For now" value from there and mark the code `// OPEN-QUESTION: <number>`.
7. **Honest numbers**: do not invent benchmark results. State the machine and the number of runs; mark numbers from a VM as such.

## How to close a task

1. Branch `t0-03-world-palette-section` (task id + slug).
2. Code + tests + benchmark (if the task has numbers) + updated documents (specification, performance-budgets, open-questions — whatever changed).
3. Task status `done` in the task file and in `tasks/README.md`.
4. Write a task report in [`task-reports/`](task-reports/README.md) (see "Task reports" below).
5. Commit message in English, in the format `T0-03: implement palette section storage`. One logical step — one commit.
6. A PR with a description: what was done, before/after numbers, what remains. CI must be green.

## Task reports

After completing **every** task, write a report to the `task-reports/` folder before opening the PR. The report is mandatory even if everything went smoothly.

- File name: `<task-id>-<slug>.md`, e.g. `task-reports/t0-03-world-palette-section.md`.
- Written in English, like everything else in the repository.
- Required sections:
  1. **What was done** — a short summary of the implementation, files/crates touched, tests and benchmarks run (with honest numbers, per rule 7).
  2. **What went wrong** — mistakes, failed approaches, reverted changes, and how they were resolved. Write "None" if there were none.
  3. **Issues encountered** — blockers, deviations from the specification or ADRs, open questions raised, and anything left unfinished or needing follow-up.

The report is committed together with the task's changes.

## Language

Everything in the repository — code, comments in code, commits, documentation in `docs/`, tasks — is written in English.

## When to stop and ask a human

- The task requires breaking one of the rules above.
- A specification contradicts an ADR or another specification.
- Credentials, keys, access to hardware (GPU, servers) or a legal decision are needed.
- A benchmark shows that a target from performance-budgets is unreachable with the current architecture.
