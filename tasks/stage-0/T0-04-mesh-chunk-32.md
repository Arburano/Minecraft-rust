# T0-04. Gather, інкрементні маски і fast_mesh для меш-чанка 32³

- **Статус:** todo
- **Залежить від:** T0-03
- **Виконавець:** агент

## Мета

У крейті `mesh`: gather 34³ з секцій (пословне розпакування), маски непрозорих/прозорих з інкрементним оновленням на `set_block`, виклик `binary-greedy-meshing` 0.6.4 `Mesher<32>::fast_mesh`, пропуск порожніх і суцільних меш-чанків. Відправна точка — `research/bench/meshing` (там юніт 62³).

## Специфікації

- [docs/architecture/meshing-and-rendering.md](../../docs/architecture/meshing-and-rendering.md)
- [docs/adr/0005-mesh-chunk-32-with-batches.md](../../docs/adr/0005-mesh-chunk-32-with-batches.md)

## Результат

- API: `MeshChunkBuilder::build(&SectionNeighborhood) -> MeshChunkQuads` і `MaskCache::on_block_set`.
- Еталонний наївний мешер для тестів (кількість видимих граней має збігатися).
- Бенчмарк на фіксованому корпусі (синтетичний рельєф із seed, 8% прозорих).

## Критерії приймання

- [ ] Видима геометрія `fast_mesh` дорівнює еталону (проптест на випадкових меш-чанках; порівнювати покриті грані, а не кількість квадив).
- [ ] Числа gather+маски, `fast_mesh`, оновлення маски записані в performance-budgets.md.
- [ ] Правка на межі меш-чанка позначає брудними правильних сусідів (тест).

## Поза межами задачі

Без GPU і без рендер-пакетів (T0-05).
