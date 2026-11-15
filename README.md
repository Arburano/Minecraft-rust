# Minecraft-rust

[![CI](https://github.com/Arburano/Minecraft-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/Arburano/Minecraft-rust/actions/workflows/ci.yml)

A custom voxel engine in Rust in the style of Minecraft: fast on low-end PCs, fully moddable through WebAssembly mods, with a server that scales to thousands of players.

> Working name of the repository. The project is not an official Minecraft product and is not affiliated with Mojang or Microsoft. Mojang assets are not part of the repository; the player downloads them after signing in through Microsoft.

**Status:** stage 0 — research and prototypes. The code is a crate skeleton; documentation and backlog are ready for development to start.

## Key decisions

- A custom engine, not a port; gameplay is the `core` mod on the public API ([ADR-0001](docs/adr/0001-own-engine-not-port.md)).
- `bevy_ecs` + a custom `wgpu` renderer with three paths for different GPUs ([ADR-0002](docs/adr/0002-bevy-ecs-only.md), [ADR-0003](docs/adr/0003-wgpu-own-renderer.md)).
- 16³ sections with a palette, 32³ mesh chunks, 64³ render batches ([ADR-0004](docs/adr/0004-sections-16-palette.md), [ADR-0005](docs/adr/0005-mesh-chunk-32-with-batches.md)).
- Mods — WASM Component Model with batched hooks ([ADR-0006](docs/adr/0006-wasm-component-model-epoch.md)).
- QUIC, region threading, shards with an ownership number ([ADR-0007](docs/adr/0007-quic-own-protocol.md), [ADR-0008](docs/adr/0008-region-threading.md), [ADR-0009](docs/adr/0009-sharding-ownership-number.md)).
- The launcher is the game's main menu + a tiny updater ([ADR-0010](docs/adr/0010-in-game-launcher-and-updater.md)).

## Documentation

- [PRD](docs/PRD.md) — product, goals, metrics, risks, plan.
- [docs/README.md](docs/README.md) — map of specifications.
- [Roadmap](docs/process/roadmap.md) and [tasks](tasks/README.md).
- [AGENTS.md](AGENTS.md) — rules for developers and AI agents.

## Build

```bash
cargo build --workspace
cargo test --workspace
```

Rust 1.97 is required (see `rust-toolchain.toml`).
