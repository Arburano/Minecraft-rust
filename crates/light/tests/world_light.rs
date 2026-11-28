//! T1-09: world-level lighting equals a fresh initial lighting of the final
//! world, through initial shortcuts, queued edits and sky changes.

use std::sync::Arc;

use light::{
    LightUpdates, WorldLight, WorldLightError, light_column, props_from_registry,
    snapshot_from_world,
};
use world::coords::{BlockPos, SectionPos};
use world::{BlockStateId, ColumnPos, Registry, WorldBounds, WorldMap};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

const BOUNDS: WorldBounds = WorldBounds {
    min_section_y: 0,
    section_count: 8,
};

fn data() -> world::ModSet {
    world::modset::source_build(world::modset::DEFAULT_BUILD).expect("build")
}

fn registry() -> Arc<Registry> {
    let mods = data();
    let b = mods.registry_builder().expect("blocks");
    Arc::new(b.build().expect("registry"))
}

/// Generated columns in `-r..=r` around the origin.
fn world(reg: &Arc<Registry>, r: i32, seed: u64) -> WorldMap {
    let src = data().read("worldgen/overworld.toml").expect("config");
    let ctx = GenContext::new(
        seed,
        WorldgenConfig::from_toml(&src, reg).expect("config"),
        BOUNDS,
    );
    let p = Pipeline::overworld();
    let mut map = WorldMap::new(Arc::clone(reg), BOUNDS);
    for x in -r..=r {
        for z in -r..=r {
            let col = p
                .generate(&ctx, ColumnPos { x, z })
                .into_column(reg)
                .expect("bounds");
            map.insert_column(col).expect("bounds");
        }
    }
    map
}

fn light_area(map: &WorldMap, r: i32, reg: &Registry) -> WorldLight {
    let props = props_from_registry(reg);
    let mut light = WorldLight::new();
    for x in -r..=r {
        for z in -r..=r {
            light_column(map, &mut light, ColumnPos { x, z }, &props).expect("neighbors loaded");
        }
    }
    light
}

#[test]
fn shortcuts_match_full_relight() {
    let reg = registry();
    let map = world(&reg, 3, 5);
    let props = props_from_registry(&reg);
    let mut light = WorldLight::new();
    let mut totals = (0, 0, 0);
    for x in -2..=2 {
        for z in -2..=2 {
            let s = light_column(&map, &mut light, ColumnPos { x, z }, &props).expect("lit");
            totals = (
                totals.0 + s.opaque,
                totals.1 + s.open_sky,
                totals.2 + s.computed,
            );
            for sy in 0..8 {
                let sp = SectionPos { x, y: sy, z };
                let mut snap =
                    snapshot_from_world(&map, &light, sp, &props, false).expect("snapshot");
                snap.full_relight();
                assert_eq!(light.get(sp), Some(&snap.section_light(1, 1, 1)), "{sp:?}");
            }
        }
    }
    assert!(
        totals.1 > 0 && totals.2 > 0,
        "both shortcut and BFS paths used: {totals:?}"
    );
}

struct Rng(u64);
impl Rng {
    fn below(&mut self, n: u64) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0 % n
    }
}

#[test]
fn queued_edits_equal_fresh_lighting() {
    let reg = registry();
    let props = props_from_registry(&reg);
    for seed in [1u64, 2, 3] {
        let mut map = world(&reg, 4, seed);
        let mut light = light_area(&map, 3, &reg);
        let id = |n: &str| reg.state_id(n).expect(n);
        let kinds = [
            BlockStateId::AIR,
            id("core:stone"),
            id("core:torch"),
            id("core:glowstone"),
            id("core:glass"),
            id("core:water"),
        ];
        let mut rng = Rng(seed * 0x9e37_79b9 + 1);
        let mut updates = LightUpdates::new();
        let mut processed = 0;
        for step in 0..60 {
            // Edits in the inner 3x3 columns, biased to the surface and caves.
            let (x, z) = (rng.below(48) as i32 - 16, rng.below(48) as i32 - 16);
            let h = map.height(x, z).expect("loaded");
            let y = (h - 8 + rng.below(16) as i32).clamp(0, 127);
            let pos = BlockPos::new(x, y, z);
            let new = kinds[rng.below(kinds.len() as u64) as usize];
            let old = map.get_block(pos).expect("loaded");
            map.set_block(pos, new).expect("valid");
            updates.record(pos, props(old));
            if step % 3 == 0 {
                processed += updates
                    .process(&map, &mut light, &props, 1)
                    .expect("lit around");
            }
        }
        while !updates.is_empty() {
            processed += updates
                .process(&map, &mut light, &props, 1)
                .expect("lit around");
        }
        assert!(processed > 10);
        let fresh = light_area(&map, 2, &reg);
        for x in -2..=2 {
            for z in -2..=2 {
                for sy in 0..8 {
                    let sp = SectionPos { x, y: sy, z };
                    assert_eq!(light.get(sp), fresh.get(sp), "seed {seed}, section {sp:?}");
                }
            }
        }
    }
}

