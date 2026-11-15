# Minecraft-rust

[![CI](https://github.com/Arburano/Minecraft-rust/actions/workflows/ci.yml/badge.svg)](https://github.com/Arburano/Minecraft-rust/actions/workflows/ci.yml)

Власний воксельний рушій на Rust у стилі Minecraft: швидкий на слабких ПК, повністю модифікований через WebAssembly-моди, з сервером, що масштабується на тисячі гравців.

> Робоча назва репозиторію. Проєкт не є офіційним продуктом Minecraft і не пов'язаний з Mojang або Microsoft. Асети Mojang не входять до репозиторію; гравець завантажує їх сам після входу через Microsoft.

**Статус:** етап 0 — дослідження і прототипи. Код — каркас крейтів; документація і беклог готові для старту розробки.

## Ключові рішення

- Власний рушій, а не порт; геймплей — мод `core` на публічному API ([ADR-0001](docs/adr/0001-own-engine-not-port.md)).
- `bevy_ecs` + власний рендер на `wgpu` з трьома шляхами під різні GPU ([ADR-0002](docs/adr/0002-bevy-ecs-only.md), [ADR-0003](docs/adr/0003-wgpu-own-renderer.md)).
- Секції 16³ з палітрою, меш-чанки 32³, рендер-пакети 64³ ([ADR-0004](docs/adr/0004-sections-16-palette.md), [ADR-0005](docs/adr/0005-mesh-chunk-32-with-batches.md)).
- Моди — WASM Component Model з пакетними хуками ([ADR-0006](docs/adr/0006-wasm-component-model-epoch.md)).
- QUIC, регіональна багатопотоковість, шарди з номером власності ([ADR-0007](docs/adr/0007-quic-own-protocol.md), [ADR-0008](docs/adr/0008-region-threading.md), [ADR-0009](docs/adr/0009-sharding-ownership-number.md)).
- Лаунчер — це головне меню гри + крихітний апдейтер ([ADR-0010](docs/adr/0010-in-game-launcher-and-updater.md)).

## Документація

- [PRD](docs/PRD.md) — продукт, цілі, метрики, ризики, план.
- [docs/README.md](docs/README.md) — карта специфікацій.
- [Роадмап](docs/process/roadmap.md) і [задачі](tasks/README.md).
- [AGENTS.md](AGENTS.md) — правила для розробників і AI-агентів.

## Збірка

```bash
cargo build --workspace
cargo test --workspace
```

Потрібен Rust 1.97 (див. `rust-toolchain.toml`).
