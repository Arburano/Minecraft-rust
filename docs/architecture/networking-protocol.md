# Network protocol

Crates: `protocol` (message types), `net` (transport). ADR: 0007 (QUIC + custom protocol).

## Transport

- One QUIC connection per client (`quinn` 0.11), TLS 1.3, ALPN `vxl/1`.
- Channels:

| Channel | QUIC type | Content | Priority |
| --- | --- | --- | --- |
| control | bidirectional stream | login, versions, ID table, mod manifest, kick | highest |
| input | datagrams | player intents with a tick number (duplicated in 2–3 datagrams against loss) | high |
| entities | datagrams | entity deltas relative to the acknowledged snapshot, 20 Hz near the player, 5–10 Hz farther | high |
| chunks | unidirectional streams, one per column/batch | palette sections + zstd 1 | by distance (`SendStream::set_priority`) |
| lod | unidirectional streams | LOD sections | lower than chunks |
| events | bidirectional stream | chat, inventory, UI, mod messages | medium |

- A datagram is ≤ ~1,200 bytes; anything larger goes into a stream.
- Per-client limit: a maximum of concurrent chunk streams (e.g. 16) so as not to clog the congestion control queue.
- Congestion control: BBR on the server (`net::server_endpoint`). The T0-11 spike showed loss-based Cubic misses the "nearest 3×3 ≤ 1 s" test below at 2% random loss / RTT 100 ms, while BBR meets it with margin ([measurements](../research/net-quic-spike-2026-11-19.md)).

## Versioning

- Messages are Rust enums in `protocol`, serialized with `bitcode`.
- `PROTOCOL_VERSION: u32` in the handshake. An incompatible version → a clear error suggesting an update (the updater).
- Replication of mod components uses the schema from the mod manifest; the schema hash is part of the handshake.

## Handshake

```text
client ─ HandshakeHeader { magic, protocol } ─► server ─ HandshakeHeader ─► client  (fixed 8 bytes)
client ─ Hello { client_version, player_name, session_jwt } ─► gateway/server
server ─ Welcome { server_info, mod_manifest_hash, capabilities } ► client
client: if the mod manifest differs — download via the menu (see client-menu-and-updater.md)
server ─ Registry { id_table, component_schemas } ────────► client
server ─ Spawn { pos, tick } + first 3×3 columns ─────────► client   (player control)
then in the background: the remaining columns by distance, then LOD
```

The "login ≤ 5 s" metric is counted up to the moment of controlling the character (columns 3–4 around); the rest is downloaded in the background.

## Implementation (T2-01, `protocol`)

The crate is transport-agnostic (no `quinn`): messages become bytes and back; `net` moves them.

- **Handshake header:** each side first sends 8 fixed bytes on the control stream: magic `VXEN` + `PROTOCOL_VERSION` (u32 LE). Its layout never changes, so builds with different message layouts still recognize each other: a mismatch is `ProtocolError::VersionMismatch` with the hint "Update the game…" (or "the server runs an older version…"). The server answers with its own header and closes. The protocol version therefore lives in the header, not in `Hello`.
- **Control:** `ClientControl::{Hello, Leave}`, `ServerControl::{Welcome, Registry, Spawn, Kick}`. `Hello` carries the client version, the player name (offline mode; the server validates it with `auth::PlayerName`) and a reserved `session_jwt` (always `None` until T0-10). `Welcome` carries the player ID, tick rate and the mod manifest hash (zeros until stage 3) and, since protocol version 5, the engine capabilities the server runs by name (ADR-0018, T4-01; the client refuses one it does not know or was built without); `RegistrySync` the block ID table (the client maps it with `Registry::remap`) and a reserved component-schema hash.
- **Framing:** control and events streams carry `u32 LE length + bitcode body` frames (`encode_frame`, `split_frame`); a length above 4 MiB is refused before the body arrives.
- **Input datagram:** `ack_snapshot` (newest complete entity snapshot) + 1–8 intents with consecutive ticks; an intent is buttons (bits, incl. the one-tick flight toggle), yaw and pitch in 16 bits, and fly-speed wheel steps.
- **Entity datagram:** `snapshot`, `baseline` (0 = full), an origin block and updates whose fields are `Option`s (absent = unchanged): kind, `RelPos` (i16 × 3 in 1/32 block, ±1,024 blocks), `Look` (three 8-bit angles), flags; plus removed IDs. Up to 128 entries; the server splits bigger deltas over datagrams of the same snapshot.
- **Chunk stream:** one `ChunkStream::Column` per stream: the position and the T1-02 column record (`world::storage::encode_column`) compressed with zstd level 1. Decoding is bounded (≤ 1 MiB compressed, ≤ 8 MiB decompressed, read incrementally) and checks the position. Light is not sent (ADR-0011). Bumping the world format version also requires a protocol version bump.
- **Events:** `ClientEvent::{Chat, Dig, Place}` with sequence numbers, `ServerEvent::{Chat, BlockChanges, BlockAck, ForgetColumn}`. Block edits travel here (reliable, exactly once) rather than in input datagrams; T2-04 builds prediction on them.
- **Safety:** every message has a size limit checked before decoding, `bitcode` rejects malformed bytes, and `Message::validate` rejects values outside the limits (text lengths, control characters, counts, tick order, unknown button bits). The sender runs the same validation, so it never sends what a receiver rejects. `tests/fuzz.rs` feeds random and corrupted bytes into every decoder (4,000 cases per run).
- **Golden files:** `crates/protocol/tests/golden/` pins the handshake bytes; a change fails CI until `PROTOCOL_VERSION` is raised and the files are refreshed with `UPDATE_GOLDEN=1`.

