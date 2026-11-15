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

## Entity deltas

- For each client the server keeps the number of the last acknowledged snapshot; a delta = changes relative to it.
- Quantization: positions 1/32 block in relative coordinates, angles 8 bits.
- Interest management: only entities in the visibility zone; distant ones less often.

## Traffic estimates (from PRD §8)

- 20 moving entities nearby at 20 Hz ≈ 48 kbit/s, 50 ≈ 105 kbit/s. The target of 50 kbit/s in steady state holds with less frequent updates for distant ones.
- Login at 12 chunks: 625 columns × 4–16 KB ≈ 2.4–9.8 MB (assumption about column size, to be measured).

## Tests

- `tc netem` 2% loss + RTT 100 ms: the nearest 3×3 columns appear in ≤ 1 s.
- 500 bot clients per node: network thread CPU ≤ 1 core per 10,000 packets/s.
- Fuzz test of decoding all messages (the server never panics on garbage input).
