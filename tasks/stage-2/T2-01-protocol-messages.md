# T2-01. Protocol crate: messages, versioning, handshake

- **Status:** in-progress
- **Depends on:** T0-11 (QUIC spike), T1-08 (offline identity)
- **Owner:** agent (cloud)

## Goal

The `protocol` crate from [networking-protocol.md](../../docs/architecture/networking-protocol.md): every message the client and server exchange, as Rust enums serialized with `bitcode`, with `PROTOCOL_VERSION`, the handshake, and fuzz-safe decoding.

## Deliverables

- **Channels and messages** (control, input, entities, chunks, events) and the handshake: `Hello` / `Welcome` / `Registry` (ID table) / `Spawn`. Offline identity only until T0-10; the session JWT field is reserved.
- **Encoding helpers:**
  - quantized positions (1/32 block, relative) and 8-bit angles;
  - a column record (palette sections + zstd level 1), reusing the T1-02 encoding where possible.
- **Errors:** a version mismatch is a clear error with the update hint.

## Acceptance criteria

- [ ] Round-trip tests for every message; golden byte files for the handshake (format changes are deliberate).
- [ ] A fuzz test (proptest or cargo-fuzz corpus in CI) shows decoding garbage never panics.
- [ ] Sizes measured: a typical column record, an entity delta of 20 entities, an input datagram (≤ 1,200 bytes).

## Out of scope

Mod component replication (stage 3), Microsoft sessions (T0-10).
