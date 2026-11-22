//! Light integrated with the world (T1-09, `docs/architecture/lighting.md`).
//!
//! - [`WorldLight`]: light per `SectionPos`, kept apart from the blocks (the
//!   dependency direction is `light → world`), with the column blob format used
//!   by `world::storage::ColumnRecord::light`.
//! - [`snapshot_from_world`]: a 3×3×3 snapshot built from the `WorldMap`, with
//!   the stored light imported and the one-cell layer around it **frozen** at
//!   the world's light. An edit in the center section reaches at most 15
//!   blocks, so it cannot change any cell outside the snapshot; with the outside
//!   light fixed, the incremental update is exact for all 27 sections.
//! - [`light_column`]: initial light of a generated column, section by
//!   section (each the exact center of its own snapshot), with shortcuts for
//!   all-opaque and open-sky sections.
//! - [`LightUpdates`]: queued edits applied a limited number of snapshots per
//!   tick; the rest waits (the client shows the old light meanwhile).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use rayon::prelude::*;

thread_local! {
    /// One reusable snapshot per thread for the parallel downward recompute
    /// (no allocation per section after the first).
    static SCRATCH: RefCell<Option<Box<LightNeighborhood>>> = const { RefCell::new(None) };
}
use std::fmt;

use world::coords::{BlockPos, SectionPos};
use world::{BlockStateId, ColumnPos, Registry, SECTION_SIZE, SECTION_VOLUME, Section, WorldMap};

use crate::neighborhood::{EDGE, LightNeighborhood, LightProps, VOLUME};
use crate::nibbles::{MAX_LIGHT, Nibbles, PACKED_BYTES, SectionLight};

/// Errors of world lighting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorldLightError {
    /// A column the snapshot needs is not loaded.
    NotLoaded(ColumnPos),
    /// A section the snapshot imports has no light yet (initial lighting first).
    NotLit(SectionPos),
    /// A stored light blob is malformed.
    Corrupt(&'static str),
}

impl fmt::Display for WorldLightError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotLoaded(c) => write!(f, "column {c:?} is not loaded"),
            Self::NotLit(s) => write!(f, "section {s:?} has no light yet"),
            Self::Corrupt(what) => write!(f, "corrupt light data: {what}"),
        }
    }
}

impl std::error::Error for WorldLightError {}

/// Light properties of every registry state, precomputed into a table (one
/// indexed load per cell when building snapshots).
pub fn props_from_registry(registry: &Registry) -> impl Fn(BlockStateId) -> LightProps + use<> {
    let table: Vec<LightProps> = (0..registry.len())
        .map(|i| {
            let p = registry.props(BlockStateId(i as u16));
            LightProps::new(p.light_emission, p.light_opacity)
        })
        .collect();
    move |id| {
        table
            .get(usize::from(id.0))
            .copied()
            .unwrap_or(LightProps::OPAQUE)
    }
}

fn encode_nibbles(n: &Nibbles, out: &mut Vec<u8>) {
    match n {
        Nibbles::Single(v) => out.extend_from_slice(&[0, *v]),
        Nibbles::Packed(p) => {
            out.push(1);
            out.extend_from_slice(&p[..]);
        }
    }
}

fn decode_nibbles(bytes: &[u8]) -> Result<(Nibbles, usize), WorldLightError> {
    match bytes.first() {
        Some(0) => {
            let v = *bytes.get(1).ok_or(WorldLightError::Corrupt("truncated"))?;
            if v > MAX_LIGHT {
                return Err(WorldLightError::Corrupt("level above 15"));
            }
            Ok((Nibbles::Single(v), 2))
        }
        Some(1) => {
            let data = bytes
                .get(1..1 + PACKED_BYTES)
                .ok_or(WorldLightError::Corrupt("truncated"))?;
            let mut packed = Box::new([0u8; PACKED_BYTES]);
            packed.copy_from_slice(data);
            Ok((Nibbles::Packed(packed), 1 + PACKED_BYTES))
        }
        _ => Err(WorldLightError::Corrupt("nibble tag")),
    }
}

/// Light of every lit section of a dimension.
#[derive(Default)]
pub struct WorldLight {
    sections: HashMap<SectionPos, SectionLight>,
    /// Sections stored since the last `take_changed`, when tracking is on
    /// (the renderer uploads them).
    changed: Option<HashSet<SectionPos>>,
}

