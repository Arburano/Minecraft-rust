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
struct Section { blocks: SectionBlocks, non_air: u16 }   // 56 bytes total

enum SectionBlocks {
    Single(BlockStateId),                       // uniform
    Paletted { palette: Box<[BlockStateId]>,    // 2..=256 states
               bits: u8,                         // 1,2,3,4,5,6,7,8
               data: Box<[u64]>,                 // indices that do not cross word boundaries
               reverse: Option<Box<ReverseIndex>> }, // only if palette.len() > 16
    Direct(Box<[BlockStateId; 4096]>),          // > 256 states, flat u16
}
```

The palette, packed words and reverse index are boxed so the whole `Section`
stays at 56 bytes (the measured `Single` size below). `ReverseIndex` is a compact
open-addressing table (one `u32` per slot: key + palette index + occupancy bit,
multiply-shift hash), not a std `HashMap`; see the rules below.

Rules:

- Indices do not cross a `u64` boundary (MC 1.16+ format): a word holds `64 / bits` indices, the remaining bits are empty. Simpler and faster access (measured `get` ≈ 3.4 ns on the test VM).
- Bit width grows when the palette overflows; compaction (repacking) happens only in `PostTick` or on save (`Section::compact`), not on every `set`.
- A reverse index for palettes > 16 entries keeps `set` O(1). Without it, `set` in an 8-bit section takes ~67 ns (linear scan). The index is a small open-addressing table with interleaved key/index entries: separate key and value arrays caused 4K-aliasing stalls against the 4 KiB packed data of an 8-bit section (~260 ns/op on the test VM); one interleaved array removes them. Measured `set` ≈ 6 ns typical (VM; see [performance-budgets.md](../process/performance-budgets.md) and the T0-03 report for the aliasing caveat).
- Packed index addressing (T1-04): the word and bit offset of index `i` at width `bits` come from a table of `64 / bits` and a multiply-shift reciprocal (`(i * MAGIC[bits]) >> 20`), not from two run-time integer divisions per access, which are slow on Zen+ (the T0-16 bare-metal `get`/`set` misses). Exhaustively tested for every width and index.
- Bulk unpacking (`unpack_into(&mut [BlockStateId; 4096])`) word by word is the basis of fast gather for meshing (measured ≈ 3 µs per section on the test VM). Target: gather + masks ≤ 100 µs per 32³ mesh chunk (estimate from 62³ ≤ 0.5 ms, see analysis §13.2).
- The `non_air` counter and the `all_air` flag are updated on `set` and let the mesher skip empty mesh chunks without unpacking. Opacity is registry/mod data, so `all_opaque` takes the caller's `is_opaque` predicate (the engine does not know block semantics); together with the counter it lets the mesher skip solid mesh chunks.

Measured sizes (test, `docs/research/optimization-tests.md`; reproduced by the
`storage_sizes_match_spec_table` unit test). Bytes = the 56-byte `Section` plus
palette (`len * 2`) and packed words (`words * 8`); allocator overhead and the
optional reverse index are excluded:

| Section | Bytes |
| --- | --- |
| Single | 56 (with overhead) |
| Surface, 3 bit | 1,640 |
| Dungeon, 4 bit | 2,124 |
| 8 bit | 4,664 |
| Worst case, direct (flat u16) | 8,248 (8,192 data + 56) |

`Direct` is implemented as the flat `Box<[BlockStateId; 4096]>` from the enum
above (8,192 B of data). The 6,616 B figure in the research prototype was a 12-bit
packed direct representation, which is not used here.

## Light in a section

Separate from blocks: `SectionLight { block: Nibbles, sky: Nibbles }`, each is `Single(u8)` or 2,048 bytes. Details in [lighting.md](lighting.md).

## Registries and the ID table

- Every block, item, entity has a string ID `namespace:path`.
- At startup the server builds a registry from all mods (deterministic order: mod by id, then by declaration order) and assigns numeric `BlockStateId`s. `AIR = 0` always.
- The client receives the table on login; numeric IDs on the client are never "guessed".
- A saved world contains a `string state ID → number` table with which its sections were written. When loading with a different set of mods, sections are re-encoded through this table. A missing state → `core:missing` (a visible placeholder block; the block data is preserved so the mod can be restored without loss).
- Implementation (T1-01, `world::registry`): `core:air` = 0 and `core:missing` = 1 are provided by the engine (a mod may redefine their properties); then mods sorted by id, blocks in declaration order (`data/blocks/*.toml` in file-name order), states in canonical order — properties sorted by name, the last one varying fastest, canonical name `ns:path[a=x,b=y]`. More than 65,536 states is a load error. The registry is immutable after startup and shared as `Arc<Registry>` (no locks). `Registry::remap(&IdTable)` re-encodes saved numeric IDs. Engine-relevant properties per state: `opaque`, `transparent`, `light_emission`, `light_opacity` (default 15 for opaque blocks, 0 otherwise), `collision`.

## Column and loading

- Column: an array of sections by height + a height map (for sky and LOD) + block entities in a separate map.
- Column loading states: `Requested → Generating/Loading → Lit → Ready → Unloading`. Meshing and simulation only for `Ready`.
- Implementation (T1-01, `world::column`, `world::map`):
  - Default dimension bounds `WorldBounds { min_section_y: -4, section_count: 24 }` = y `-64..320` (configurable per dimension).
  - Heightmap per block column: the world y from which the column is open to the sky (topmost block with `light_opacity > 0`, plus one); maintained on every `set`, recomputed after generation/loading. Used by sky light (`lighting.md`).
  - `WorldMap` owns the loaded columns of a dimension on one thread (no locks); worker jobs get section snapshots. Columns live in a direct-mapped 64×64 slot grid (`x mod 64, z mod 64`), colliding columns in an overflow hash map, so a loaded area up to 1,024 blocks wide needs no hashing.
  - Dirty tracking per consumer (`Mesh`, `Light`, `Save`): `set_block` queues the edited section once per consumer; `take_dirty` drains it. Mesh consumers add border neighbors themselves (`mesh::affected_mesh_chunks`).
  - Light is not part of `Column`: it lives in the `light` crate under the same `SectionPos` key (the dependency direction is `light → world`).

## On-disk storage

- One file per region file of 32×32 columns (`r.X.Z.bin`).
- Header: column offset table, format version, ID table.
- Column: palette sections (as in memory) + light + entities, zstd level 3 compression. Backups — level 9.
- Copy-on-write writes: the new column version is written to free space, the offset is updated atomically; the tick does not stop.
- World index and metadata — `redb`.
- The format is versioned; migrations are separate `vN → vN+1` functions with tests on golden files.
- Implementation, format v1 (T1-02, `world::storage`):
  - Region file `<world>/region/r.X.Z.bin`: magic `VXRG`, u32 format version, 1,024 header entries `{offset u64, length u32, FNV-1a-32 checksum u32}` (16,392-byte header). Column records are zstd-3 compressed and appended; a single save writes the record, syncs, rewrites the 16-byte entry and syncs; `save_all` batches a region (append all, sync, update all entries, sync). A crash leaves each column at its old or new version; a torn record fails its checksum (tested). "Free space" is the end of the file; old versions become garbage reclaimed by `compact` (temporary file + rename).
  - Column record: u32 format version, u32 ID-table number, x, z, min section y, section count, sections (`Section::encode`: tag + single u16 / bits + palette + packed words / 4,096 u16), then two length-prefixed blobs — light (owned by the `light` crate) and entities — so `world` keeps its dependency direction. Decoding validates every packed index against its palette.
  - ID tables live in `<world>/world.redb` (table `id_tables`: number → newline-joined state names; `meta`: format version) rather than in each region header, and every record names its table: a world saved under several mod sets stays readable, and columns are remapped on load (`Registry::remap`, unknown → `core:missing`). The heightmap is recomputed on load, not stored.
  - Golden file: `crates/world/tests/golden/r.0.0.bin` (16 corpus columns).

## Network format

The same palette sections + zstd level 1. A zstd dictionary only after a test on real worlds (synthetic tests showed it hurts on whole columns).

## Open numbers

- The real bit-width histogram on a real world (currently 17 MiB per 15,000 sections — an assumed mix). A stage 1 task.
- The real compressed column size (login time and traffic depend on it).
