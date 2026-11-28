# T4-08: zones

## What was done

This task implements the `zones` capability (PRD §14 No. 11, ADR-0018): named world areas with rules for building, entry and PvP, plus spawn tables and a difficulty. It is the first capability with a real subsystem.

**`world::zones`** (new module)

- `ZoneDef` is the file format (`data/zones/*.toml`, `deny_unknown_fields`):
  - an id, a display name, a priority and a difficulty;
  - `areas`: boxes with inclusive ranges; when `y` is absent the box covers the whole height;
  - `members`: player names exempt from a denied build or entry rule;
  - `rules`: `build`, `entry`, `pvp`, each optional;
  - `spawns`: optional. When it is absent the zones below decide; `[]` means nothing spawns here;
  - `override`.
- `validate()` checks ids, ranges, weights, group sizes and light ranges.
- `ZoneIndex` is built once and only read afterwards:
  - The map is cut into 64 × 64 tiles, each with a list of areas in decision order: priority descending, then id.
  - Areas wider than 65,536 tiles go to a "wide" list that is merged in during lookup.
  - `resolve(pos)` decides each rule separately; the first zone that sets a rule decides it. Where no zone sets a rule, it is allowed.
  - Query functions: `may_build` / `may_enter` (members are exempt), `pvp_allowed(attacker, victim)` (PvP must be allowed at both ends), `spawn_table(pos)` with a weighted, light-filtered `SpawnTable::pick`, and `zone_at`.
  - `resolve_reference` scans every area without tiles. It is the reference implementation the index is checked against.
- `from_mods(&ModSet)` loads zones from the mods.
  - A zone's namespace must be its mod's id.
  - A later mod may replace a zone of a mod it depends on with `override = true`; any other repeated id is an error that names both mods.
- `read_world` / `write_world` handle `<world>/zones/zones.toml`. Writes are atomic: a temporary file, then a rename.

**`server`**

- New `server::zones`:
  - The `Zones` subsystem is registered in `SubsystemRegistry::builtin()` for `Capability::Zones`.
  - It loads mod zones plus world zones; a world zone with the same id replaces the mod zone.
  - `ZoneControl` is the server-side zone API (`define`, `remove`), reached through `Server::zones()` and `World::zone_control()`. A definition is validated when `define` is called. Changes are applied in the subsystem tick, between ticks, and saved to the world file. A change that would break the index is refused and logged.
- `Subsystem` has two new default methods, `zones()` and `zone_control()`. `Subsystems` forwards them.
- The `World` keeps `Option<Arc<ZoneIndex>>` and refreshes it after the subsystem tick, so changes apply from the next tick. Regions read it through `Shared::zones`, an immutable reference, with no locks (ADR-0008).
- Enforcement:
  - **Build:** `check_edit` refuses an edit where the player may not build. The client gets `BlockAck { accepted: false }` and rolls back its prediction.
  - **Entry:** `move_players` undoes a step that takes a player from an allowed cell into a denied one and sets the velocity to zero. The existing state-hash check then sends a correction. A player who is already inside can move and leave.
  - Undone steps are counted in `RegionOutput::zone_refusals` and `World::zone_refusals()`.
- Without the capability, `Shared::zones` is `None`. The cost is one branch per player step and per edit, and no copy of the body is made. Existing builds are unchanged: classic still starts no subsystem.

**Tests**

- `world::zones` has 12 unit tests:
  - the documented format;
  - clear errors for bad definitions and duplicate ids;
  - inclusive bounds and negative coordinates across tile borders;
  - height limits;
  - per-rule priority resolution and tie order;
  - member exemption, and that members do not bypass PvP;
  - wide areas merged in order;
  - spawn tables from the zone under a position only, with weights 3:1 giving exactly 3000:1000 over 4000 rolls and the light filter;
  - the world file round trip;
  - mod override rules;
  - an empty index;
  - **index vs reference scan** on 300 random zones (including wide ones), with 20,000 random positions, all equal.
- `server/tests/zones.rs` runs on the real generated world, with a test mod that requires `zones`:
  - **Build rights:** bob's dig in the no-build town is refused and member alice's is accepted. A world zone defined through the API (priority 10, `build = true`) lets bob dig on the next tick and is saved to `zones/zones.toml`. An invalid definition is refused when `define` is called. After `remove`, the town rule holds again and the file is empty. Once the world stops, `define` fails. A new `World` on the same folder loads the saved zone.
  - **Entry:** bob and alice fly at the no-entry wall. Bob pushes against the border for ≥ 30 ticks and never reaches x ≥ 48 (he stops at x ≈ 47.03). The server refused his steps (≥ 3 refusals) and corrected his client. Alice, a member, passes to x > 68.
- `server/tests/capabilities.rs`: the "unimplemented capability" test now uses a `dims_test` mod requiring `dimensions`, because `zones` is implemented now.