/// Sky-changing edits at every depth relative to section borders: shafts of
/// random depth are dug from the surface, roofed and opened at random heights
/// (often a section's top or bottom rows), in random batch sizes. T1-16 skips
/// the downward recompute when a change stays inside the snapshot, so this
/// checks the boundary of that rule.
#[test]
fn sky_edits_at_section_borders_equal_fresh_lighting() {
    let reg = registry();
    let props = props_from_registry(&reg);
    let stone = reg.state_id("core:stone").expect("stone");
    let glass = reg.state_id("core:glass").expect("glass");
    let water = reg.state_id("core:water").expect("water");
    for seed in [4u64, 5, 6, 7, 8] {
        let mut map = world(&reg, 4, seed);
        let mut light = light_area(&map, 3, &reg);
        let mut rng = Rng(seed * 0x51_7cc1 + 7);
        let mut updates = LightUpdates::new();
        let edit = |map: &mut WorldMap, updates: &mut LightUpdates, pos: BlockPos, id| {
            if !(0..128).contains(&pos.y) {
                return;
            }
            let old = map.get_block(pos).expect("loaded");
            map.set_block(pos, id).expect("valid");
            updates.record(pos, props(old));
        };
        for _ in 0..12 {
            let (x, z) = (rng.below(24) as i32 - 12, rng.below(40) as i32 - 12);
            let top = map.height(x, z).expect("loaded");
            let depth = 1 + rng.below(48) as i32;
            let bottom = (top - depth).max(4);
            for y in bottom..top {
                edit(
                    &mut map,
                    &mut updates,
                    BlockPos::new(x, y, z),
                    BlockStateId::AIR,
                );
            }
            // A room under the shaft, so changed sky light has somewhere to
            // spread below the snapshot.
            let (rw, rh) = (1 + rng.below(4) as i32, 1 + rng.below(5) as i32);
            for y in (bottom - rh).max(1)..bottom {
                for dz in -rw..=rw {
                    for dx in -rw..=rw {
                        edit(
                            &mut map,
                            &mut updates,
                            BlockPos::new(x + dx, y, z + dz),
                            BlockStateId::AIR,
                        );
                    }
                }
            }
            // A roof at a section border row (or anywhere), then maybe open it.
            let border = [0, 1, 14, 15][rng.below(4) as usize];
            // A tunnel stepping down from the room's corner: a path for light
            // below the room's floor, away from the sky column.
            let floor = (bottom - rh).max(1);
            for k in 0..16 {
                let p = BlockPos::new(x + rw + 1 + k, floor - k, z);
                if p.y >= 1 {
                    edit(&mut map, &mut updates, p, BlockStateId::AIR);
                    edit(
                        &mut map,
                        &mut updates,
                        BlockPos::new(p.x, p.y + 1, p.z),
                        BlockStateId::AIR,
                    );
                }
            }
            let section = bottom.div_euclid(16) + rng.below(3) as i32;
            let roof_y = if rng.below(3) == 0 {
                top + rng.below(6) as i32
            } else {
                section * 16 + border
            };
            let roof = [stone, glass, water][rng.below(3) as usize];
            edit(&mut map, &mut updates, BlockPos::new(x, roof_y, z), roof);
            if rng.below(2) == 0 {
                edit(
                    &mut map,
                    &mut updates,
                    BlockPos::new(x, roof_y, z),
                    BlockStateId::AIR,
                );
            }
            if rng.below(2) == 0 {
                let batch = 1 + rng.below(3) as usize;
                updates
                    .process(&map, &mut light, &props, batch)
                    .expect("lit");
            }
        }
        while !updates.is_empty() {
            updates.process(&map, &mut light, &props, 1).expect("lit");
        }
        let fresh = light_area(&map, 2, &reg);
        for x in -2..=2 {
            for z in -2..=2 {
                for sy in 0..8 {
                    let sp = SectionPos { x, y: sy, z };
                    assert_eq!(light.get(sp), fresh.get(sp), "seed {seed}, section {sp:?}");
                }
            }
        }
    }
}