Measured sizes (`cargo test -p protocol --release --test sizes -- --nocapture`; sizes are deterministic, not machine-dependent):

| Message | Size |
| --- | --- |
| Input datagram, 1 / 3 / 8 intents | 14 / 32 / 48 B (protocol v1; v2 with the state hash: 19 / 45 / 81 B) |
| Entity delta, 20 entities, every field changed | 252 B → 40.3 kbit/s at 20 Hz, payload only (v1; v2 steady state: 168 B, see "Entity deltas") |
| Fully-changed entities per 1,200-byte datagram | 99 |
| Generated column (overworld, seed 7, 24 sections, no light), 64 columns | raw record 6.8–11.0 KB (mean 9.4 KB); on the wire 0.87–2.3 KB (mean 1.6 KB) |
| Login at 12 chunks (625 columns) | ≈ 1.0 MB (vs. the 2.4–9.8 MB estimate below) |

## Server and client (T2-02)

- **Server** (`crates/server`, library + binary): `server --offline [--world DIR] [--seed N] [--view-distance 14]`. Threads: a 2-worker tokio runtime for QUIC (accept loop, a task per connection and per column stream), the world thread (20 TPS, owns the `WorldMap`), the generation job pool and a storage thread that owns the `WorldStore` (loads on request, writes unloaded edited columns), so region-file I/O never runs in the tick. One region for now (T2-06).
- **Join:** the handshake above; a new player spawns at the world spawn (the 5×5 columns around it stay loaded even with nobody online), a returning player where they left (`<world>/players/<id>.toml`). The client renders `view_distance − 2` columns (the server streams 2 more for light and meshing of the border).
- **Streaming:** each tick the world thread hands every player the nearest columns they lack, at most **32 streams in flight** per player (a stream counts until the client has acknowledged all of it); stream priority = −distance. Encoded columns are cached until the column unloads. Beyond `view_distance + 2` the server sends `ForgetColumn` and unloads columns no player needs.
- **Movement:** the client sends its intents (each datagram repeats the last 3); the server runs the same `sim::physics::step` on them (≤ 3 per tick to catch up after jitter). Both sides convert intents through `server::input`, and the client simulates with the quantized yaw, so the two positions are bit-identical (tested). Reconciliation by state hash and corrections: simulation-and-ai.md (T2-03).
- **Client:** `client --offline-name NAME --connect HOST:PORT` (the menu entry is T2-08). `Session::connect` puts the loader in network mode (`LoadConfig::generate = false`: columns come only from the network); columns are decompressed, decoded and mapped to the client's block IDs on the network threads; light and meshes stay on the client (ADR-0011). Edits are disabled until T2-04.
- **Block edits (T2-04):** the client predicts dig and place at once (the T1-12 editor: remesh and relight locally) and sends `ClientEvent::Dig` / `Place` with a sequence number on its own events stream; place states are translated to the server's numbering. The world thread validates each edit in arrival order: spawned, cell loaded and inside the height, eye-to-cell distance ≤ reach + 1.5 blocks (the server's view of the player lags a round trip), dig only non-air, place a known non-air state only into air or a fluid and never into another player (the editor checks itself). Rate: ≤ 20 edits/s, burst 40; excess edits are refused, not queued. Accepted edits change the map (saved through the storage thread like any edited column) and drop the column's encoded cache. Every player that has the column gets the tick's `BlockChanges`, then the editor gets `BlockAck`s — on one ordered stream, so the authoritative state always arrives first.
- **Rollback on the client:** while its own edit at a cell is pending, server changes there only update a shadow copy; when the last pending edit at the cell is answered, the cell takes the server's state. An accepted edit therefore stays, a refused one rolls back, and a concurrent edit by someone else wins in server order. Changes for a column that has not arrived yet are buffered and applied to it on arrival. Measured (cloud VM, debug): another client sees an edit after 36 ms on loopback; at RTT 100 ms the worst of 5 was 152 ms (50 ms to the server + ≤ 1 tick + 50 ms to the other client).
- **Single-player and LAN (T2-08):** single-player runs the same server in-process and joins it over loopback; offline servers on a private LAN announce themselves (`protocol::lan`, UDP 47778); see client-menu-and-updater.md. Protocol version 3 adds the fly speed to `Spawn` (it is saved with the player).
- **Limits on client input:** handshake ≤ 10 s; frames bounded by each message's size before reading; ≤ 60 input datagrams/s (burst 120), excess dropped; more than 50 malformed datagrams → disconnect; a second `Hello` → disconnect. Tests: `crates/server/tests/join.rs` (garbage headers, oversized frames, bad names, noisy datagrams, wrong version; the server keeps serving).
- **Transport:** both game endpoints use an initial RTT estimate of 100 ms instead of QUIC's 333 ms, so a lost handshake packet is resent after ≈ 0.3 s rather than ≈ 1 s.