impl WorldLight {
    /// Empty.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Light of a section, if lit.
    #[must_use]
    pub fn get(&self, pos: SectionPos) -> Option<&SectionLight> {
        self.sections.get(&pos)
    }

    /// Stores a section's light.
    pub fn insert(&mut self, pos: SectionPos, light: SectionLight) {
        if let Some(c) = &mut self.changed {
            c.insert(pos);
        }
        self.sections.insert(pos, light);
    }

    /// Starts recording which sections get new light (off by default, so
    /// headless users do not accumulate a log nobody drains).
    pub fn track_changes(&mut self) {
        self.changed.get_or_insert_with(HashSet::new);
    }

    /// Moves the sections stored since the last call into `out` (any order).
    pub fn take_changed(&mut self, out: &mut Vec<SectionPos>) {
        if let Some(c) = &mut self.changed {
            out.extend(c.drain());
        }
    }

    /// Number of lit sections.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sections.len()
    }

    /// Whether nothing is lit.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    /// Drops the light of an unloaded column.
    pub fn remove_column(&mut self, pos: ColumnPos) {
        self.sections.retain(|s, _| s.x != pos.x || s.z != pos.z);
    }

    /// Block and sky light at a position (`None` if its section is not lit).
    #[must_use]
    pub fn at(&self, pos: BlockPos) -> Option<(u8, u8)> {
        let l = self.sections.get(&pos.section())?;
        let i = pos.index_in_section();
        Some((l.block.get(i), l.sky.get(i)))
    }

    /// The column's light as a storage blob (`ColumnRecord::light`): for each
    /// section bottom to top, a presence byte and two channels (`0, level` or
    /// `1` + 2,048 bytes).
    #[must_use]
    pub fn encode_column(&self, pos: ColumnPos, bounds: world::WorldBounds) -> Vec<u8> {
        let mut out = Vec::new();
        for i in 0..bounds.section_count as i32 {
            let sp = SectionPos {
                x: pos.x,
                y: bounds.min_section_y + i,
                z: pos.z,
            };
            match self.sections.get(&sp) {
                None => out.push(0),
                Some(l) => {
                    out.push(1);
                    encode_nibbles(&l.block, &mut out);
                    encode_nibbles(&l.sky, &mut out);
                }
            }
        }
        out
    }

    /// Restores a column's light from [`Self::encode_column`] output.
    pub fn decode_column(
        &mut self,
        pos: ColumnPos,
        bounds: world::WorldBounds,
        bytes: &[u8],
    ) -> Result<(), WorldLightError> {
        let mut at = 0;
        let mut decoded = Vec::new();
        for i in 0..bounds.section_count as i32 {
            let sp = SectionPos {
                x: pos.x,
                y: bounds.min_section_y + i,
                z: pos.z,
            };
            match bytes.get(at) {
                Some(0) => at += 1,
                Some(1) => {
                    let (block, n) = decode_nibbles(&bytes[at + 1..])?;
                    let (sky, m) = decode_nibbles(&bytes[at + 1 + n..])?;
                    at += 1 + n + m;
                    decoded.push((sp, SectionLight { block, sky }));
                }
                _ => return Err(WorldLightError::Corrupt("section presence")),
            }
        }
        if at != bytes.len() {
            return Err(WorldLightError::Corrupt("trailing bytes"));
        }
        self.sections.extend(decoded);
        Ok(())
    }
}

/// Section of `map` at `pos`: `Ok(None)` above the world, an opaque marker
/// below it, an error if the column is not loaded.
enum SectionSlot<'a> {
    Loaded(&'a Section),
    AboveWorld,
    BelowWorld,
}

fn section_at(map: &WorldMap, pos: SectionPos) -> Result<SectionSlot<'_>, WorldLightError> {
    let col = ColumnPos { x: pos.x, z: pos.z };
    let column = map.column(col).ok_or(WorldLightError::NotLoaded(col))?;
    let bounds = map.bounds();
    Ok(match column.section(pos.y) {
        Some(s) => SectionSlot::Loaded(s),
        None if pos.y >= bounds.min_section_y => SectionSlot::AboveWorld,
        None => SectionSlot::BelowWorld,
    })
}

