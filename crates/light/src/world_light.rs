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

use std::collections::HashMap;
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
        self.sections.insert(pos, light);
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

/// Last section looked up while freezing the border: position, implicit light
/// (outside the world) and stored light.
type CachedSection<'a> = (SectionPos, Option<(u8, u8)>, Option<&'a SectionLight>);

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
    let origin = BlockPos::new(
        (center.x - 1) * SECTION_SIZE,
        (center.y - 1) * SECTION_SIZE,
        (center.z - 1) * SECTION_SIZE,
    );
    snap.begin_load(origin);
    let mut ids = [BlockStateId::AIR; SECTION_VOLUME];
    for dy in 0..3 {
        for dz in 0..3 {
            for dx in 0..3 {
                let sp = SectionPos {
                    x: center.x + dx as i32 - 1,
                    y: center.y + dy as i32 - 1,
                    z: center.z + dz as i32 - 1,
                };
                match section_at(map, sp)? {
                    SectionSlot::Loaded(section) if section.palette_len() == 1 => {
                        snap.load_uniform(dx, dy, dz, props(section.get(0)));
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
    let top = origin.y + EDGE as i32;
    for z in 0..EDGE {
        for x in 0..EDGE {
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
    // Freeze the surrounding layer (only the six faces) at the world's light.
    let e = EDGE as i32;
    let mut cache: Option<CachedSection<'_>> = None;
    for y in -1..=e {
        for z in -1..=e {
            let full_row = !(0..e).contains(&y) || !(0..e).contains(&z);
            let xs: &[i32] = if full_row { &[] } else { &[-1, e] };
            let mut visit = |x: i32| -> Result<(), WorldLightError> {
                let wp = BlockPos::new(origin.x + x, origin.y + y, origin.z + z);
                let sp = wp.section();
                let (implicit, stored) = match cache {
                    Some((p, i, st)) if p == sp => (i, st),
                    _ => {
                        let entry = (implicit_light(map, sp), light.get(sp));
                        cache = Some((sp, entry.0, entry.1));
                        entry
                    }
                };
                let (b, sky) = match (stored, implicit) {
                    (Some(l), _) => {
                        let i = wp.index_in_section();
                        (l.block.get(i), l.sky.get(i))
                    }
                    (None, Some(v)) => v,
                    (None, None) => return Err(WorldLightError::NotLit(sp)),
                };
                snap.freeze_border(x, y, z, b, sky);
                Ok(())
            };
            if full_row {
                for x in -1..=e {
                    visit(x)?;
                }
            } else {
                for &x in xs {
                    visit(x)?;
                }
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
        props: &impl Fn(BlockStateId) -> LightProps,
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
            let mut snap = match self.scratch.take() {
                Some(mut s) => {
                    reload_from_world(&mut s, map, light, center, props, true)?;
                    s
                }
                None => Box::new(snapshot_from_world(map, light, center, props, true)?),
            };
            self.revert_pending(&mut snap);
            let _ = snap.take_touched();
            let mut rest = Vec::with_capacity(self.pending.len());
            let mut applied: Vec<BlockPos> = Vec::new();
            let mut sky_changed = false;
            for (pos, before) in std::mem::take(&mut self.pending) {
                if pos.section() != center {
                    rest.push((pos, before));
                    continue;
                }
                if applied.contains(&pos) {
                    continue;
                }
                applied.push(pos);
                let now = map.get_block(pos).map_or(LightProps::AIR, props);
                // Records are in edit order, so this is the cell's oldest state.
                sky_changed |= (before.opacity > 0) != (now.opacity > 0);
                let _ = snap.relight_remove(pos, now);
            }
            self.pending = rest;
            let touched = snap.take_touched();
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
                            light.insert(sp, snap.section_light(dx, dy, dz));
                        }
                    }
                }
            }
            if sky_changed {
                self.relight_below(map, light, props, center, &mut snap)?;
            }
            self.scratch = Some(snap);
            done += 1;
        }
        Ok(done)
    }

    fn revert_pending(&self, snap: &mut LightNeighborhood) {
        // Newest first, so the oldest "before" of each cell wins.
        for &(pos, before) in self.pending.iter().rev() {
            let _ = snap.set_props_raw(pos, before);
        }
        snap.rebuild_sky_runs();
    }

    fn relight_below(
        &self,
        map: &WorldMap,
        light: &mut WorldLight,
        props: &impl Fn(BlockStateId) -> LightProps,
        center: SectionPos,
        snap: &mut LightNeighborhood,
    ) -> Result<(), WorldLightError> {
        let bounds = map.bounds();
        let mut sy = center.y - 1;
        while sy >= bounds.min_section_y {
            let mut changed = false;
            for cz in center.z - 1..=center.z + 1 {
                for cx in center.x - 1..=center.x + 1 {
                    let sp = SectionPos {
                        x: cx,
                        y: sy,
                        z: cz,
                    };
                    reload_from_world(snap, map, light, sp, props, false)?;
                    self.revert_pending(snap);
                    snap.full_relight();
                    let fresh = snap.section_light(1, 1, 1);
                    if light.get(sp) != Some(&fresh) {
                        light.insert(sp, fresh);
                        changed = true;
                    }
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
