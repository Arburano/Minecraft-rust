# Як писати моди

Цей гайд для авторів модів. Статус: SDK і `engine-cli` ще будуються (етап 3); приклади показують цільовий API.

## 1. Почніть з даних

Більшість контенту — датапак: блоки, предмети, рецепти, лут, біоми, зони. Код потрібен лише для поведінки, яку даними не описати. Див. [datapacks.md](datapacks.md).

## 2. Створіть мод

```bash
engine-cli new copper_pipes      # шаблон: mod.toml, data/, assets/, src/lib.rs
cd copper_pipes
engine-cli dev                   # локальний сервер + клієнт, гаряче перезавантаження даних, асетів і .wasm
```

## 3. Поведінка кодом (Rust → wasm32-wasip2)

```rust
use engine_mod_sdk::prelude::*;

#[derive(Component, Default)]
#[replicate]                      // лише для side = "both"
struct Flow { amount: u16 }

#[mod_init]
fn init(app: &mut ModApp) {
    app.component::<Flow>("copper_pipes:flow")
       .system("flow", Phase::Simulate, flow_system)   // доступ виводиться з сигнатури
       .every_n_ticks(2)
       .on::<BlockPlaced>(on_placed);
}

fn flow_system(region: &mut Region, pipes: Query<(&BlockPos, &mut Flow)>) {
    let mut changes = Vec::new();
    for (pos, mut flow) in pipes {
        let below = pos.down();
        if region.contains(below) {
            changes.push((below, flow.amount / 2));
        } else {
            region.send_to(below, "copper_pipes:transfer", &(flow.amount / 2));
        }
        flow.amount /= 2;
    }
    region.apply_flow_batch(&changes);    // один виклик хоста на систему
}
```

Що тут важливо:

- Стан (`Flow`) живе в хості, а не в статичних змінних мода.
- Сусідній регіон — лише через `send_to`; відповідь прийде в наступному тіку.
- Зміни — пакетом.

## 4. Тести

```bash
engine-cli test       # headless-сервер: фіксований seed, N тіків, перевірки стану; fuel замість epoch → повторювано
engine-cli bench      # час мода на тік і пам'ять; падає, якщо вихід за [budget]
```

## 5. Публікація

```bash
engine-cli publish    # перевірка, збирання, підпис ed25519, завантаження в реєстр
```

## 6. Налагодження продуктивності

- `/profile mods` на сервері — час кожного мода на тік по регіонах.
- Правила — [performance-rules.md](performance-rules.md).

## Інші мови

Будь-яка мова з підтримкою WASI 0.2 Component Model (C, Zig, Go через TinyGo, AssemblyScript) за тим самим WIT. Офіційний SDK — Rust; друга мова — етап 7.
