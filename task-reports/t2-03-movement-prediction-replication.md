# T2-03: player movement — prediction, reconciliation, entity replication

## What was done

**Protocol version 2 (`crates/protocol`):**

- `Intent::state_hash` — the client's hash of its predicted state after the tick.
- `EntityDatagram` gained:
  - `ack_tick`, the newest applied intent;
  - `correction: Option<Correction>`, the exact state after a tick;
  - `EntityUpdate::moved: Option<Step8>`, a small move since the baseline;
  - flag constants.
- Golden files refreshed; `PROTOCOL_VERSION = 2`. Round-trip, fuzz and size tests cover the new fields.

**Server (`crates/server`):**

- `input.rs`: `state_hash` (FNV-1a over the exact `f64` bits of position, velocity, flags and fly speed), `correction` / `corrected`, `flags`.
- `game.rs`:
  - every player gets a `NetId`;
  - after applying an intent the server compares hashes and keeps a pending correction until a later intent agrees;
  - every tick each player gets one datagram (`replicate`), built from the other spawned players within the view radius;
  - the snapshot counter advances once per tick.
- `replication.rs`:
  - `Replicator` (per client) keeps the last 64 snapshots *as the client will see them*, builds deltas against the acknowledged one and keeps the nearest updates that fit;
  - near entities (≤ 48 blocks) update every tick, far ones every 4th tick;
  - `Reconstructor` is the client side.
- `conn.rs`: forwards `ack_snapshot`; sends `ToConn::Datagram` as QUIC datagrams.

**Client (`crates/client`):**

- `remote.rs`: a datagram receiver; `Remote::bytes()` (UDP payload counters).
- `session.rs`:
  - each tick stores the intent and the predicted state (128 ticks), and sends the hash and the snapshot acknowledgement;
  - `reconcile` restores the server's state, replays the later intents and turns the change of the drawn position into an offset;
  - the offset decays with a 100 ms time constant, and a correction over 2 blocks snaps;
  - new API: `others()`, `net_stats()`, `nudge_for_test`.
- `others.rs`: rebuilds snapshots and glides each entity over the time between its last two updates (50–300 ms; teleports over 8 blocks).
- `view.rs` draws other players as boxes (body, head, a "nose" showing the look direction) with a new `render::entities::BoxRenderer`. It uses one instanced draw with vertex attributes only, so it works on paths B, Bi and C; the shader is validated in a unit test.
- The debug overlay shows players in view and corrections.

**Tests:**

- `crates/server/tests/determinism.rs`: a 600-tick scripted route on a generated world; golden hash `0x170d4d5d`, the same in debug and release.
- `server::replication` unit tests: deltas rebuild the server view under 1-in-7 datagram loss, joins and leaves; steady-state deltas are smaller than full updates and empty when nothing moves; 300 entities arrive nearest first over several snapshots.
- `crates/client/tests/replication.rs` (cloud VM, 4 vCPU Xeon 2.1 GHz, debug, one run each):
  - scripted 12 s walk at 2% loss / RTT 100 ms: 29.7 blocks walked, **0 corrections**;
  - forced 0.6-block divergence: 1 correction, back exactly on the server's position, largest drawn step per frame **0.03 blocks**, offset gone after 2 s;
  - two clients see each other (within 0.5 blocks);
  - 20 bots walking circles near an observer: the observer receives **27.4 kbit/s** and sends 15.7 kbit/s of QUIC UDP payload in steady state (10 s window after a 6 s warm-up).
- `cargo fmt --all`, `cargo clippy --workspace --all-targets` with `-D warnings`, `cargo test --workspace`.

## What went wrong

- A replication unit test assumed a walking entity stays "near", but it crossed 48 blocks and switched to 5 Hz updates. The test route now stays near.
- Two closures in the traffic test were declared `mut` without need (warnings); fixed.

## Issues encountered

- **Corrections by hash instead of acknowledging every state.** The spec says "the server acknowledges … the authoritative state". Sending the state every tick would cost ≈ 50 B/tick per player. Instead the client sends a 4-byte hash with each intent, and the server sends the state only on a mismatch. The behaviour is the same, and traffic is lower. Documented in `simulation-and-ai.md`.
- **Lost intents still cause a correction:** if all three copies of an intent are lost, the server skips that tick. That needs 3 losses in a row (≈ 1 in 125,000 datagrams at 2% loss), and the correction is smoothed out.
- **Bandwidth was measured on loopback with bots**, not on a LAN with game clients. The 200-bot number comes with T2-05/T2-09.
- **Other players are boxes** until models exist (stage 4). There are no animations, and the yaw is shown only by the nose box.
- **Mobs and entities other than players:** the replication handles any `kind`, but only players exist (stage 4).
- **Feel review:** the owner plays two clients on LAN (`client --offline-name A --connect <server>`, and a second client with another name). This is still open, so the task stays in progress.
- `Reconstructor` lives in `server::replication`, shared with the client like `server::input`. If the protocol grows more shared logic, a small shared crate may be cleaner.
