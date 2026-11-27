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
client ─ Hello { protocol, client_version, session_jwt } ─► gateway/server
server ─ Welcome { server_info, mod_manifest_hash } ──────► client
client: if the mod manifest differs — download via the menu (see client-menu-and-updater.md)
server ─ Registry { id_table, component_schemas } ────────► client
server ─ Spawn { pos, tick } + first 3×3 columns ─────────► client   (player control)
then in the background: the remaining columns by distance, then LOD
```

The "login ≤ 5 s" metric is counted up to the moment of controlling the character (columns 3–4 around); the rest is downloaded in the background.

## Messages and encoding (T2-01)

Implemented in the `protocol` crate (`crates/protocol`); the transport that carries them is `net`.

| Channel | Client → server | Server → client |
| --- | --- | --- |
| control (bi stream) | `Hello`, then `ClientControl::{Ready, Disconnect}` | `ServerHello::{Accepted(Welcome), Rejected(Rejection)}`, then `ServerControl::{Registry, Spawn, Kick}` |
| input (datagram) | `InputDatagram { ack_snapshot, frames }` — the last 2–3 `InputFrame`s (tick, button bits, `i8` move axes, `u8` fly speed, 16-bit yaw and pitch); at most 8 frames | — |
| entities (datagram) | — | `EntityDelta { snapshot, baseline, tick, ack_input, spawns, updates, despawns }` |
| chunks (uni stream per column) | — | `ColumnData { x, z, min_section_y, section_count, payload }`: the T1-02 section encoding, concatenated bottom to top, zstd level 1, in the server's block IDs (the client remaps through `Registry::id_table`). No light (ADR-0011) |
| events (bi stream) | `ClientEvent::Chat` | `ServerEvent::{Chat, System}` |

- **Bodies** are `bitcode`. **Stream frames** are `u32` LE length + body, at most 4 MiB; a longer length is rejected before allocating. **Datagrams** are at most 1,200 bytes; `EntityDelta::encode` fails above that and the server splits the snapshot.
- **Handshake prefix.** The first frame in each direction starts with `VXLH` + `PROTOCOL_VERSION` (`u32` LE), then the body. The prefix layout never changes, so a peer of another version gets `HandshakeError::VersionMismatch` with an update hint instead of a decoding error. On a mismatch the server answers with the prefix alone. Golden bytes of every handshake frame are in `crates/protocol/tests/golden/`; changing them requires a version bump.
- **Identity.** `Hello` carries `ClaimedIdentity::Offline { name }`; the server checks it with `auth::AdmissionPolicy` and derives the ID. `session_jwt` is reserved and must be `None` until T0-10 (open question 4).
- **Quantization** (`protocol::quant`): positions `FixedPos` in 1/32 block (`i32` per axis), deltas `PosDelta` (`i16`, ±1,024 blocks; farther moves are sent as `PosUpdate::Absolute`); entity angles 8 bits, the player's own view 16 bits.
- **Garbage input.** Every decoder returns an error, never panics; text fields, input frame counts, section counts and the zstd output size are bounded. Covered by `crates/protocol/tests/fuzz.rs` (random bytes, mutated valid messages, every truncation, a zstd bomb).
- **Measured sizes** (deterministic; `cargo test --release -p protocol --test sizes -- --nocapture`): input datagram of 3 ticks 47 B; entity delta of 20 moving entities 179 B; real overworld column median 1.69 KB (p95 2.22 KB) — see [performance budgets](../process/performance-budgets.md).

## Server, join and column streaming (T2-02)

Implemented in `crates/server` (lib + binary) and `net::client`; the client side is `client::session::Session::connect`.

- **Threads.** A tokio runtime (2 workers) runs QUIC with one task per connection. The **world thread** owns the block map, the players and what each client has been sent; it ticks at 20 TPS and also wakes on every message. Columns come from the **storage thread** (owns `world::WorldStore`; saved columns, player files `players/<id>.toml`) or from **generation jobs** on a `JobPool`; a job also packs the `ColumnData` frame, so the world thread only moves finished bytes. Storage answers and job results go to the world thread's own channel, so a finished column goes out at once instead of at the next tick. One region for now (T2-06).
- **Join.** `Hello` → admission (`auth::AdmissionPolicy`, name rules, no session token) → the world admits the player at once (`AlreadyConnected`, `ServerFull`) and starts loading the spawn area → `Welcome` + `Registry` → client `Ready` → the world places the player (a new player stands on the ground of column (0, 0), a returning one where it left) → `Spawn` → columns. Generating the spawn column overlaps the handshake round trips.
- **Streaming.** The server streams every column within Chebyshev distance `view_distance` of the player's column, one framed `ColumnData` per unidirectional stream, nearest first (`SendStream::set_priority` by squared distance), at most **16 streams in flight** per client; a stream counts until the client acknowledged it, so the limit bounds data in flight. When the player moves, columns farther than `view_distance + 1` are forgotten (resent on return) and pending ones are dropped. The client keeps columns up to `view_distance + 2`, one column of slack for prediction differences. The client lights and meshes up to `view_distance − 2` (ADR-0011).
- **Input.** The client sends every tick an `InputDatagram` with the last 3 frames; it predicts with the dequantized frame, which is exactly what the server simulates (`sim::physics::step` on the server's map). The server applies new ticks only, at most 4 per tick, and waits while its own 3×3 around the player is loading. Reconciliation is T2-03.
- **Limits** (the server never panics on client input): `Hello` ≤ 16 KiB, control frames after the handshake ≤ 1 KiB, every length checked before allocating; the client may open only the control and events streams (`max_concurrent_uni_streams = 0`, bidi ≤ 2); input datagrams ≤ 60/s with a burst of 30, more is dropped and a client that keeps flooding is closed; timeouts for `Hello` (10 s), admission (10 s), `Ready` (60 s) and placement (30 s); idle connections close after 30 s (keep-alive 5 s). Close codes: `server::conn::close`.
- **Certificates.** An offline server makes a self-signed certificate at start; an offline client accepts it without checking the issuer, still verifying the TLS handshake signatures (encrypted, not authenticated). Offline mode is development and private LAN only (legal.md rule 7). Real certificates: open question 16.
- **Run it.** `cargo run -p server -- --offline [--bind <LAN address>] [--world DIR] [--view-distance N]`, then `cargo run -p client -- --offline-name NAME --connect 127.0.0.1:47777`, or headless `cargo run -p client --example net_join -- 127.0.0.1:47777` for the login numbers.

## Entity deltas

- For each client the server keeps the number of the last acknowledged snapshot; a delta = changes relative to it.
- Quantization: positions 1/32 block in relative coordinates, angles 8 bits.
- Interest management: only entities in the visibility zone; distant ones less often.

## Traffic estimates (from PRD §8)

- 20 moving entities nearby at 20 Hz ≈ 48 kbit/s, 50 ≈ 105 kbit/s. The target of 50 kbit/s in steady state holds with less frequent updates for distant ones.
- Login at 12 chunks: 625 columns × 4–16 KB ≈ 2.4–9.8 MB (PRD assumption). Measured in T2-01: real overworld columns are 1.69 KB median on the wire (zstd 1, no light), so ≈ 1.07 MB.
- 20 moving entities: measured 179 B per delta (T2-01) ≈ 36 kbit/s at 20 Hz including packet overhead, inside the 48 kbit/s estimate.

## Tests

- `tc netem` 2% loss + RTT 100 ms: the nearest 3×3 columns appear in ≤ 1 s. In CI (no root) the same path is emulated by `net::lossy::LossyRelay` — see `crates/net/tests/quic_spike.rs` and the [T0-11 measurements](../research/net-quic-spike-2026-11-19.md).
- 500 bot clients per node: network thread CPU ≤ 1 core per 10,000 packets/s.
- Fuzz test of decoding all messages (the server never panics on garbage input).
- T2-02: `crates/server/tests/robustness.rs` (garbage handshakes, 4 GiB frame lengths, other versions, rejections, garbage and flooded datagrams, oversized control frames — the server keeps serving) and `crates/client/tests/multiplayer.rs` (join over `LossyRelay` 2% / RTT 100 ms, walking streams new columns and unloads old ones, rejoin where the player left).
