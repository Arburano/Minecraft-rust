//! Block edits on the client (T1-12): `set_block` → masks → relight →
//! remesh of the affected mesh chunks → render batch re-copy.
//!
//! Order inside one frame:
//! 1. [`Editor::set_block`] (or [`dig`] / [`place`]) changes the map, cancels
//!    loader jobs made stale by the edit, re-gathers and re-meshes the
//!    affected mesh chunks on the main thread and stores them in the loader,
//!    and queues the relight.
//! 2. [`Editor::process_light`] relights a capped number of sections
//!    (deferred light, owner decision for stage 1: the old light shows until
//!    the section is processed).
//! 3. [`BatchSync::collect`] + [`BatchSync::upload`] re-copy the render
//!    batches of changed mesh chunks; the renderer uploads them and the
//!    changed sections' light (`WorldLight::take_changed`).

use std::collections::{HashMap, HashSet};

use glam::{DVec3, IVec3};
use light::{LightProps, LightUpdates, WorldLight, WorldLightError};
use mesh::batch::{RenderBatch, slot};
use mesh::{ChunkVisibility, MeshChunkBuilder, MeshChunkQuads, PackedBatch, SectionNeighborhood};
use sim::physics::{PhysicsConfig, PlayerBody};
use sim::raycast::{RayHit, raycast};
use world::coords::{BlockPos, MeshChunkPos, SectionPos};
use world::map::DirtyKind;
use world::registry::Collision;
use world::{BlockStateId, ColumnPos, Section, WorldError, WorldMap};

use crate::loading::Loader;

/// Applies block edits and keeps meshes and light in step.
pub struct Editor {
    builder: Box<MeshChunkBuilder>,
    quads: MeshChunkQuads,
    light: LightUpdates,
    props: Box<dyn Fn(BlockStateId) -> LightProps>,
    dirty: Vec<SectionPos>,
}

impl Editor {
    /// An editor for a world with `map`'s registry.
    #[must_use]
    pub fn new(map: &WorldMap) -> Self {
        Self {
            builder: Box::new(MeshChunkBuilder::new()),
            quads: MeshChunkQuads::default(),
            light: LightUpdates::new(),
            props: Box::new(light::props_from_registry(map.registry())),
            dirty: Vec::new(),
        }
    }

    /// Sets a block and re-meshes every meshed chunk whose geometry or
    /// padding holds `pos` (the loader then reports them as changed).
    /// Returns the previous state.
    pub fn set_block(
        &mut self,
        map: &mut WorldMap,
        loader: &mut Loader,
        pos: BlockPos,
        id: BlockStateId,
    ) -> Result<BlockStateId, WorldError> {
        let before = map.get_block(pos).map_or(LightProps::AIR, &self.props);
        let old = map.set_block(pos, id)?;
        // This editor handles meshing and light itself; the save marks stay
        // for the storage writer.
        for kind in [DirtyKind::Mesh, DirtyKind::Light] {
            self.dirty.clear();
            map.take_dirty(kind, &mut self.dirty);
        }
        if old == id {
            return Ok(old);
        }
        self.light.record(pos, before);
        loader.block_changed(pos);
        let registry = std::sync::Arc::clone(map.registry());
        for &m in mesh::affected_mesh_chunks(pos).as_slice() {
            // Chunks not meshed yet get their mesh from a loader job, which
            // reads the edited blocks.
            if !loader.has_mesh(m) {
                continue;
            }
            let neighborhood = neighborhood(map, m);
            self.builder.build_into(
                &neighborhood,
                |b| registry.props(b).transparent,
                &mut self.quads,
            );
            let vis = ChunkVisibility::from_voxels(self.builder.voxels(), |v| {
                registry.props(BlockStateId(v)).opaque
            });
            loader.store_mesh(m, self.quads.clone(), vis);
        }
        Ok(old)
    }

    /// Relights at most `max_sections` sections of queued edits.
    pub fn process_light(
        &mut self,
        map: &WorldMap,
        light: &mut WorldLight,
        max_sections: usize,
    ) -> Result<usize, WorldLightError> {
        self.light.process(map, light, &self.props, max_sections)
    }

    /// Edits waiting for relighting.
    #[must_use]
    pub fn pending_light(&self) -> usize {
        self.light.len()
    }
}

/// The 4×4×4 sections around a mesh chunk, copied from the map (missing ones
/// are air).
fn neighborhood(map: &WorldMap, m: MeshChunkPos) -> SectionNeighborhood {
    SectionNeighborhood::from_fn(m, |sp| {
        map.column(ColumnPos { x: sp.x, z: sp.z })
            .and_then(|col| col.section(sp.y))
            .cloned()
            .unwrap_or_else(Section::new)
    })
}