**Benchmark** `cargo bench -p world --bench zones` (`zones_lookup`, new). It resolves the rules at 1,000 random positions over 8,192 × 8,192 blocks, with random boxes of 16–256 blocks plus 2 whole-map zones. Results are per entity lookup. Machine: a cloud **VM**, 4 vCPU Intel Xeon 2.3 GHz. There were 2 runs; the numbers below are from the second, and the first was within 8%.

| Zones | `resolve` | `may_build` | reference (scan every area) |
| --- | --- | --- | --- |
| 0 | 7.1 ns | 2.4 ns | 8.5 ns |
| 10 + 2 | 15.2 ns | 14.0 ns | 12.8 ns |
| 100 + 2 | 14.6 ns | 14.1 ns | 100 ns |
| 1,000 + 2 | 19.8 ns | 18.6 ns | 2,197 ns |
| 4,000 + 2 | 36.0 ns | 36.5 ns | — |

The proposed budget row in `performance-budgets.md` is ≤ 100 ns per entity at 1,000 zones; it is met on the VM. For scale, 200 players stepping every tick cost ≈ 4 µs per tick at 1,000 zones.

**Docs**

- New `docs/modding/zones.md`: format, rule resolution, world zones and API, cost, what is not done.
- Updated:
  - `datapacks.md`: the path is now `data/zones/`, and the example uses the real format;
  - `builds.md`: the capability row and "How it works";
  - `worldgen.md`: game zones;
  - `simulation-and-ai.md`: spawning;
  - `docs/README.md`: index row;
  - `performance-budgets.md`: new row.

**Commands**

- `cargo fmt --all`: run.
- `RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets`: clean.
- `cargo test --workspace`: 359 tests passed in 80 suites, 0 failed (cloud VM, 4 vCPU, one run).
- `cargo test -p server --no-default-features --lib --test capabilities --test zones`: 26 tests passed.
- `cargo bench --workspace --no-run`: compiles.

## What went wrong

- **The entry test passed too easily at first.** Its loop stopped as soon as bob reached the border, after 2 refusals. It now keeps bob pushing for 30 ticks. A first attempt then asserted ≥ 30 refusals, which was wrong: after a refusal the velocity is zero, so the next few steps stay inside the last allowed cell and are not refusals. The test now asserts the real property: bob is never inside, and his steps were refused at least 3 times.
- **The doc comment of `columns_by_distance` was split.** A scripted insertion of `feet_cell` landed between that function's doc comment and the function. This was caught when the file was read back and fixed before building.

## Issues encountered

- **Base branch.** T4-08 depends on T4-01, which exists only on the unmerged branch `ccr-b8153911-es3dnf`. That branch is stacked on T1-23 / T1-21 / T1-18 / … / T2-01. This work sits on top of it (branch `ccr-b8153911-w9uk07`), not on `main`. As the T4-01 report says, `main` is behind several unmerged lines. Open PRs #38 and #39 redo T2-01 / T2-02 separately, and other unmerged branches hold more parallel work (`ccr-b8153911-v42apu` T1-20, `ccr-b8153911-17d0gw` T1-25, `ccr-b8153911-awpqgm`, another T2-01). **The owner needs to choose one line to merge.**
- **Order note.** `tasks/README.md` suggests "T4-02, then T4-03 and T4-08". The task file lists only T4-01 as a dependency, and nothing in T4-08 needs dimensions. When T4-02 lands, zones become per dimension: one `ZoneIndex` per dimension, and a `dimension` field or one zone file per dimension.
- **PvP and spawn tables have no caller yet.** The server has no combat and no natural spawning. The task's "rules enforced server-side" is met for build and entry on the real server. PvP and spawning are server-side queries tested in `world::zones`, ready for the gameplay systems of stage 4. They must call `pvp_allowed` and `spawn_table` when those systems are written.
- **"Mod API"** is a Rust API for now (`ZoneControl`). A WIT interface comes with the stage 3 mod API, as ADR-0006 says.
- **The client does not know zones.** It mispredicts edits in no-build zones (rolled back after one round trip) and learns about a closed border from the correction. Sending zones to the client is a follow-up; it needs protocol messages and a version bump.
- **Entry uses the feet cell**, so the body may overlap a closed border by half its width (≈ 0.3 blocks). This is documented in `zones.md`. Checking the whole box is possible if the owner wants a strict border.
- **Members are player names.** Offline identity uses names. With Microsoft accounts (T0-10) names can change, so members should move to stable ids. Role-based permissions belong to the planned `roles` module.
- **Zones by biome** (the old `datapacks.md` example had `biomes = [...]`) need the biome of a position at runtime. This is left for `rpg_world` and the `map` engine, and listed under "Not done yet".
- **Zone-file writes happen on the world thread.** They are small and only happen after an operator or mod change, never every tick. If zone edits become frequent (claims on a busy server), they should move to the storage thread.
