# T0-09 report: server regions prototype

Date: 2026-11-20. Branch: `t0-09-sim-region-prototype`. PR: [#18](https://github.com/Arburano/Minecraft-rust/pull/18). Task: [T0-09](../tasks/stage-0/T0-09-sim-region-prototype.md).

## What was done

Implemented the region core in `crates/sim` per ADR-0008 and
`docs/architecture/server-regions.md`:

- **Region = `bevy_ecs::World`.** Entities carry a stable `NetId` component;
  a `NetId → Entity` index per region. `Body { position, velocity }` is the
  prototype component set. Sections are stored as `world::Section` in a
  per-region map; the prototype world is flat (one solid ground layer,
  single section layer y = 0).
- **Tick phases.** `RegionNode::tick` runs PreTick (intents, sequential) →
  Simulate (regions in parallel on a rayon pool, `min(regions, cores)`
  threads) → Exchange (collect outboxes, sort by
  `(tick, sender_region_stable_id, sender_pos, counter)`, route by target
  position, deliver) → PostTick (structural operations between ticks).
  A message sent in tick N is processed in tick N+1; a reply arrives no
  earlier than N+2 (unit-tested).
- **Transfers.** When an entity's section leaves its region's territory,
  Simulate stages a `Payload::Transfer` (body copied by `NetId`, source
  despawns at send time); Exchange spawns it in the target region and
  updates the owner map. Routing is by position, so a merge or split
  between send and delivery does not lose the message (unit-tested).
- **Merge/split.** Merge requires a shared full edge, keeps the first
  region's stable id and retires the second forever; sections, entities
  and inboxes move. Split is deferred while any entity occupies the
  sections at the new border (buffer invariant: ≥ 1 section between
  regions' occupied territory); the minimum region side is 3 sections.
- **Determinism.** FNV-1a state hasher (no process-random seed), systems
  iterate entities in sorted `NetId` order through a reused scratch Vec
  (no per-tick allocation in the hot loop), unique message sort keys.
- **Tests.** 20 unit tests (territory geometry, message ordering, phases,
  transfer delivery, merge/split rules, owner map); `tests/conservation.rs`
  proptest (64 cases, up to 100 bots, up to 40 random merge/split attempts
  per case — resident + in-flight ids always equal the initial set,
  territories always tile the arena); `tests/determinism.rs` (200 bots,
  200 ticks with a merge and a split — identical state hashes across pool
  sizes 1/2/4 and a repeat run); `tests/tick_latency.rs` (ignored,
  on-demand percentile measurement).
- **Benchmarks and evidence.** 3 runs × 1 000 ticks (+100 warmup) for
  100/500/2 000 bots × 1/8 regions, plus Criterion (30 samples) tick
  matrix and the entity-transfer primitive. All configs far under the
  25 ms budget (worst p99 ≈ 0.27 ms); transfer ≈ 168.5 ns vs the ≤ 5 µs
  hypothesis. Raw outputs in `docs/research/sim/`, summary in
  [docs/research/sim-regions-2026-11-20.md](../docs/research/sim-regions-2026-11-20.md).
- **Docs.** `performance-budgets.md`: entity-transfer row Hypothesis →
  Measured, first region-tick data added (the 500-player/16-core Target
  unchanged); `open-questions.md` No. 6: first data note, answer still
  stage 2; `server-regions.md`: prototype status and relaxations.

Machine: bare-metal ASUS TUF FX505DU, Ryzen 7 3750H (4 cores / 8 logical
CPUs), Windows 11 build 28000, Rust 1.97.1, release profile (thin LTO,
1 codegen unit). Frequency not pinned, background not controlled.

## What went wrong

- **Owner map was not updated on transfer delivery.** The first transfer
  test failed: after Exchange the entity lived in the target region but
  `owner_of` still returned the sender. Fixed by updating the owner map
  when a transfer is delivered; the test now asserts the target owns the
  entity after Exchange.
- **bevy_ecs 0.20 `Mut<T>` borrow surprises.** `world.get_mut::<Body>`
  returns `Mut<Body>`; `body.position += body.velocity * dt` fails E0502
  (two mutable derefs). Fixed by copying velocity to a local first.
- **Wrong test expectations, not code bugs:** splitting a merged 16 × 8
  arena at line 4 yields 32/96-section halves, not 32/32; `inner()` of a
  3 × 3 rect is `Some(1 × 1)`; the ground-clamp test needed a start just
  above ground; the reply-latency test had to account for intents to
  in-flight entities being dropped (reply path N → N+3, not N+2).
- **Compile fixes:** `world` re-exports only `Section` at the root —
  coordinates live in `world::coords`; a `tick` getter collided with the
  `tick(intents)` method (renamed `current_tick`).
- **Clippy (CI `-D warnings`):** `extend(drain(..))` → `append`, `vec!` →
  array in a test, `#![allow(dead_code)]` in the shared test module (each
  integration test compiles it separately and uses a subset).

## Issues encountered

- **Buffer vs continuous movement.** The spec's buffer invariant is about
  occupied sections, but ownership by territory containment means a
  walking entity transiently occupies the buffer ring before its transfer
  is delivered next tick. Kept as a documented prototype relaxation
  (territory containment is what makes conservation checkable); the
  structural invariant is enforced where it matters — split deferral at
  new borders.
- **Prototype scope relaxations** (documented in `server-regions.md`):
  flat single-layer world, rectangular x/z territories, intents for
  in-flight entities dropped deterministically, scripted kinematic bots
  instead of gameplay. The benchmark therefore measures region-machinery
  overhead, **not** player capacity — open question No. 6 stays open.
- **Hardware gap:** the spec's test condition is 16 cores; this machine
  has 8 logical CPUs (4 physical). The 500-player/16-core Target stays
  unverified; 8 regions give ≈ 1.5× p50 speedup at 2 000 bots with
  comparable p99 (exchange and pool synchronization dominate the tail at
  this light load).
- **Known flaky test in `net`:** during the final verification
  `net::quic_spike::lossless_full_transfer` failed once (arrival-order
  assertion, the same flake that hit PR #16 CI); an immediate rerun passed
  3/3. Not touched by this task; it deserves its own stabilization task.
- **Disk pressure:** C: filled to 100% during the final verification —
  the MSVC linker failed with LNK1318 (PDB write) and rustc with os error
  112. Freed space by deleting `target/debug/incremental` (1.1 GB) and
  stale duplicate-version artifacts in `target/debug/deps` (11.2 GB,
  keeping the newest hash per crate); all regenerable. Verification
  re-run green afterwards.

## Verification

- `cargo fmt --all --check` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — all pass (sim: 20 unit + conservation
  proptest 64 cases + determinism; latency test ignored by design).
- `cargo bench --workspace --no-run` — builds.
- Measurements as above; no VM numbers, all bare-metal.