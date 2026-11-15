# T0-01. CI зелений і відтворюваний

- **Статус:** todo
- **Залежить від:** —
- **Виконавець:** людина + агент

## Мета

Переконатися, що GitHub Actions проходить на `main`: fmt, clippy з `-D warnings`, test, `bench --no-run`. Тулчейн відкріплений у `rust-toolchain.toml` (1.97).

## Специфікації

- [docs/process/testing-and-benchmarks.md](../../docs/process/testing-and-benchmarks.md)
- [docs/process/coding-standards.md](../../docs/process/coding-standards.md)

## Результат

- Перший прогін CI зелений.
- Бейдж CI у README.

## Критерії приймання

- [ ] Workflow `.github/workflows/ci.yml` проходить на чистому клоні.
- [ ] `cargo test --workspace` локально і в CI дає однаковий результат.

## Поза межами задачі

Не додавати нових перевірок, яких немає в coding-standards.md.
