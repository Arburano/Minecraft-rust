# Датапаки

Мод рівня 1: усе, що можна описати даними, описується даними. Рушій компілює датапаки в реєстри при завантаженні; у рантаймі вони нічого не коштують.

## Структура

```text
<mod>/data/
  blocks/*.toml      items/*.toml      entities/*.toml
  recipes/*.toml     loot/*.toml       tags/*.toml
  worldgen/biomes/*.toml   worldgen/structures/*.toml   worldgen/zones/*.toml
  physics/player.toml      # параметри руху (core)
<mod>/assets/
  textures/   models/   sounds/   lang/<locale>.json   shaders/*.wgsl
```

## Блок

```toml
id = "copper_pipes:copper_pipe"
model = "copper_pipes:models/pipe"
textures = { all = "copper_pipes:block/copper_pipe" }
hardness = 2.5
tool = "pickaxe"
opaque = false
transparent = true                      # іде в маску прозорих
light_emission = 0
collision = "model"                     # full | none | model
states = { facing = ["north", "south", "east", "west", "up", "down"], waterlogged = [false, true] }
components = ["copper_pipes:flow"]      # поведінка з WASM
tick = "none"                           # none | random | scheduled
```

Кожна комбінація `states` — окремий `BlockStateId`. Тримайте кількість станів малою: вона множиться.

## Зона (PRD §14, № 11)

```toml
id = "core:frozen_peaks"
biomes = ["core:snowy_slopes", "core:frozen_peaks"]
difficulty = 3
spawns = [
  { entity = "core:stray", weight = 80, group = [2, 4], light = [0, 7] },
  { entity = "core:goat",  weight = 20, group = [1, 3] },
]
structures = ["core:igloo"]
dungeons = []
```

## Перевизначення

Мод може змінити запис іншого мода лише якщо оголосив його залежністю і поставив `override = true`. Конфлікт двох перевизначень одного ID — помилка завантаження з переліком модів.

## Валідація

`engine-cli check` і завантажувач перевіряють: схему кожного файлу, посилання на текстури/моделі (з запасними асетами), унікальність ID, діапазони чисел. Помилка показує файл і рядок.