/// Light of a section that has none stored: open air above the world is sky
/// 15; below the world is dark.
fn implicit_light(map: &WorldMap, pos: SectionPos) -> Option<(u8, u8)> {
    let b = map.bounds();
    if pos.y >= b.min_section_y + b.section_count as i32 {
        Some((0, MAX_LIGHT))
    } else if pos.y < b.min_section_y {
        Some((0, 0))
    } else {
        None
    }
}

/// Builds the snapshot around `center` from the world's blocks. With
/// `import`, the stored light of the 27 sections is loaded and the surrounding
/// layer is frozen at the world's light (both must be lit); without it, light
/// is zero (call `full_relight`; the center section is then exact).
pub fn snapshot_from_world(
    map: &WorldMap,
    light: &WorldLight,
    center: SectionPos,
    props: &impl Fn(BlockStateId) -> LightProps,
    import: bool,
) -> Result<LightNeighborhood, WorldLightError> {
    let mut snap = LightNeighborhood::from_props(
        BlockPos::new(0, 0, 0),
        &vec![LightProps::AIR; VOLUME],
        &vec![false; EDGE * EDGE],
    )
    .map_err(|_| WorldLightError::Corrupt("snapshot size"))?;
    reload_from_world(&mut snap, map, light, center, props, import)?;
    Ok(snap)
}

/// [`snapshot_from_world`] into an existing snapshot, without allocating.
pub fn reload_from_world(
    snap: &mut LightNeighborhood,
    map: &WorldMap,
    light: &WorldLight,
    center: SectionPos,
    props: &impl Fn(BlockStateId) -> LightProps,
    import: bool,
) -> Result<(), WorldLightError> {
    reload_region(snap, map, light, center, props, import, [0; 3], [EDGE; 3])
}

