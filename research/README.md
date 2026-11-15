# research/

Код досліджень, що обґрунтували рішення в ADR. Не входить у основний workspace.

## bench/

Окремий Cargo workspace з бенчмарками етапу «до коду» (запускались на VM з 2 vCPU Xeon 2,1 ГГц, шум ±30%):

| Крейт | Що міряє | Результат |
| --- | --- | --- |
| `meshing` | `binary-greedy-meshing` 0.6.4 на юніті 62³, наївний мешер, rayon | `docs/research/optimization-tests.md` §1 |
| `palette` | палітрові секції 16³: розмір, get/set | §2 |
| `wasmbench` | wasmtime 50: виклики core і component, пакети, epoch, fuel | §3 |
| `light` | BFS-світло, 16³ і 3×3×3 | §4 |
| `zstdbench` | zstd на синтетичних колонах (дані нереалістичні) | §5 |

```bash
cd research/bench && cargo run --release -p meshing
```

Сирі виводи — `bench/results/`. Шляхи `/home/claude/...` у документах `docs/research/` — це місця, де бенчмарки запускались під час дослідження; у репозиторії вони відповідають `research/bench/` і `docs/research/`.

Крейти робочого коду (`crates/world`, `crates/mesh`, `crates/light`) можуть брати код звідси як відправну точку (задачі T0-03, T0-04, T0-08), але не залежать від цього workspace.