/// The block the player looks at: the first non-air cell that is not a swim
/// block, within reach of the eye.
#[must_use]
pub fn pick(
    map: &WorldMap,
    eye: DVec3,
    dir: DVec3,
    reach: f64,
    swim: &[BlockStateId],
) -> Option<RayHit> {
    raycast(eye, dir, reach, |p| {
        map.get_block(p)
            .is_some_and(|id| id != BlockStateId::AIR && !swim.contains(&id))
    })
}

/// Breaks the picked block (sets air). Returns the broken block's position.
pub fn dig(
    editor: &mut Editor,
    map: &mut WorldMap,
    loader: &mut Loader,
    hit: &RayHit,
) -> Result<BlockPos, WorldError> {
    editor.set_block(map, loader, hit.block, BlockStateId::AIR)?;
    Ok(hit.block)
}

/// Who places a block: the player's body and movement config (no block may
/// be placed into the player) and the swim blocks (replaceable like air).
#[derive(Clone, Copy)]
pub struct Placer<'a> {
    /// The player's body.
    pub body: &'a PlayerBody,
    /// Movement config (box size).
    pub cfg: &'a PhysicsConfig,
    /// Blocks a placed block may replace besides air.
    pub swim: &'a [BlockStateId],
}

/// Places `id` against the picked face. Refuses (returns `Ok(None)`) when the
/// cell is occupied by a non-replaceable block or a colliding block would
/// overlap the player.
pub fn place(
    editor: &mut Editor,
    map: &mut WorldMap,
    loader: &mut Loader,
    hit: &RayHit,
    id: BlockStateId,
    placer: Placer<'_>,
) -> Result<Option<BlockPos>, WorldError> {
    let at = hit.place_pos();
    if hit.normal == IVec3::ZERO {
        return Ok(None);
    }
    let current = map
        .get_block(at)
        .ok_or(WorldError::NotLoaded(ColumnPos::of_block(at)))?;
    if current != BlockStateId::AIR && !placer.swim.contains(&current) {
        return Ok(None);
    }
    if map.registry().props(id).collision != Collision::None {
        let b = placer.body.aabb(placer.cfg);
        let min = DVec3::new(f64::from(at.x), f64::from(at.y), f64::from(at.z));
        let max = min + DVec3::ONE;
        let overlaps = (0..3).all(|a| b.max[a] > min[a] && b.min[a] < max[a]);
        if overlaps {
            return Ok(None);
        }
    }
    editor.set_block(map, loader, at, id)?;
    Ok(Some(at))
}

/// Keeps one render batch (2×2×2 mesh chunks) per batch position in step
/// with the loader's meshes and re-packs the changed ones for upload.
#[derive(Default)]
pub struct BatchSync {
    batches: HashMap<IVec3, RenderBatch>,
    dirty: HashSet<IVec3>,
    changed: Vec<MeshChunkPos>,
    packed: PackedBatch,
}

/// The render batch holding a mesh chunk (units of 64 blocks).
#[must_use]
pub fn batch_of(m: MeshChunkPos) -> IVec3 {
    IVec3::new(m.x, m.y, m.z).div_euclid(IVec3::splat(2))
}

impl BatchSync {
    /// Empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Copies changed mesh chunks into their batches (call after the loader
    /// tick and edits; needs [`Loader::track_mesh_changes`]).
    pub fn collect(&mut self, loader: &mut Loader) {
        self.changed.clear();
        loader.take_changed_meshes(&mut self.changed);
        let empty = MeshChunkQuads::default();
        for &m in &self.changed {
            let b = batch_of(m);
            let local = IVec3::new(m.x, m.y, m.z) - b * 2;
            let quads = loader.meshes().get(&m).unwrap_or(&empty);
            self.batches.entry(b).or_default().set_chunk(
                slot(local.x as usize, local.y as usize, local.z as usize),
                quads,
            );
            self.dirty.insert(b);
        }
    }

    /// Batches waiting for upload.
    #[must_use]
    pub fn pending(&self) -> usize {
        self.dirty.len()
    }

    /// Packs every changed batch and hands it to `upload` (an empty packed
    /// batch means "remove"); empty batches are forgotten afterwards.
    pub fn upload(&mut self, mut upload: impl FnMut(IVec3, &PackedBatch)) {
        for b in self.dirty.drain() {
            let Some(batch) = self.batches.get(&b) else {
                continue;
            };
            batch.pack_into(&mut self.packed);
            upload(b, &self.packed);
            if batch.is_empty() {
                self.batches.remove(&b);
            }
        }
    }
}