/// Loads only the box `lo..hi` (local cells) of the snapshot around
/// `center`; the rest stays opaque and dark. With `import`, stored light is
/// loaded inside the box and the layer just outside its six faces is frozen
/// at the world's light.
#[allow(clippy::too_many_arguments)]
fn reload_region(
    snap: &mut LightNeighborhood,
    map: &WorldMap,
    light: &WorldLight,
    center: SectionPos,
    props: &impl Fn(BlockStateId) -> LightProps,
    import: bool,
    lo: [usize; 3],
    hi: [usize; 3],
) -> Result<(), WorldLightError> {
    let origin = BlockPos::new(
        (center.x - 1) * SECTION_SIZE,
        (center.y - 1) * SECTION_SIZE,
        (center.z - 1) * SECTION_SIZE,
    );
    snap.begin_load(origin);
    snap.set_region(lo, hi);
    let s = SECTION_SIZE as usize;
    let overlaps = |d: [usize; 3]| (0..3).all(|a| d[a] * s < hi[a] && d[a] * s + s > lo[a]);
    let mut ids = [BlockStateId::AIR; SECTION_VOLUME];
    for dy in 0..3 {
        for dz in 0..3 {
            for dx in 0..3 {
                if !overlaps([dx, dy, dz]) {
                    continue;
                }
                let sp = SectionPos {
                    x: center.x + dx as i32 - 1,
                    y: center.y + dy as i32 - 1,
                    z: center.z + dz as i32 - 1,
                };
                match section_at(map, sp)? {
                    SectionSlot::Loaded(section) if section.palette_len() == 1 => {
                        snap.load_uniform(dx, dy, dz, props(section.get(0)));
                    }
                    // A section the box covers only partly is read cell by
                    // cell; a mostly covered one is unpacked whole.
                    SectionSlot::Loaded(section)
                        if snap.clip_volume([dx, dy, dz]) < SECTION_VOLUME / 2 =>
                    {
                        snap.load_cells(dx, dy, dz, |j| props(section.get(j)));
                    }
                    SectionSlot::Loaded(section) => {
                        section.unpack_into(&mut ids);
                        snap.load_ids(dx, dy, dz, &ids, props);
                    }
                    SectionSlot::AboveWorld => snap.load_uniform(dx, dy, dz, LightProps::AIR),
                    SectionSlot::BelowWorld => snap.load_uniform(dx, dy, dz, LightProps::OPAQUE),
                }
            }
        }
    }
    let top = origin.y + hi[1] as i32;
    for z in lo[2]..hi[2] {
        for x in lo[0]..hi[0] {
            let h = map
                .height(origin.x + x as i32, origin.z + z as i32)
                .ok_or(WorldLightError::NotLoaded(ColumnPos::of_block(origin)))?;
            snap.set_open_top(x, z, h <= top);
        }
    }
    snap.rebuild_sky_runs();
    if !import {
        return Ok(());
    }
    for dy in 0..3 {
        for dz in 0..3 {
            for dx in 0..3 {
                if !overlaps([dx, dy, dz]) {
                    continue;
                }
                let sp = SectionPos {
                    x: center.x + dx as i32 - 1,
                    y: center.y + dy as i32 - 1,
                    z: center.z + dz as i32 - 1,
                };
                match (light.get(sp), implicit_light(map, sp)) {
                    (Some(l), _) => snap.import_section(dx, dy, dz, l),
                    (None, Some((b, sky))) => snap.import_section(
                        dx,
                        dy,
                        dz,
                        &SectionLight {
                            block: Nibbles::Single(b),
                            sky: Nibbles::Single(sky),
                        },
                    ),
                    (None, None) => return Err(WorldLightError::NotLit(sp)),
                }
            }
        }
    }
    // Freeze the layer just outside the box's six faces at the world's light
    // (edges and corners are not face neighbors of the box, so not needed).
    // Per face, the layer is split by section; each section's light is
    // decoded once.
    let (mut fb, mut fs) = ([0u8; SECTION_VOLUME], [0u8; SECTION_VOLUME]);
    for axis in 0..3 {
        let (a1, a2) = ((axis + 1) % 3, (axis + 2) % 3);
        for side in [lo[axis] as i32 - 1, hi[axis] as i32] {
            let mut u0 = lo[a1] as i32;
            while u0 < hi[a1] as i32 {
                let u1 = ((u0.div_euclid(SECTION_SIZE) + 1) * SECTION_SIZE).min(hi[a1] as i32);
                let mut v0 = lo[a2] as i32;
                while v0 < hi[a2] as i32 {
                    let v1 = ((v0.div_euclid(SECTION_SIZE) + 1) * SECTION_SIZE).min(hi[a2] as i32);
                    let mut l = [0i32; 3];
                    l[axis] = side;
                    l[a1] = u0;
                    l[a2] = v0;
                    let sp =
                        BlockPos::new(origin.x + l[0], origin.y + l[1], origin.z + l[2]).section();
                    match (light.get(sp), implicit_light(map, sp)) {
                        (Some(st), _) => {
                            st.block.unpack_into(&mut fb);
                            st.sky.unpack_into(&mut fs);
                        }
                        (None, Some((b, sky))) => {
                            fb.fill(b);
                            fs.fill(sky);
                        }
                        (None, None) => return Err(WorldLightError::NotLit(sp)),
                    }
                    for u in u0..u1 {
                        for v in v0..v1 {
                            l[a1] = u;
                            l[a2] = v;
                            let wp =
                                BlockPos::new(origin.x + l[0], origin.y + l[1], origin.z + l[2]);
                            let k = wp.index_in_section();
                            snap.freeze_border(l[0], l[1], l[2], fb[k], fs[k]);
                        }
                    }
                    v0 = v1;
                }
                u0 = u1;
            }
        }
    }
    let _ = snap.take_touched();
    Ok(())
}

/// What initial lighting did for a column (for tests and profiling).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ColumnLightStats {
    /// Sections set without a BFS because they are opaque and non-emitting.
    pub opaque: u32,
    /// Sections set without a BFS because they are open sky with no blocks around.
    pub open_sky: u32,
    /// Sections lit with a full snapshot BFS.
    pub computed: u32,
}

