//! WorldMap against a naive `HashMap<BlockPos, BlockStateId>` reference (T1-01).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use proptest::prelude::*;
use world::coords::{BlockPos, SectionPos};
use world::registry::blocks_from_dir;
use world::{
    BlockDef, BlockProps, BlockStateId, Column, ColumnPos, DirtyKind, Registry, RegistryBuilder,
    WorldBounds, WorldError, WorldMap,
};

fn registry() -> Arc<Registry> {
    let glass = BlockProps {
        opaque: false,
        transparent: true,
        light_emission: 0,
        light_opacity: 0,
        collision: world::Collision::Full,
        mesher: world::MesherKind::Cubic,
    };
    let water = BlockProps {
        light_opacity: 1,
        ..glass
    };
    let mut b = RegistryBuilder::new();
    b.add_mod(
        "t",
        vec![
            BlockDef::simple("t:stone", BlockProps::MISSING),
            BlockDef::simple("t:glass", glass),
            BlockDef::simple("t:water", water),
        ],
    );
    Arc::new(b.build().expect("valid registry"))
}

const BOUNDS: WorldBounds = WorldBounds {
    min_section_y: -1,
    section_count: 3,
};

/// Reference heightmap: topmost block with light opacity > 0, plus one.
fn reference_height(
    reg: &Registry,
    blocks: &HashMap<BlockPos, BlockStateId>,
    x: i32,
    z: i32,
) -> i32 {
    (BOUNDS.min_y()..BOUNDS.max_y())
        .rev()
        .find(|&y| {
            blocks
                .get(&BlockPos::new(x, y, z))
                .is_some_and(|&b| reg.props(b).light_opacity > 0)
        })
        .map_or(BOUNDS.min_y(), |y| y + 1)
}

#[derive(Clone, Debug)]
enum Op {
    Set(i32, i32, i32, u16),
    Load(i32, i32),
    Unload(i32, i32),
}

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (-20i32..20, -20i32..36, -20i32..20, 0u16..6).prop_map(|(x, y, z, b)| Op::Set(x, y, z, b)),
        // A tight cluster so cells are overwritten and removed often (heightmap drops).
        6 => (-2i32..2, -3i32..6, -2i32..2, 0u16..6).prop_map(|(x, y, z, b)| Op::Set(x, y, z, b)),
        1 => (-2i32..2, -2i32..2).prop_map(|(x, z)| Op::Load(x, z)),
        1 => (-2i32..2, -2i32..2).prop_map(|(x, z)| Op::Unload(x, z)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn map_matches_naive_reference(ops in prop::collection::vec(op(), 1..300)) {
        let reg = registry();
        let mut map = WorldMap::new(Arc::clone(&reg), BOUNDS);
        let mut loaded: HashSet<ColumnPos> = HashSet::new();
        let mut blocks: HashMap<BlockPos, BlockStateId> = HashMap::new();
        let mut dirty_ref: HashSet<SectionPos> = HashSet::new();
        for op in ops {
            match op {
                Op::Set(x, y, z, b) => {
                    let pos = BlockPos::new(x, y, z);
                    let id = BlockStateId(b);
                    let result = map.set_block(pos, id);
                    let col = ColumnPos::of_block(pos);
                    if !reg.contains(id) {
                        prop_assert_eq!(result, Err(WorldError::UnknownState(id)));
                    } else if !loaded.contains(&col) {
                        prop_assert_eq!(result, Err(WorldError::NotLoaded(col)));
                    } else if !(BOUNDS.min_y()..BOUNDS.max_y()).contains(&y) {
                        prop_assert_eq!(result, Err(WorldError::OutOfHeight(pos)));
                    } else {
                        let old = blocks.insert(pos, id).unwrap_or(BlockStateId::AIR);
                        prop_assert_eq!(result, Ok(old));
                        if old != id {
                            dirty_ref.insert(pos.section());
                        }
                    }
                }
                Op::Load(x, z) => {
                    let pos = ColumnPos { x, z };
                    map.insert_column(Column::new(pos, BOUNDS)).expect("same bounds");
                    loaded.insert(pos);
                    blocks.retain(|p, _| ColumnPos::of_block(*p) != pos);
                    dirty_ref.retain(|s| ColumnPos::of_section(*s) != pos);
                }
                Op::Unload(x, z) => {
                    let pos = ColumnPos { x, z };
                    prop_assert_eq!(map.remove_column(pos).is_some(), loaded.remove(&pos));
                    blocks.retain(|p, _| ColumnPos::of_block(*p) != pos);
                    dirty_ref.retain(|s| ColumnPos::of_section(*s) != pos);
                }
            }
        }
        for x in -20..20 {
            for z in -20..20 {
                let col = ColumnPos::of_block(BlockPos::new(x, 0, z));
                if loaded.contains(&col) {
                    prop_assert_eq!(map.height(x, z), Some(reference_height(&reg, &blocks, x, z)));
                } else {
                    prop_assert_eq!(map.height(x, z), None);
                }
                for y in -20..36 {
                    let pos = BlockPos::new(x, y, z);
                    let expected = (loaded.contains(&col) && (BOUNDS.min_y()..BOUNDS.max_y()).contains(&y))
                        .then(|| blocks.get(&pos).copied().unwrap_or(BlockStateId::AIR));
                    prop_assert_eq!(map.get_block(pos), expected);
                }
            }
        }
        for kind in [DirtyKind::Mesh, DirtyKind::Light, DirtyKind::Save] {
            let mut out = Vec::new();
            map.take_dirty(kind, &mut out);
            let set: HashSet<SectionPos> = out.iter().copied().collect();
            prop_assert_eq!(set.len(), out.len(), "each section reported once");
            prop_assert_eq!(&set, &dirty_ref);
            map.take_dirty(kind, &mut out);
            prop_assert_eq!(out.len(), set.len(), "drained");
        }
    }
}

