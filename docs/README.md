# Документація

Головний документ — [`PRD.md`](PRD.md): навіщо, цілі, метрики, ризики, план. Решта тек — специфікації для розробки: що саме будуємо, з якими інваріантами і як перевіряємо.

Порядок читання для нового розробника чи агента:

1. [`../AGENTS.md`](../AGENTS.md) — правила роботи в репозиторії.
2. [`architecture/overview.md`](architecture/overview.md) — карта крейтів, потоків і даних.
3. [`process/roadmap.md`](process/roadmap.md) — поточний етап і критерії виходу.
4. Специфікація області, з якою працюєте (таблиця нижче).
5. [`adr/`](adr/) — чому прийняті ключові рішення. Не переглядайте їх без нового ADR.

## Карта документів

| Область | Документ | Крейт |
| --- | --- | --- |
| Тік, фази, планувальник, бюджети | [architecture/engine-core.md](architecture/engine-core.md) | `engine-core` |
| Секції, палітра, ID, збереження | [architecture/world-storage.md](architecture/world-storage.md) | `world` |
| Мешинг і рендер | [architecture/meshing-and-rendering.md](architecture/meshing-and-rendering.md) | `mesh`, `render` |
| LOD | [architecture/lod.md](architecture/lod.md) | `lod` |
| Світло | [architecture/lighting.md](architecture/lighting.md) | `light` |
| Генерація світу | [architecture/worldgen.md](architecture/worldgen.md) | `worldgen` |
| Симуляція, фізика, ШІ мобів | [architecture/simulation-and-ai.md](architecture/simulation-and-ai.md) | `sim` |
| Мережевий протокол | [architecture/networking-protocol.md](architecture/networking-protocol.md) | `protocol`, `net` |
| Регіони сервера | [architecture/server-regions.md](architecture/server-regions.md) | `sim`, `server` |
| Шардування | [architecture/sharding.md](architecture/sharding.md) | `server` (+ майбутній `gateway`) |
| Меню-лаунчер і апдейтер | [architecture/client-menu-and-updater.md](architecture/client-menu-and-updater.md) | `client`, `updater` |
| Вхід і асети | [architecture/auth-and-assets.md](architecture/auth-and-assets.md) | `auth`, `assets` |
| Моди: огляд | [modding/overview.md](modding/overview.md) | `modhost`, `mod-sdk` |
| Маніфест мода | [modding/mod-manifest.md](modding/mod-manifest.md) | `modhost` |
| Датапаки | [modding/datapacks.md](modding/datapacks.md) | `modhost`, `world` |
| WASM API | [modding/wasm-api.md](modding/wasm-api.md), [`../wit/engine-api.wit`](../wit/engine-api.wit) | `modhost`, `mod-sdk` |
| Як писати моди | [modding/writing-mods.md](modding/writing-mods.md) | `mod-sdk`, `engine-cli` |
| Правила швидких модів | [modding/performance-rules.md](modding/performance-rules.md) | — |
| Етапи і критерії виходу | [process/roadmap.md](process/roadmap.md) | — |
| Бюджети продуктивності | [process/performance-budgets.md](process/performance-budgets.md) | — |
| Стиль коду | [process/coding-standards.md](process/coding-standards.md) | — |
| Тести і бенчмарки | [process/testing-and-benchmarks.md](process/testing-and-benchmarks.md) | — |
| Юридичні обмеження | [legal.md](legal.md) | — |
| Відкриті питання | [open-questions.md](open-questions.md) | — |
| Терміни | [glossary.md](glossary.md) | — |
| Рісьорч і бенчмарки етапу «до коду» | [research/](research/) | `../research/bench` |

## Правило джерела правди

- Рішення — в ADR. PRD і специфікації посилаються на ADR, а не дублюють аргументацію.
- Числа (бюджети, цілі) — у [`process/performance-budgets.md`](process/performance-budgets.md). Якщо число змінюється, міняйте його там і в PRD однаково.
- Якщо специфікація суперечить коду, неправий хтось один: виправте або код, або специфікацію в тому ж PR.
