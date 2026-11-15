# World: coordinates, sections, palette, storage

Crate: `world`. ADR: 0004 (16³ sections with a palette).

## Coordinates

| Type | Size | Computed from block |
| --- | --- | --- |
| `BlockPos` | 1 block | — |
| `SectionPos` | 16³ | `div_euclid(16)` |
| `MeshChunkPos` | 32³ | `div_euclid(32)` |
| Column | 16 × height × 16 | `(x.div_euclid(16), z.div_euclid(16))` |
| Shard cell | 512 × height × 512 (configurable) | `div_euclid(512)` |

Always `div_euclid`/`rem_euclid`, never `/` and `%` for coordinates: block −1 lies in section −1. The index within a section is YZX order (`(y*16 + z)*16 + x`). Implemented in `crates/world/src/coords.rs` with tests.

## Section

```text
enum SectionBlocks {
    Single(BlockStateId),                       // uniform: 2 bytes + tag
    Paletted { palette: Vec<BlockStateId>,      // 2..=256 states
               bits: u8,                         // 1,2,3,4,5,6,7,8
               data: Vec<u64>,                   // indices that do not cross word boundaries
               reverse: Option<HashMap<BlockStateId, u8>> }, // only if palette.len() > 16
    Direct(Box<[BlockStateId; 4096]>),          // > 256 states
}
```

Rules:

- Indices do not cross a `u64` boundary (MC 1.16+ format): a word holds `64 / bits` indices, the remaining bits are empty. Simpler and faster access (test: `get` ≈ 3.8 ns).
- Bit width grows when the palette overflows; compaction (repacking) happens only in `PostTick` or on save, not on every `set`.
- A reverse hash index for palettes > 16 entries. Without it, `set` in an 8-bit section takes 67 ns due to linear search; the target is ≤ 10 ns at any bit width.
- Bulk unpacking (`unpack_into(&mut [BlockStateId; 4096])`) word by word is the basis of fast gather for meshing. Target: gather + masks ≤ 100 µs per 32³ mesh chunk (estimate from 62³ ≤ 0.5 ms, see analysis §13.2).
- The opaque block counter and the `all_air`/`all_opaque` flags are updated on `set`: this lets the mesher skip empty and solid mesh chunks without unpacking.

Measured sizes (test, `docs/research/optimization-tests.md`):

| Section | Bytes |
| --- | --- |
| Single | 56 (with overhead) |
| Surface, 3 bit | 1,640 |
| Dungeon, 4 bit | 2,124 |
| 8 bit | 4,664 |
| Worst case, direct | 6,616 (8,192 for a flat u16) |

## Light in a section

Separate from blocks: `SectionLight { block: Nibbles, sky: Nibbles }`, each is `Single(u8)` or 2,048 bytes. Details in [lighting.md](lighting.md).

## Registries and the ID table

- Every block, item, entity has a string ID `namespace:path`.
- At startup the server builds a registry from all mods (deterministic order: mod by id, then by declaration order) and assigns numeric `BlockStateId`s. `AIR = 0` always.
- The client receives the table on login; numeric IDs on the client are never "guessed".
- A saved world contains a `string state ID → number` table with which its sections were written. When loading with a different set of mods, sections are re-encoded through this table. A missing state → `core:missing` (a visible placeholder block; the block data is preserved so the mod can be restored without loss).

## Column and loading

- Column: an array of sections by height + a height map (for sky and LOD) + block entities in a separate map.
- Column loading states: `Requested → Generating/Loading → Lit → Ready → Unloading`. Meshing and simulation only for `Ready`.

## On-disk storage

- One file per region file of 32×32 columns (`r.X.Z.bin`).
- Header: column offset table, format version, ID table.
- Column: palette sections (as in memory) + light + entities, zstd level 3 compression. Backups — level 9.
- Copy-on-write writes: the new column version is written to free space, the offset is updated atomically; the tick does not stop.
- World index and metadata — `redb`.
- The format is versioned; migrations are separate `vN → vN+1` functions with tests on golden files.

## Network format

The same palette sections + zstd level 1. A zstd dictionary only after a test on real worlds (synthetic tests showed it hurts on whole columns).

## Open numbers

- The real bit-width histogram on a real world (currently 17 MiB per 15,000 sections — an assumed mix). A stage 1 task.
- The real compressed column size (login time and traffic depend on it).
