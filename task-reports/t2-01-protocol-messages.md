# T2-01: protocol messages

## What was done

The `protocol` crate (was a skeleton) now holds every message of the five channels in `networking-protocol.md`, serialized with `bitcode` 0.6.9 (already in `[workspace.dependencies]`, first use):

- `control.rs`: the fixed 8-byte `HandshakeHeader` (magic `VXEN` + `PROTOCOL_VERSION`), `ClientControl::{Hello, Leave}`, `ServerControl::{Welcome, Registry, Spawn, Kick}`. The session JWT field is reserved (`Option`, `None` until T0-10).
- `input.rs`: `InputDatagram` (acknowledged snapshot + 1–8 consecutive intents; buttons as bits, 16-bit yaw/pitch, fly-speed steps).
- `entities.rs`: `EntityDatagram` with optional per-field updates, `RelPos` positions (1/32 block, relative to an origin, ±1,024 blocks) and 8-bit angles.
- `chunks.rs`: `ChunkStream::Column(ColumnPayload)`: the T1-02 column record (`world::storage::encode_column`) with zstd level 1; bounded incremental decompression (≤ 8 MiB), position check.
- `events.rs`: chat, dig/place with sequence numbers, block changes, block acks, column unloads.
- `frame.rs`: `u32` length-prefixed frames for the control and events streams, with the length checked before the body arrives.
- `wire.rs`: `NetId`, `ColumnXz`, `BlockXyz` (conversions to `world` types), `RelPos`, `Angle8`, `Angle16`.
- `lib.rs`: `PROTOCOL_VERSION = 1`, `ALPN`, `MAX_DATAGRAM = 1200`, `ProtocolError` (version mismatch with an update hint), and the `Message` trait: size limit → `bitcode` decode → `validate()`. `encode()` runs the same validation, so a sender cannot produce what a receiver rejects.

Tests (`cargo test -p protocol`): 6 unit tests (quantization, angles, framing, error text), `tests/messages.rs` (round trip of every message, plain and framed; column payload round trip; wrong position; handshake header versions and bad magic; invalid values refused on both sides; golden files for header, Hello, Welcome, Registry, Spawn, Kick), `tests/fuzz.rs` (proptest, 2,000 random byte strings and 2,000 corrupted/truncated valid messages into every decoder, incl. the column decompressor), `tests/sizes.rs` (sizes below; asserts the datagrams fit).

Measured sizes (deterministic, release build, `cargo test -p protocol --release --test sizes -- --nocapture`):

- input datagram: 14 B (1 intent), 32 B (3), 48 B (8);
- entity delta of 20 entities with every field changed: 252 B → 40.3 kbit/s at 20 Hz, payload only; 99 such entities fit one datagram;
- 64 generated overworld columns (seed 7, 24 sections, no light): raw record mean 9.4 KB (6.8–11.0 KB), on the wire mean 1.6 KB (0.87–2.3 KB); login at 12 chunks ≈ 1.0 MB.

Docs updated: `networking-protocol.md` (implementation section and sizes), `performance-budgets.md` (traffic row), `open-questions.md` (No. 7, column size), task status.

`cargo fmt --all`, `cargo clippy --workspace --all-targets` with `-D warnings` and `cargo test --workspace` run on the cloud VM (4 vCPU Xeon 2.1 GHz).

## What went wrong

- First build failed on `world::BlockPos` (it lives in `world::coords`) and a missing `Default` on `NetId`; fixed.
- A test compared `Result<ColumnRecord, _>` with `assert_eq!`, but `ColumnRecord` has no `Debug`/`PartialEq`; switched to `matches!`.
- One clippy `precedence` lint in test data; fixed.

## Issues encountered

- **Protocol version lives in the fixed header, not in `Hello`.** The spec's handshake sketch had `Hello { protocol, … }`. If the version were inside a `bitcode` message, a build with a different `Hello` layout could fail to decode it and never get to the version check. The spec now describes the header.
- **Block edits on the events stream.** The spec does not say where dig/place go; they are reliable events with sequence numbers, not input datagrams. T2-04 may change this (with a version bump).
- **The column encoding is tied to the world format.** Reusing `world::storage::encode_column` means a world format bump is also a protocol bump; documented in `chunks.rs` and the spec.
- **Traffic is at the limit.** 40.3 kbit/s of payload for 20 fully moving entities becomes ≈ 50–53 kbit/s with QUIC/UDP/IP overhead (estimated, not measured). The lower update rate for distant entities (spec) is needed; real numbers come from T2-05/T2-09.
- Mod component replication (stage 3) and LOD streams (stage 5) are not defined yet; `component_schema_hash` and `mod_manifest_hash` are reserved as zeros.
- `net` does not use the new messages yet; that is T2-02.