/// Initial light of every section of a loaded column. Its neighbors' blocks
/// must be loaded (light within 15 blocks depends on them); their light is not
/// needed. Results are independent of the order columns are lit in.
pub fn light_column(
    map: &WorldMap,
    light: &mut WorldLight,
    pos: ColumnPos,
    props: &impl Fn(BlockStateId) -> LightProps,
) -> Result<ColumnLightStats, WorldLightError> {
    let bounds = map.bounds();
    let column = map.column(pos).ok_or(WorldLightError::NotLoaded(pos))?;
    let mut stats = ColumnLightStats::default();
    let mut snap: Option<LightNeighborhood> = None;
    // Highest heightmap value over the 3×3 columns around.
    let mut max_height = i32::MIN;
    for cz in pos.z - 1..=pos.z + 1 {
        for cx in pos.x - 1..=pos.x + 1 {
            let c = map
                .column(ColumnPos { x: cx, z: cz })
                .ok_or(WorldLightError::NotLoaded(ColumnPos { x: cx, z: cz }))?;
            for z in 0..16 {
                for x in 0..16 {
                    max_height = max_height.max(c.height(x, z));
                }
            }
        }
    }
    for i in 0..bounds.section_count as i32 {
        let sy = bounds.min_section_y + i;
        let center = SectionPos {
            x: pos.x,
            y: sy,
            z: pos.z,
        };
        let Some(section) = column.section(sy) else {
            continue;
        };
        let dark = section.all_opaque(|b| {
            let p = props(b);
            p.opacity >= MAX_LIGHT && p.emission == 0
        });
        let result = if dark {
            stats.opaque += 1;
            SectionLight {
                block: Nibbles::Single(0),
                sky: Nibbles::Single(0),
            }
        } else if max_height <= sy * SECTION_SIZE && neighbors_empty(map, center)? {
            stats.open_sky += 1;
            SectionLight {
                block: Nibbles::Single(0),
                sky: Nibbles::Single(MAX_LIGHT),
            }
        } else {
            stats.computed += 1;
            let snap = match &mut snap {
                Some(s) => {
                    reload_from_world(s, map, light, center, props, false)?;
                    s
                }
                None => snap.insert(snapshot_from_world(map, light, center, props, false)?),
            };
            snap.full_relight();
            snap.section_light(1, 1, 1)
        };
        light.insert(center, result);
    }
    Ok(stats)
}

/// Whether all 27 sections around `center` (inside the world) are air.
fn neighbors_empty(map: &WorldMap, center: SectionPos) -> Result<bool, WorldLightError> {
    for dy in -1..=1 {
        for dz in -1..=1 {
            for dx in -1..=1 {
                let sp = SectionPos {
                    x: center.x + dx,
                    y: center.y + dy,
                    z: center.z + dz,
                };
                match section_at(map, sp)? {
                    SectionSlot::Loaded(s) if !s.all_air() => return Ok(false),
                    SectionSlot::BelowWorld => return Ok(false),
                    _ => {}
                }
            }
        }
    }
    Ok(true)
}

/// Block edits waiting for relighting. Record each edit with the block's light
/// properties **before** the edit; `process` relights a limited number of
/// sections per call and leaves the rest queued.
#[derive(Default)]
pub struct LightUpdates {
    pending: Vec<(BlockPos, LightProps)>,
    /// Reused snapshot: no allocation per processed section after the first.
    scratch: Option<Box<LightNeighborhood>>,
}

impl LightUpdates {
    /// Empty queue.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an edit already applied to the world map.
    pub fn record(&mut self, pos: BlockPos, before: LightProps) {
        self.pending.push((pos, before));
    }

