//! T1-09: world-level lighting equals a fresh initial lighting of the final
//! world, through initial shortcuts, queued edits and sky changes.

use std::path::PathBuf;
use std::sync::Arc;

use light::{
    LightUpdates, WorldLight, WorldLightError, light_column, props_from_registry,
    snapshot_from_world,
};
use world::coords::{BlockPos, SectionPos};
use world::registry::blocks_from_dir;
use world::{BlockStateId, ColumnPos, Registry, RegistryBuilder, WorldBounds, WorldMap};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

const BOUNDS: WorldBounds = WorldBounds {
    min_section_y: 0,
    section_count: 8,
};

fn data() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data")
}

fn registry() -> Arc<Registry> {
    let mut b = RegistryBuilder::new();
    b.add_mod(
        "core",
        blocks_from_dir(&data().join("blocks")).expect("blocks"),
    );
    Arc::new(b.build().expect("registry"))
}

/// Generated columns in `-r..=r` around the origin.
fn world(reg: &Arc<Registry>, r: i32, seed: u64) -> WorldMap {
    let src = std::fs::read_to_string(data().join("worldgen/overworld.toml")).expect("config");
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
