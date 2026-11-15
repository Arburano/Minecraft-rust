# T0-07. WIT 0.1 і спайк хоста модів

- **Статус:** todo
- **Залежить від:** T0-01
- **Виконавець:** агент

## Мета

Довести `wit/engine-api.wit` до 0.1: Rust-гість через `wit-bindgen` (target `wasm32-wasip2`) у `mods/examples/hello`, хост у `modhost` через `wasmtime::component::bindgen!`. Виміряти: порожній `run-system`, `on-events` з 1 000 подій (`list<event>` з рядками), `set-blocks` на 4 000 змін; перевірити epoch-переривання нескінченного циклу і відкидання пачки змін.

## Специфікації

- [docs/modding/wasm-api.md](../../docs/modding/wasm-api.md)
- [docs/modding/overview.md](../../docs/modding/overview.md)
- [docs/adr/0006-wasm-component-model-epoch.md](../../docs/adr/0006-wasm-component-model-epoch.md)

## Результат

- Робочий приклад мода і хоста, інтеграційний тест у `crates/modhost/tests/`.
- Бенчмарк і числа в performance-budgets.md.
- Оновлений WIT (валідний `wasm-tools component wit`).

## Критерії приймання

- [ ] Нескінченний цикл у моді переривається ≤ 1 мс після дедлайну, сервер продовжує тік (тест).
- [ ] Зміни світу перерваного виклику не застосовані (тест).
- [ ] Диспетчеризація хука ≤ 1 мкс, ≤ 5 нс на подію (або задокументоване відхилення).

## Поза межами задачі

Без дозволів, реєстру модів і гарячого перезавантаження — етап 3.
