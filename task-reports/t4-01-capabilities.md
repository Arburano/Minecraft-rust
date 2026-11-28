# T4-01: engine capabilities per build

## What was done

Implements [ADR-0018](../docs/adr/0018-engine-capabilities-per-build.md). The engine starts only the optional features that the build's mods require.

**`world`**

- New module `world::capability`:
  - `Capability` holds the six stable names: `dimensions`, `moving_grids`, `space`, `zones`, `instances`, `progression`. `cargo_feature()` returns `moving_grids` for the one heavy capability.
  - `Capabilities` is a `Copy` bit set with union, difference, iteration, names and `Display` (`"dimensions, zones"` / `"none"`).
- `world::modset`:
  - `mod.toml` takes `[requires] capabilities = [...]`. The table uses `deny_unknown_fields`. An unknown name is a `Parse` error that names the mod and lists the known names.
  - `ModInfo::capabilities`, `ModSet::capabilities()` (the union over the mods) and `ModSet::requirer()`.
  - `ModSet::check_compiled(compiled, binary)` returns the new `ModSetError::CapabilityNotCompiled`. Message: "build X needs capability Y (required by mod Z); this server binary was built without it (cargo feature "Y")".

**`server`**

- Cargo features: `default = ["moving_grids"]` and `moving_grids = []`. The feature is empty for now; it will gate the rigid-body physics dependency in T4-03.
- New module `server::capabilities`:
  - `compiled()` gives the binary's capability set.
  - The `Subsystem` trait has `tick(tick)` and `shutdown()`.
  - `SubsystemRegistry` maps a capability to a factory. `builtin()` is empty, because no capability is implemented yet.
  - `Subsystems` holds the started set. It shuts down once, also on drop, so a world that fails after subsystems started still stops them.
  - `check_world()` checks the capabilities recorded for a world.
- `Server::start`:
  1. checks the compiled set first;
  2. checks `server.toml` `capabilities` and refuses a world whose saved capabilities the build lacks;
  3. starts one subsystem per capability of the build. A required capability without a factory is an error: "… which this engine version does not implement yet".
- The world thread ticks subsystems after the regions simulate. It skips the call entirely when the list is empty. Subsystems stop first on world shutdown.
- `ServerConfig::subsystems` holds the registry. `Server::capabilities()` and `World::capabilities()` return the running set.
- `server.toml` gains `capabilities = [...]`: the union of every build that opened the world, since data written once stays. Absent means none, so existing worlds open unchanged.

**`protocol`**

- `Welcome.capabilities: Vec<String>` lists the running capabilities by name, so a client can name one it does not know. At most 64 names are validated.
- `PROTOCOL_VERSION` is raised 4 → 5. The golden files `header.bin` and `welcome.bin` are regenerated, and the sample `Welcome` now carries two capabilities.

**`client`**

- Features `default = ["moving_grids"]` and `moving_grids = ["server/moving_grids"]`. The client depends on `server` with `default-features = false`, so its features decide the embedded server's set.
- `client::capabilities()` gives the client's set.
- `main` checks the build at start.
- `remote::check_server_capabilities` refuses a server whose capability the client does not know ("update the game") or was built without. It is called right after `Welcome`. New error variant: `RemoteError::Capability`.

**Tests**

- `world`: name round-trip, set operations, the union over mods, `requirer`, the not-compiled message (exact text), the unknown-name error, and a check that the shipped `classic` build requires nothing.
- `server` unit tests: classic starts no subsystem; `check_world` refuses unknown and missing capabilities; `compiled()` follows the feature.
- `server/tests/capabilities.rs` uses a test `zones` subsystem that starts a `cap-zones` thread and counts ticks.
  - **Classic build:** the factory is not called, there are 0 ticks, no `cap-zones` thread exists (read from `/proc/self/task`), and `Welcome.capabilities` is empty.
  - **Build with a `zones_test` mod:** the subsystem starts once, ticks with the world, its thread runs, `Welcome` lists `zones`, and it is shut down with the server.
  - **Reopening that world with classic** is refused, and the error names the build and the capability.
  - **An unimplemented capability** gives an error naming the build, the mod and the capability.
  - **`moving_grids`:** the expected error depends on the feature: "built without it" when the feature is off, "does not implement" when it is on.
- `client`: `check_server_capabilities` (known, not compiled, unknown).
- Commands:
  - `cargo test -p server --no-default-features --lib --test capabilities` passes (20 + 4);
  - so does the same run with default features;
  - `cargo clippy --workspace --all-targets` with `RUSTFLAGS=-D warnings` is clean;
  - `cargo fmt --all` was run;
  - `cargo test --workspace` passes: 345 tests in 79 suites, 0 failures, on a cloud VM with 4 vCPU and one run.
- **CI:** a new step `cargo test -p server --no-default-features --lib --test capabilities`. The normal `cargo test --workspace` step is the "all features" run, because `moving_grids` is a default feature.

**Docs:** `docs/modding/mod-manifest.md` (`[requires]` and its validation rule), `docs/modding/builds.md` ("How it works (T4-01)" and the "Not done yet" list) and `docs/architecture/networking-protocol.md` (`Welcome`, protocol version 5).

**Numbers:** none measured. When a build has no capabilities, the world tick only checks whether an empty `Vec` is empty. No benchmark was run because there is no subsystem to compare against yet.

## What went wrong

- The first background build reported errors because it had started before the edits were finished. A clean rebuild after the edits had no errors.
- `Subsystems::shutdown` first iterated without draining. A world that started its subsystems and then failed (for example, the world store did not open) would have left their threads running. Shutdown now drains the list, and `Drop` calls it.

## Issues encountered

- **Base branch.** The task depends on T1-24 (`world::modset`, builds), which exists only on the unmerged branch `ccr-b8153911-k3g0kc`. This work sits on top of that branch, not `main`. `main` itself is behind several unmerged branches, and open PRs #38 and #39 redo T2-01 and T2-02 separately. The owner needs to choose one line to merge.
- **"With none" in CI** is narrowed to `-p server --no-default-features`. A workspace-wide `--no-default-features` would also turn off `worldgen/builtin-overworld`, and the classic build cannot generate terrain without it. That is a different switch, already checked by its own CI step.
- **What a world records.** `server.toml` records every capability of every build that opened the world, not only the capabilities whose subsystem actually wrote data. This is conservative and matches ADR-0018 rule 4. A world opened once with a test build that requires `zones` can no longer be opened with `classic`. If that is too strict, subsystems could report "has data" in a later change.
- **Client-side subsystems** (a capability's render pass) and capability-gated WIT linking (ADR-0018 rule 5) are not done. They come with T4-06 and with the stage 3 mod API. They are listed in `builds.md`, "Not done yet".
- **Protocol 4 → 5:** any build of `main`, or of PRs #38 and #39, cannot talk to this branch. The version check reports this clearly.