/// The exact cases the T1-16 shortcut must not skip: a roof placed on (or
/// removed from) the bottom row of section 4 (y 64) changes a shaft's sky run
/// that ends 1, 5 or 12 cells above the snapshot's bottom (snapshot y
/// 48..95), and a hole next to the shaft carries that light below the
/// snapshot (down to y 30). The light under y 48 must change too. At 12
/// cells this is the deepest run whose change still reaches the border
/// (15 − 12 − 1 down − 1 sideways = 1); the rule recomputes below 15.
#[test]
fn roof_over_a_shallow_run_darkens_below_the_snapshot() {
    let reg = registry();
    let props = props_from_registry(&reg);
    let stone = reg.state_id("core:stone").expect("stone");
    for (floor, roof_open_first) in [
        (49, false),
        (53, false),
        (53, true),
        (60, false),
        (60, true),
    ] {
        let mut map = world(&reg, 3, 12);
        let (x, z) = (8, 8);
        let air = BlockStateId::AIR;
        for y in floor..128 {
            map.set_block(BlockPos::new(x, y, z), air).expect("valid");
        }
        map.set_block(BlockPos::new(x, floor - 1, z), stone)
            .expect("valid");
        for y in floor..floor + 2 {
            for dx in -2..=2 {
                for dz in -2..=2 {
                    if (dx, dz) != (0, 0) {
                        map.set_block(BlockPos::new(x + dx, y, z + dz), air)
                            .expect("valid");
                    }
                }
            }
        }
        for y in 30..floor {
            map.set_block(BlockPos::new(x + 1, y, z), air)
                .expect("valid");
        }
        let roof = BlockPos::new(x, 64, z);
        if roof_open_first {
            map.set_block(roof, stone).expect("valid");
        }
        let mut light = light_area(&map, 2, &reg);
        let mut updates = LightUpdates::new();
        let (old, new) = if roof_open_first {
            (stone, air)
        } else {
            (air, stone)
        };
        map.set_block(roof, new).expect("valid");
        updates.record(roof, props(old));
        while !updates.is_empty() {
            updates.process(&map, &mut light, &props, 1).expect("lit");
        }
        let fresh = light_area(&map, 1, &reg);
        for sy in 0..8 {
            let sp = SectionPos { x: 0, y: sy, z: 0 };
            assert_eq!(
                light.get(sp),
                fresh.get(sp),
                "floor {floor}, roof open first {roof_open_first}: {sp:?}"
            );
        }
    }
}

#[test]
fn breaking_a_cave_roof_relights_far_below() {
    let reg = registry();
    let props = props_from_registry(&reg);
    let mut map = world(&reg, 3, 9);
    // Carve a shaft from y 100 down to y 8 at (5, 5), sealed at the top.
    let stone = reg.state_id("core:stone").expect("stone");
    for y in 8..120 {
        map.set_block(BlockPos::new(5, y, 5), BlockStateId::AIR)
            .expect("valid");
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            map.set_block(BlockPos::new(5 + dx, y, 5 + dz), stone)
                .expect("valid");
        }
    }
    map.set_block(BlockPos::new(5, 120, 5), stone)
        .expect("valid");
    let mut light = light_area(&map, 2, &reg);
    assert_eq!(
        light.at(BlockPos::new(5, 10, 5)).map(|l| l.1),
        Some(0),
        "sealed shaft is dark"
    );
    let mut updates = LightUpdates::new();
    map.set_block(BlockPos::new(5, 120, 5), BlockStateId::AIR)
        .expect("valid");
    updates.record(BlockPos::new(5, 120, 5), props(stone));
    while !updates.is_empty() {
        updates.process(&map, &mut light, &props, 1).expect("lit");
    }
    assert_eq!(
        light.at(BlockPos::new(5, 10, 5)).map(|l| l.1),
        Some(15),
        "sky reaches 110 blocks down"
    );
    let fresh = light_area(&map, 1, &reg);
    for sy in 0..8 {
        let sp = SectionPos { x: 0, y: sy, z: 0 };
        assert_eq!(light.get(sp), fresh.get(sp), "{sp:?}");
    }
}

#[test]
fn storage_blob_round_trips_and_unlit_neighbors_are_errors() {
    let reg = registry();
    let props = props_from_registry(&reg);
    let map = world(&reg, 2, 4);
    let light = light_area(&map, 1, &reg);
    let blob = light.encode_column(ColumnPos::default(), BOUNDS);
    let mut back = WorldLight::new();
    back.decode_column(ColumnPos::default(), BOUNDS, &blob)
        .expect("decodes");
    for sy in 0..8 {
        let sp = SectionPos { x: 0, y: sy, z: 0 };
        assert_eq!(back.get(sp), light.get(sp));
    }
    assert!(
        back.decode_column(ColumnPos::default(), BOUNDS, &blob[..blob.len() - 1])
            .is_err()
    );
    // The border of a snapshot centered at column 1 needs column 2's light (unlit).
    let err = snapshot_from_world(&map, &light, SectionPos { x: 1, y: 3, z: 0 }, &props, true);
    assert!(matches!(err, Err(WorldLightError::NotLit(_))));
}