    /// Edits still waiting.
    #[must_use]
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Whether nothing waits.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Relights at most `max_sections` sections' worth of edits; returns how
    /// many were processed.
    ///
    /// Order: the highest pending section first, so no pending edit lies above
    /// a snapshot (its "open to the sky above" flags then match the stored
    /// light). For each section: a snapshot with stored light and a frozen
    /// border; every still-pending edit inside it reverted to its old
    /// properties (so stored light matches the blocks); this section's edits
    /// applied incrementally; all 27 sections exported.
    ///
    /// Direct sky light runs down a column without limit, so an edit that
    /// changes whether a cell passes direct sky can change light far below the
    /// snapshot. Then the 3×3 sections at each level from the snapshot's
    /// bottom ring downwards are recomputed exactly (full snapshot BFS, center
    /// section) until a whole level is unchanged.
    pub fn process(
        &mut self,
        map: &WorldMap,
        light: &mut WorldLight,
        props: &(impl Fn(BlockStateId) -> LightProps + Sync),
        max_sections: usize,
    ) -> Result<usize, WorldLightError> {
        let bounds = map.bounds();
        let mut done = 0;
        while done < max_sections && !self.pending.is_empty() {
            let center = self
                .pending
                .iter()
                .map(|(p, _)| p.section())
                .max_by_key(|s| s.y)
                .unwrap_or(SectionPos::default());
            // This section's edits, each once, in edit order.
            let mut applied: Vec<(BlockPos, LightProps)> = Vec::new();
            let mut rest = Vec::with_capacity(self.pending.len());
            for (pos, before) in std::mem::take(&mut self.pending) {
                if pos.section() != center {
                    rest.push((pos, before));
                } else if !applied.iter().any(|(p, _)| *p == pos) {
                    applied.push((pos, before));
                }
            }
            self.pending = rest;
            // Light changes from an edit reach at most 14 cells, so only a box
            // of ±15 cells around the edits is loaded, with the layer around
            // it frozen. An edit that may move a column's direct-sky run
            // (opacity flips at or above the column's sky height) extends the
            // box down to the snapshot's bottom.
            let origin = BlockPos::new(
                (center.x - 1) * SECTION_SIZE,
                (center.y - 1) * SECTION_SIZE,
                (center.z - 1) * SECTION_SIZE,
            );
            let (mut lo, mut hi) = ([EDGE; 3], [0; 3]);
            for &(pos, before) in &applied {
                let l = [pos.x - origin.x, pos.y - origin.y, pos.z - origin.z].map(|v| v as usize);
                for a in 0..3 {
                    lo[a] = lo[a].min(l[a].saturating_sub(15));
                    hi[a] = hi[a].max((l[a] + 16).min(EDGE));
                }
                let now = map.get_block(pos).map_or(LightProps::AIR, props);
                let at_top = map.height(pos.x, pos.z).is_none_or(|h| pos.y + 1 >= h);
                if at_top && (before.opacity > 0) != (now.opacity > 0) {
                    lo[1] = 0;
                }
            }
            let mut snap = self.scratch.take().map_or_else(
                || snapshot_from_world(map, light, center, props, true).map(Box::new),
                Ok,
            )?;
            for attempt in 0..2 {
                if attempt == 1 {
                    // A sky run moved although the box did not reach the
                    // bottom (should not happen): redo with the whole snapshot.
                    (lo, hi) = ([0; 3], [EDGE; 3]);
                }
                reload_region(&mut snap, map, light, center, props, true, lo, hi)?;
                self.revert_pending_raw(&mut snap);
                for &(pos, before) in &applied {
                    let _ = snap.set_props_raw(pos, before);
                }
                snap.rebuild_sky_runs();
                let _ = snap.take_touched();
                let _ = snap.take_sky_low();
                for &(pos, _) in &applied {
                    let now = map.get_block(pos).map_or(LightProps::AIR, props);
                    let _ = snap.relight_remove(pos, now);
                }
                if lo[1] == 0 || snap.take_sky_low() == EDGE {
                    break;
                }
            }
            let touched = snap.take_touched();
            let sky_low = snap.take_sky_low();
            for dy in 0..3 {
                for dz in 0..3 {
                    for dx in 0..3 {
                        if touched & (1 << ((dy * 3 + dz) * 3 + dx)) == 0 {
                            continue;
                        }
                        let sp = SectionPos {
                            x: center.x + dx as i32 - 1,
                            y: center.y + dy as i32 - 1,
                            z: center.z + dz as i32 - 1,
                        };
                        if bounds.section_index(sp.y).is_some() {
                            let merged = snap.section_light_merged(dx, dy, dz, light.get(sp));
                            light.insert(sp, merged);
                        }
                    }
                }
            }
            // Direct sky runs down a column without limit. The snapshot is
            // exact while every changed sky source lies ≥ 15 cells above its
            // bottom (light spreads at most 14 cells). A run change that ends
            // inside the snapshot (`sky_low` ≥ 1) disturbs only a slab around
            // the snapshot's bottom; a run that reaches the bottom may go on
            // to any depth, so everything below is recomputed.
            if sky_low == 0 {
                self.relight_below(map, light, props, center)?;
            } else if sky_low < MAX_LIGHT as usize {
                self.relight_slab(map, light, props, center, &mut snap, lo, hi)?;
            }
            self.scratch = Some(snap);
            done += 1;
        }
        Ok(done)
    }

