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
