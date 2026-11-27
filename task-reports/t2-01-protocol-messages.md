# T2-01: protocol crate — messages, versioning, handshake

## What was done

The `protocol` crate (`crates/protocol`) is no longer a skeleton. It defines every message of the five channels in [networking-protocol.md](../docs/architecture/networking-protocol.md), all serialized with `bitcode`:

- **`codec`:**
  - `encode` / `decode` for bodies;
  - stream frames (`u32` LE length + body, at most 4 MiB; the length is checked before allocating) with `encode_frame`, `frame_len` and a `FrameReader` that splits a byte stream;
  - datagrams of at most 1,200 bytes (`encode_datagram` / `decode_datagram`).
- **`handshake`:**
  - `Hello { client_version, identity: ClaimedIdentity::Offline { name }, session_jwt }`; `session_jwt` is reserved, `// OPEN-QUESTION: 4`;
  - `ServerHello::{Accepted(Welcome), Rejected(Rejection)}`; `Welcome` has the server info, player ID and mod manifest hash;
  - `Registry { id_table, entity_kinds, component_schemas }` and `Spawn`.
  - The first frame in each direction starts with a fixed prefix, `VXLH` + `PROTOCOL_VERSION` (`u32` LE). A peer of another version always gets `HandshakeError::VersionMismatch`. Its message tells the user what to do: "update the game" when the peer is newer, or "the other side needs an update" when it is older.
- **`messages`:**
  - control: `ClientControl::{Ready, Disconnect}`, `ServerControl::{Registry, Spawn, Kick}`;
  - input: `InputDatagram { ack_snapshot, frames }` with quantized `InputFrame`s;
  - entities: `EntityDelta` (spawns / updates / despawns against an acknowledged baseline, plus `ack_input` for reconciliation);
  - events: `ClientEvent::Chat`, `ServerEvent::{Chat, System}`.
  - Text lengths and input frame counts are checked on decode.
- **`quant`:**
  - `FixedPos` (1/32 block, `i32`) and `PosDelta` (`i16`, with a fallback to absolute);
  - `Angle8` for entities and `Angle16` for the player's own view;
  - `i8` analog axes;
  - NaN and infinity never panic.
- **`column`:** `ColumnData` holds the T1-02 `Section::encode` output, concatenated and compressed with zstd level 1. It has no light (ADR-0011). Unpacking caps the zstd output at `section_count × 8,193 B` (at most 64 sections) and validates every section.
- **Dependencies:** `bitcode` and `zstd` (both already in `[workspace.dependencies]`). `proptest`, `criterion` and `worldgen` are dev-only.
- **CI:** the protocol bench is added to the fast benchmark subset (`bench.yml`).
- **Docs:**
  - `networking-protocol.md`: a new "Messages and encoding" section and measured traffic numbers;
  - `performance-budgets.md`: two new rows;
  - open question 7: real column sizes.

**Tests** (`cargo test -p protocol`, all pass):
- 15 unit tests: frames, datagram limit, quantization, including proptests for position, angle and delta error bounds; handshake; column packing.
- `roundtrip.rs` (7 tests): every message through a frame or datagram, every `Rejection`, plus proptests for random input datagrams, entity deltas and chat.
- `golden.rs` (8 tests): golden byte files `tests/golden/{hello,welcome,rejected,version_mismatch,registry,spawn,ready}.bin`. They are regenerated only with `UPDATE_GOLDEN=1`, together with a `PROTOCOL_VERSION` bump.
- `fuzz.rs` (5 tests): every decoder is run on:
  - 4,096 random byte strings;
  - 4,096 random strings behind a valid handshake prefix;
  - 4,096 mutated valid messages (flips, truncations, insertions);
  - every truncation of every message;
  - a zstd bomb (4 MiB of zeros in a one-section column).

  None panic, and accepted results respect the limits.
- `sizes.rs` (3 tests): the size measurements below.

**Sizes.** These are deterministic; run `cargo test --release -p protocol --test sizes -- --nocapture`:

| Message | Size |
| --- | --- |
| Input datagram, 1 / 3 / 8 ticks | 19 / 47 / 62 B |
| Entity delta, 20 moving entities + 1 spawn + 1 despawn | 179 B (≈ 36.3 kbit/s at 20 Hz incl. ~48 B packet overhead) |
| Entity delta, 50 / 100 moving entities | 364 / 669 B |
| Entity delta, 200 moving entities | 1,279 B — does not fit; split |
| Column record (frame), real overworld, seed 7, 15 × 15 columns, 24 sections | min 1,004 B, median 1,689 B, mean 1,708 B, p95 2,220 B, max 2,649 B; raw sections mean 9,748 B |
| Login at 12 chunks (625 columns) | ≈ 1.07 MB (PRD §8 assumed 2.4–9.8 MB) |

The entity samples move pseudo-randomly, up to ±14/32 block per axis with scattered IDs, so `bitcode`'s packing is not flattered by regular data.

**Benchmark** (`cargo bench -p protocol --bench codec`). Cloud **VM**, 4 vCPU Intel Xeon 2.1 GHz, 3 runs, Criterion with 1 s warm-up and 3 s measurement. The ranges are the medians of the 3 runs:

| Operation | Time |
| --- | --- |
| Column pack (section encoding + zstd 1 + frame) | 41.7–48.9 µs |
| Column unpack (frame decode + zstd + validated sections) | 85.8–110.5 µs |
| Entity delta of 20, encode / decode | 2.14–2.21 µs / 0.85–1.35 µs |
| Input datagram of 3, encode / decode | 0.44–0.75 µs / 90–102 ns |

## What went wrong

- The first datagram-limit unit test used 1,300 zero bytes as an "oversized" message. `bitcode` packed it to well under 1,200 bytes, so the test failed. The test now uses non-repeating bytes.
- The first entity-delta sample was too regular: 127 B for 20 entities. The sample now uses pseudo-random deltas, IDs and yaws, which gives 179 B. The numbers above use the realistic sample.

## Issues encountered

- **Version mismatch on the server side.** When the versions differ, the server answers with the prefix alone (empty body), because the other version's body layout is unknown. The client builds its error from the prefix. The rejection reasons (`Rejection`) are only readable between peers of the same version.
- **Additions to the spec's handshake.** The spec's `Registry { id_table, component_schemas }` gained `entity_kinds` (entity kind IDs for `EntitySpawn`). `Welcome` gained `player_id`, and `Spawn` gained the world bounds and the player's own entity ID. The spec's table documents these.
- **Entity snapshots larger than one datagram.** The protocol defines no split format. `EntityDelta::encode` fails above 1,200 B, and splitting a snapshot (e.g. several deltas with the same `snapshot` number) is left to T2-03. About 130 moving entities fit in one datagram.
- **Decode cost.** Column unpacking takes about 90 µs on the VM and is dominated by `Section::decode` validation, which checks every packed index and builds the reverse map. It runs on worker threads, so this is fine for now. Entity delta encoding allocates a new `Vec` every time; reusing a `bitcode::Buffer` is an easy win if T2-05's network CPU measurement shows it matters.
- **Not wired up yet.** `net` and the server do not use these messages yet; that is T2-02. The T0-11 spike framing in `net::spike::wire` stays as it is.
- All numbers are from a cloud VM. Sizes do not depend on the machine; times should be confirmed on the laptop with the other stage 2 numbers.
