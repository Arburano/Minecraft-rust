# T1-04. Optimize section get/set and gather + masks

- **Status:** in-progress (2026-11-21: division-free packed addressing done and measured on a VM; laptop confirmation pending)
- **Depends on:** T0-16
- **Owner:** agent (cloud for the work, owner's laptop for confirmation)

## Goal

Bring the hot paths that missed on bare metal back into budget (T0-16, owner decision: targets kept): section `get` 8.3–8.8 ns vs ≈ 3.8 ns reference, `set` 12.7–29.4 ns vs ≤ 10 ns, gather + masks 112.8 µs vs ≤ 100 µs p50 on Ryzen 7 3750H.

## Specs

- [T0-16 summary](../../docs/research/bare-metal-2026-11-21/summary.md)
- [T0-03 report](../../task-reports/t0-03-world-palette-section.md) (4K aliasing)
- [world-storage.md](../../docs/architecture/world-storage.md)
- [meshing-and-rendering.md](../../docs/architecture/meshing-and-rendering.md)

## Deliverables

- Profile first (cache misses, 4K aliasing, bounds checks, branch on palette width); write down the findings before changing code.
- Optimizations with before/after Criterion numbers on the same machine; the existing proptests and the reference implementations stay green.
- Confirmation run on the owner's laptop (3 runs, as in T0-16).

## Acceptance criteria

- [ ] `get`, `set`, gather + masks meet their rows on the owner's laptop, **or** a measured explanation goes to the owner with a proposal (AGENTS.md: unreachable target → ask).
- [ ] performance-budgets rows updated from deviation to measured.

## Out of scope

Changing the section format (ADR-0004) without a new ADR.
