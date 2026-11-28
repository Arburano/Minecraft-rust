# Mods: overview

Crates: `modhost` (host in the engine), `mod-sdk` (library for authors). ADR: 0006. Contract: [`../../wit/engine-api.wit`](../../wit/engine-api.wit).

## Three levels

| Level | What | Who | Cost |
| --- | --- | --- | --- |
| 1. Datapack | TOML/JSON + assets: blocks, items, recipes, loot, biomes, structures, zones, models, sounds, localization | designers, artists | zero at runtime (compiled into registries) |
| 2. WASM mod | a WebAssembly component: systems, AI, mechanics, commands, UI, generation stages | modders | ≈ 0.4 µs per component call; batched APIs are mandatory |
| 3. Native plugin | a Rust crate in the workspace: render passes, world formats, physics | team only | same as the engine |

Third-party native `.dll`/`.so` are not supported: there is no stable ABI and no sandbox.

## How the engine runs WASM mods

- One `wasmtime::Engine` per process; a `Component` is compiled once and cached on disk.
- `InstancePre` + pooling allocator: a mod instance **per worker thread** (not per region: otherwise 1–15 GB). A Store cannot be called from two threads at once.
- **Mod state lives in the host** (ECS components via the `components` interface). Linear memory is scratch for a single call. Therefore hot reload and moving between threads do not lose state.
- Hooks are batched: `on-events(list<event>)` once per tick per event type; `run-system(index)` once per tick per system (or once per N ticks).
- World changes (`set-blocks`) are applied only if the call finished successfully. An interruption (epoch, panic, budget overrun) → the whole batch is discarded.
- Epoch interruption is always on (0–9% cost). Fuel — only in `engine-cli test` and replays (+50–90%).
- Access only to its own region; for anything far away — `send-to`. Global data is a snapshot of the previous tick.

## Execution side

`side = server | client | both` in the manifest. The client never receives a server `.wasm`. Component replication (`replicate = true`) is only for `both` mods; the schema is part of the handshake hash.

## Security

- Permissions in the manifest; without a permission there is no import (the component is not linked).
- Client mods have no network or files by default.
- Mods of public servers are signed with ed25519; the client verifies the signature and SHA-256.
- A linear memory limit per mod; a per-tick time limit (soft — warning, hard — epoch).

## Documents

- [builds.md](builds.md) — builds (sets of mods), the base-game modules and the rules for cutting them (ADR-0017).
- [movement-profiles.md](movement-profiles.md) — the movement slot: profile numbers and abilities.
- [mod-manifest.md](mod-manifest.md) — `mod.toml`.
- [datapacks.md](datapacks.md) — data format.
- [wasm-api.md](wasm-api.md) — the WIT contract and API evolution rules.
- [writing-mods.md](writing-mods.md) — guide for the mod author.
- [performance-rules.md](performance-rules.md) — rules for fast mods.
