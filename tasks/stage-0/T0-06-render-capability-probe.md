# T0-06. Пробник можливостей GPU і прототип vertex pulling

- **Статус:** todo
- **Залежить від:** T0-04
- **Виконавець:** агент пише, людина запускає на залізі

## Мета

У крейті `render`: (1) утиліта `gpu-probe`, що друкує `features`, `limits`, `downlevel_capabilities` для всіх адаптерів і бекендів (Vulkan, DX12, GL) у JSON; (2) прототип малювання 0,5 млн квадив з вибором шляху B (vertex pulling) або C (per-instance) з вимірюванням GPU-часу через timestamp queries, якщо доступні.

## Специфікації

- [docs/architecture/meshing-and-rendering.md](../../docs/architecture/meshing-and-rendering.md)
- [docs/adr/0003-wgpu-own-renderer.md](../../docs/adr/0003-wgpu-own-renderer.md)

## Результат

- Бінарник `cargo run -p render --bin gpu-probe` і `--bin quad-bench`.
- Логіка вибору шляху — чиста функція з юніт-тестами на фейкових можливостях (CI без GPU).
- Інструкція для людини: як запустити на iGPU і GTX 1050 і куди покласти JSON (`docs/research/gpu/`).

## Критерії приймання

- [ ] Юніт-тести вибору шляху проходять у CI без GPU.
- [ ] Людина приклала дампи і числа з двох машин; рядки «GPU-час геометрії» і «CPU запису команд» оновлені.

## Поза межами задачі

Без compute-кулінгу (шлях A) — етап 2.
