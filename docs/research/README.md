# Рісьорч

Матеріали, з яких виросли PRD і ADR (листопад 2026).

| Файл | Що це | Мова |
| --- | --- | --- |
| [optimization-search.md](optimization-search.md) | Веб-рісьорч підходів оптимізації воксельних ігор з посиланнями | англ. |
| [optimization-tests.md](optimization-tests.md) | Результати бенчмарків з `research/bench` (VM 2 vCPU, ±30%) | англ. |
| [optimization-analysis.md](optimization-analysis.md) | Аналіз і рекомендації (те саме, що PRD §13) | укр. |
| [review-budget.py](review-budget.py) | Розрахунки рецензента: пам'ять, VRAM, трафік, час входу | Python |

Правила:

- Нові вимірювання кладіть сюди окремими файлами `<тема>-<дата>.md` з описом машини, версій і методики.
- Дампи можливостей GPU (задача T0-06) — у `gpu/`.
- Якщо нове вимірювання змінює ціль, оновіть `docs/process/performance-budgets.md` і PRD.
