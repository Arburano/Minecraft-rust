# T4-01. Engine capabilities per build

- **Status:** done (cloud; report: [t4-01-capabilities](../../task-reports/t4-01-capabilities.md))
- **Depends on:** T1-24
- **Owner:** agent (cloud)

## Goal

[ADR-0018](../../docs/adr/0018-engine-capabilities-per-build.md): the engine starts only the optional features that the build's mods require.

## Deliverables

- `mod.toml` `[requires] capabilities = [...]`, parsed by `world::modset`; the build's capability set is the union; unknown names are load errors.
- A `Capabilities` value passed to the server and client; subsystems start only when present (a test subsystem proves that an absent capability adds no thread, system or message).
- A cargo feature per heavy capability (first: `moving_grids`); a binary without a required feature fails at load with a clear message.
- Worlds record the capabilities their saved data needs (`server.toml`); a build without them refuses the world.
- CI: `cargo test --workspace` with all features and with none.
- Docs: `mod-manifest.md`, `builds.md`.

## Acceptance criteria

- [x] A build whose mods require nothing starts no optional subsystem (test: `server/tests/capabilities.rs`).
- [x] The error messages name the build, the mod and the capability.
