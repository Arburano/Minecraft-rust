# WASM API

Contract: [`../../wit/engine-api.wit`](../../wit/engine-api.wit), package `engine:api@0.1.0` (draft; finalized by task T0-07). Syntax check: `wasm-tools component wit wit/engine-api.wit`.

## Interfaces

| Interface | Direction | Purpose |
| --- | --- | --- |
| `types` | shared | positions, states, events, errors |
| `world-access` | host → guest (import) | reading and batched modification of blocks of its own region, `send-to` |
| `components` | import | registration of mod components, batched reading/writing of state in the host |
| `host-log` | import | logging with levels (rate-limited) |
| `guest` | mod export | `init` (declaration of systems and subscriptions), `on-events`, `run-system` |

## API design rules (mandatory for changes)

1. **Batching.** No function works with a single block or a single entity if it can be called for a list. A component call costs ≈ 0.4 µs; 1,000 separate calls — 0.5 ms, one batch — ≈ 3 µs.
2. **Target:** ≤ 1 µs per hook dispatch, ≤ 5 ns per event in a batch.
3. **State in the host.** The API does not encourage keeping the world in the mod's linear memory.
4. **Region.** Everything that reads or changes the world acts within the region; outside it — an `out-of-region` error, not a silent denial.
5. **Atomicity.** World changes from one call are applied together or not applied at all.
6. **No global time.** The mod receives the tick number from the host; the system clock is unavailable (test determinism).

## Versioning

- Semver of the package. A minor version only adds (new functions, new variants at the end). A major version may break.
- The engine supports the current and previous major via an adapter component.
- Any change to `wit/engine-api.wit` is a separate PR with updates to this document, `mod-sdk` and the test mod; a breaking change — with an ADR.

## Future interfaces (not in 0.1)

UI (menu and HUD widgets), render passes in WGSL (level 3), queries to global services (asynchronous), interfaces between mods (a mod exports its own WIT).