Measured (cloud VM, 4 vCPU Xeon 2.1 GHz, release, `LossyRelay` 2% loss + RTT 100 ms, in-process server with a warm spawn area, `crates/client/tests/remote.rs`): connect to **controllable** (nearest 3×3 received, lit and meshed on the client) **0.51–0.74 s for 9 of 10 seeds, 1.28 s for one** (seed 7: a lost packet at the end of the login burst waits for QUIC's probe timeout, ≈ 0.6 s). Without loss: 0.52 s. 16 streams in flight instead of 32 were ≈ 0.15 s slower (0.93–1.04 s vs 0.77–0.85 s on the same seeds, before the initial-RTT change), hence 32.

## Entity deltas

- For each client the server keeps the number of the last acknowledged snapshot; a delta = changes relative to it.
- Quantization: positions 1/32 block in relative coordinates, angles 8 bits.
- Interest management: only entities in the visibility zone; distant ones less often.

Implementation (T2-03, `server::replication`, protocol version 2):

- One `EntityDatagram` per client per server tick: snapshot number, baseline (the newest snapshot the client acknowledged in `InputDatagram::ack_snapshot`, or 0), the newest applied intent tick, an optional `Correction`, and entity updates.
- The server keeps, per client, the last 64 snapshots **as the client will see them**: entities that do not fit the datagram, or are far and not due, keep their baseline state, so a delta is always exact against what the client has. The client (`Reconstructor`) rebuilds full snapshots and drops a datagram whose baseline it lacks; the next one is relative to an older acknowledged snapshot.
- An update carries only what changed: a known entity's movement as `Step8` (i8 × 3 in 1/32 block, ±4 blocks), a full `RelPos` only for new entities or longer moves, the look and flags only when they change. Entities that stop cost nothing.
- Interest: other players within the view radius; within 48 blocks every tick (20 Hz), farther every 4th tick (5 Hz). Over-full datagrams keep the nearest updates; the rest follow in later snapshots.
- Measured sizes (deterministic): 20 known entities moving and turning **168 B** (26.9 kbit/s payload at 20 Hz), 20 appearing 265 B, an ack-only datagram 17 B, a correction +63 B (protocol 4 with the ability state, T1-18: +68 B when it is all zero, up to ≈ 32 B more with active abilities); 96 appearing entities fit one datagram.
- Measured on the wire (cloud VM, loopback, `crates/client/tests/replication.rs`): an observer with 20 bots walking circles nearby receives **27.4 kbit/s** of QUIC UDP payload in steady state (sends 15.7 kbit/s); UDP/IP headers add ≈ 28 B per packet.

## Traffic estimates (from PRD §8)

- 20 moving entities nearby at 20 Hz ≈ 48 kbit/s, 50 ≈ 105 kbit/s. The target of 50 kbit/s in steady state holds with less frequent updates for distant ones.
- Login at 12 chunks: 625 columns × 4–16 KB ≈ 2.4–9.8 MB (assumption about column size, to be measured).

## Tests

- `tc netem` 2% loss + RTT 100 ms: the nearest 3×3 columns appear in ≤ 1 s. In CI (no root) the same path is emulated by `net::lossy::LossyRelay` — see `crates/net/tests/quic_spike.rs` and the [T0-11 measurements](../research/net-quic-spike-2026-11-19.md).
- 500 bot clients per node: network thread CPU ≤ 1 core per 10,000 packets/s.
- Fuzz test of decoding all messages (the server never panics on garbage input).