#[test]
fn from_sections_heightmap_matches_incremental() {
    let reg = registry();
    let stone = BlockStateId(2);
    let mut incremental = Column::new(ColumnPos { x: 0, z: 0 }, BOUNDS);
    for (x, y, z) in [(0, -16, 0), (0, 5, 0), (3, 30, 7), (15, 0, 15)] {
        incremental.set(x, y, z, stone, &reg);
    }
    let rebuilt = Column::from_sections(
        incremental.pos(),
        BOUNDS,
        incremental.sections().to_vec(),
        &reg,
    )
    .expect("matching section count");
    for z in 0..16 {
        for x in 0..16 {
            assert_eq!(incremental.height(x, z), rebuilt.height(x, z));
        }
    }
    assert_eq!(incremental.height(0, 0), 6);
    assert_eq!(incremental.height(3, 7), 31);
    assert_eq!(incremental.height(1, 1), BOUNDS.min_y());
    assert!(Column::from_sections(incremental.pos(), BOUNDS, Vec::new(), &reg).is_none());
}

#[test]
fn bounds_mismatch_is_rejected() {
    let mut map = WorldMap::new(registry(), BOUNDS);
    let other = Column::new(ColumnPos::default(), WorldBounds::default());
    assert!(matches!(
        map.insert_column(other),
        Err(WorldError::BoundsMismatch(_))
    ));
}

#[test]
fn core_datapack_builds_a_registry() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data/blocks");
    let blocks = blocks_from_dir(&dir).expect("core blocks parse");
    assert!(blocks.len() >= 10);
    let mut b = RegistryBuilder::new();
    b.add_mod("core", blocks);
    let reg = b.build().expect("valid core registry");
    let torch = reg.state_id("core:torch").expect("torch exists");
    assert_eq!(reg.props(torch).light_emission, 14);
    let water = reg.state_id("core:water").expect("water exists");
    assert_eq!(reg.props(water).light_opacity, 1);
    assert!(reg.props(reg.state_id("core:stone").expect("stone")).opaque);
}

#[test]
fn colliding_columns_use_overflow_and_get_promoted() {
    let reg = registry();
    let stone = BlockStateId(2);
    let mut map = WorldMap::new(reg, BOUNDS);
    // 64 columns apart = the same direct slot.
    let (a, b, c) = (
        ColumnPos { x: 0, z: 0 },
        ColumnPos { x: 64, z: 0 },
        ColumnPos { x: -64, z: 64 },
    );
    for p in [a, b, c] {
        assert!(
            map.insert_column(Column::new(p, BOUNDS))
                .expect("bounds")
                .is_none()
        );
    }
    assert_eq!(map.len(), 3);
    let in_col = |p: ColumnPos| BlockPos::new(p.x * 16 + 3, 4, p.z * 16 + 5);
    for (p, y) in [(a, 1), (b, 2), (c, 3)] {
        map.set_block(BlockPos { y, ..in_col(p) }, stone)
            .expect("loaded");
    }
    for (p, y) in [(a, 1), (b, 2), (c, 3)] {
        assert_eq!(map.get_block(BlockPos { y, ..in_col(p) }), Some(stone));
        assert_eq!(
            map.get_block(BlockPos {
                y: y + 1,
                ..in_col(p)
            }),
            Some(BlockStateId::AIR)
        );
    }
    assert!(map.remove_column(a).is_some());
    assert_eq!(map.get_block(in_col(a)), None);
    assert_eq!(
        map.get_block(BlockPos { y: 2, ..in_col(b) }),
        Some(stone),
        "promoted"
    );
    assert_eq!(map.get_block(BlockPos { y: 3, ..in_col(c) }), Some(stone));
    assert!(map.remove_column(c).is_some());
    assert!(map.remove_column(b).is_some());
    assert!(map.remove_column(b).is_none());
    assert!(map.is_empty());
    let mut out = Vec::new();
    map.take_dirty(DirtyKind::Mesh, &mut out);
    assert!(out.is_empty(), "marks of removed columns are dropped");
}
