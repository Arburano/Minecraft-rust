//! Coordinates, block-state IDs, palette sections 16^3, chunk columns, registries and
//! the persisted ID table.
//!
//! Spec:
//! - `docs/architecture/world-storage.md`
//!
//! Status: coordinates, `Section` palette storage (T0-03), block-state registry
//! with the persisted ID table, columns with a sky heightmap and the lock-free
//! `WorldMap` of loaded columns with dirty tracking (T1-01).

pub mod column;
pub mod coords;
pub mod map;
pub mod registry;
pub mod section;

pub use column::{Column, ColumnPos, ColumnState, WorldBounds};
pub use map::{DirtyKind, WorldError, WorldMap};
pub use registry::{
    BlockDef, BlockProps, Collision, IdTable, Registry, RegistryBuilder, RegistryError,
};
pub use section::Section;

/// Edge length of a storage section in blocks (16^3 = 4096 blocks).
pub const SECTION_SIZE: i32 = 16;
/// Blocks in one section.
pub const SECTION_VOLUME: usize = 4096;
/// Edge length of a mesh chunk in blocks (2x2x2 sections). Decided in ADR-0005.
pub const MESH_CHUNK_SIZE: i32 = 32;
/// Mesh chunks per render batch edge (2x2x2 mesh chunks = 64^3). Decided in ADR-0005.
pub const RENDER_BATCH_MESH_CHUNKS: i32 = 2;
/// Default world height in blocks (24 sections). Configurable per dimension.
pub const DEFAULT_WORLD_HEIGHT: i32 = 384;

/// Runtime numeric ID of a block state. Assigned by the server per session from the
/// registry; persisted worlds store a string-ID table instead (see world-storage.md).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockStateId(pub u16);

impl BlockStateId {
    /// Air is always 0 in every registry.
    pub const AIR: BlockStateId = BlockStateId(0);
}

// Compile-time invariants (ADR-0004, ADR-0005).
const _: () = assert!(SECTION_VOLUME == (SECTION_SIZE * SECTION_SIZE * SECTION_SIZE) as usize);
const _: () = assert!(
    MESH_CHUNK_SIZE % SECTION_SIZE == 0,
    "mesh chunk must be whole sections"
);
// binary-greedy-meshing packs quad coordinates in 6 bits (0..=63).
const _: () = assert!(MESH_CHUNK_SIZE * RENDER_BATCH_MESH_CHUNKS <= 64);
