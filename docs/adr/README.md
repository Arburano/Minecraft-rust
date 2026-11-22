# Architecture Decision Records

Every architectural decision that is costly to change is recorded here. ADRs are not edited after acceptance: a new decision is a new ADR with the status "Supersedes ADR-NNNN", and the old one gets the status "Superseded".

An ADR is needed when:

- a new link between crates goes against the dependency direction;
- a change of on-disk or network format, or a breaking WIT change;
- `unsafe`, a new heavy dependency, or a stack change;
- a decision that conflicts with the PRD or an existing ADR.

Template — [0000-template.md](0000-template.md).

| No. | Decision | Status |
| --- | --- | --- |
| [0001](0001-own-engine-not-port.md) | Own engine, not a port of Minecraft | Accepted |
| [0002](0002-bevy-ecs-only.md) | `bevy_ecs` without the rest of Bevy | Accepted |
| [0003](0003-wgpu-own-renderer.md) | Own renderer on wgpu with three paths | Accepted |
| [0004](0004-sections-16-palette.md) | 16³ sections with a palette | Accepted |
| [0005](0005-mesh-chunk-32-with-batches.md) | 32³ mesh chunk + 2×2×2 render batches | Accepted |
| [0006](0006-wasm-component-model-epoch.md) | WASM Component Model, epoch, host-side state | Accepted |
| [0007](0007-quic-own-protocol.md) | QUIC and own protocol | Accepted |
| [0008](0008-region-threading.md) | Regional multithreading | Accepted |
| [0009](0009-sharding-ownership-number.md) | Sharding with ownership number | Accepted |
| [0010](0010-in-game-launcher-and-updater.md) | In-game launcher menu + separate updater | Accepted |
| [0011](0011-client-light-server-lod.md) | Light on the client, LOD from the server | Accepted |
| [0012](0012-no-std-guest-allocator.md) | Isolated canonical-ABI allocator for the no_std guest | Accepted |
| [0013](0013-replaceable-movement-and-worldgen.md) | Replaceable movement and world generation (gameplay slots) | Proposed |
