# Маніфест мода (`mod.toml`)

```toml
[mod]
id = "copper_pipes"          # [a-z0-9_], унікальний; простір імен усіх ID мода
version = "0.3.1"            # semver
api = "^0.1"                 # версія engine:api (WIT-пакет)
side = "both"                # server | client | both
authors = ["Hi"]
description = "Мідні труби для рідин"
license = "MIT"

[dependencies]
core = "^0.1"
fluids = { version = "^0.5", optional = true }

[permissions]                # без запису — немає доступу
filesystem = "mod_data"      # лише власна тека даних мода
network = false              # клієнтські моди: заборонено за замовчуванням

[budget]
tick_ms = 0.5                # очікуваний час на тік на регіон (soft); hard = 4× або серверний ліміт
memory_mb = 32               # ліміт лінійної пам'яті

[wasm]                       # якщо є код
server = "build/server.wasm"
client = "build/client.wasm"

[[component]]                # мод-компоненти з реплікацією (лише side = both)
name = "copper_pipes:flow"
replicate = true
schema = "u16"               # опис схеми для хешу рукостискання
```

## Правила валідації (`modhost` при завантаженні, `engine-cli` при збиранні)

- `id` не збігається з іншим модом; усі ID у датапаку мають префікс `id:` (крім явних перевизначень з `override = true`, лише для залежностей).
- `api` сумісна з рушієм (поточна або попередня мажорна через адаптер).
- Граф залежностей без циклів; порядок завантаження — топологічний, рівні — за `id`.
- Компонент WASM імпортує лише інтерфейси, дозволені `[permissions]`; інакше — помилка лінкування з поясненням.
- `side = client` не може мати `[wasm].server`, і навпаки.