    /// Recomputes direct sky exactly in the slab ±15 cells around the bottom
    /// `B` of the snapshot centered at `center` (horizontally the box
    /// `lo..hi` of the pass). Cells from `B + 14` up are already exact after
    /// the pass, so the slab's frozen top face (`B + 15`) is right; a run
    /// change ending at or above `B + 1` reaches at most `B − 14`, so the
    /// bottom face (`B − 16`) and the side faces (≥ 15 cells from the edited
    /// columns) are unchanged. A sky-only recompute from the slab's sources
    /// and frozen faces is then exact inside it; block light is unchanged
    /// below the pass's section.
    #[allow(clippy::too_many_arguments)]
    fn relight_slab(
        &self,
        map: &WorldMap,
        light: &mut WorldLight,
        props: &(impl Fn(BlockStateId) -> LightProps + Sync),
        center: SectionPos,
        snap: &mut LightNeighborhood,
        lo: [usize; 3],
        hi: [usize; 3],
    ) -> Result<(), WorldLightError> {
        let below = SectionPos {
            y: center.y - 1,
            ..center
        };
        if map.bounds().section_index(below.y - 1).is_none() {
            // The slab would reach below the world: fall back to the full pass.
            return self.relight_below(map, light, props, center);
        }
        // In the snapshot one section lower, B is local y 16.
        let (slo, shi) = ([lo[0], 1, lo[2]], [hi[0], 31, hi[2]]);
        reload_region(snap, map, light, below, props, true, slo, shi)?;
        self.revert_pending(snap);
        snap.full_relight_sky();
        let _ = snap.take_touched();
        let _ = snap.take_sky_low();
        for dy in 0..2 {
            for dz in 0..3 {
                for dx in 0..3 {
                    if snap.clip_volume([dx, dy, dz]) == 0 {
                        continue;
                    }
                    let sp = SectionPos {
                        x: below.x + dx as i32 - 1,
                        y: below.y + dy as i32 - 1,
                        z: below.z + dz as i32 - 1,
                    };
                    let merged = snap.section_light_merged(dx, dy, dz, light.get(sp));
                    light.insert(sp, merged);
                }
            }
        }
        Ok(())
    }

    fn revert_pending(&self, snap: &mut LightNeighborhood) {
        self.revert_pending_raw(snap);
        snap.rebuild_sky_runs();
    }

    /// Reverts pending edits' properties without rebuilding sky runs.
    fn revert_pending_raw(&self, snap: &mut LightNeighborhood) {
        // Newest first, so the oldest "before" of each cell wins.
        for &(pos, before) in self.pending.iter().rev() {
            let _ = snap.set_props_raw(pos, before);
        }
    }

    fn relight_below(
        &self,
        map: &WorldMap,
        light: &mut WorldLight,
        props: &(impl Fn(BlockStateId) -> LightProps + Sync),
        center: SectionPos,
    ) -> Result<(), WorldLightError> {
        let bounds = map.bounds();
        let mut sy = center.y - 1;
        while sy >= bounds.min_section_y {
            // The 9 sections of a level are independent (each is the exact
            // center of its own snapshot), so they are computed in parallel.
            let level: Vec<SectionPos> = (center.z - 1..=center.z + 1)
                .flat_map(|z| {
                    (center.x - 1..=center.x + 1).map(move |x| SectionPos { x, y: sy, z })
                })
                .collect();
            let shared: &WorldLight = light;
            let fresh: Vec<Result<SectionLight, WorldLightError>> = level
                .par_iter()
                .map(|&sp| {
                    SCRATCH.with(|cell| {
                        let mut slot = cell.borrow_mut();
                        let snap = match slot.as_mut() {
                            Some(s) => {
                                reload_from_world(s, map, shared, sp, props, false)?;
                                s
                            }
                            None => slot.insert(Box::new(snapshot_from_world(
                                map, shared, sp, props, false,
                            )?)),
                        };
                        self.revert_pending(snap);
                        // Edits below the center pass change only direct sky
                        // here; block light stays as stored.
                        Ok(match shared.get(sp) {
                            Some(stored) => {
                                snap.full_relight_sky();
                                SectionLight {
                                    block: stored.block.clone(),
                                    sky: snap.section_sky(1, 1, 1),
                                }
                            }
                            None => {
                                snap.full_relight();
                                snap.section_light(1, 1, 1)
                            }
                        })
                    })
                })
                .collect();
            let mut changed = false;
            for (sp, f) in level.into_iter().zip(fresh) {
                let f = f?;
                if light.get(sp) != Some(&f) {
                    light.insert(sp, f);
                    changed = true;
                }
            }
            if !changed && sy < center.y - 1 {
                break;
            }
            sy -= 1;
        }
        Ok(())
    }
}
