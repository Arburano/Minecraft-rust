//! T1-03: the overworld pipeline is deterministic, consistent across column
//! borders and configured only by mod data.

use std::sync::Arc;

use engine_core::jobs::JobPool;
use world::{BlockStateId, ColumnPos, Registry, WorldBounds};
use worldgen::{ConfigError, GenContext, Pipeline, ProtoColumn, Stage, WorldgenConfig};

fn build() -> world::ModSet {
    world::modset::source_build(world::modset::DEFAULT_BUILD).expect("build")
}

fn registry() -> Registry {
    let mods = build();
    let b = mods.registry_builder().expect("blocks");
    b.build().expect("registry")
}

fn context(reg: &Registry, seed: u64) -> GenContext {
    let src = build()
        .read("worldgen/overworld.toml")
        .expect("config file");
    let config = WorldgenConfig::from_toml(&src, reg).expect("valid config");
    GenContext::new(seed, config, WorldBounds::default())
}

fn area(n: i32) -> Vec<ColumnPos> {
    (-n..n)
        .flat_map(|x| (-n..n).map(move |z| ColumnPos { x, z }))
        .collect()
}

#[test]
fn order_and_thread_count_do_not_change_columns() {
    let reg = registry();
    let ctx = Arc::new(context(&reg, 42));
    let pipeline = Arc::new(Pipeline::overworld());
    let positions = area(2);
    let serial: Vec<u64> = positions
        .iter()
        .map(|&p| pipeline.generate(&ctx, p).hash())
        .collect();
    let reversed: Vec<u64> = positions
        .iter()
        .rev()
        .map(|&p| pipeline.generate(&ctx, p).hash())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    assert_eq!(serial, reversed);
    let mut sorted = positions.clone();
    sorted.sort_by_key(|p| (p.x, p.z));
    let expected: Vec<u64> = sorted
        .iter()
        .map(|&p| pipeline.generate(&ctx, p).hash())
        .collect();
    for threads in [1, 2, 4] {
        let pool = JobPool::new(threads).expect("pool");
        let par: Vec<u64> = pipeline
            .generate_parallel(&ctx, &pool, &positions)
            .iter()
            .map(ProtoColumn::hash)
            .collect();
        assert_eq!(par, expected, "{threads} threads");
    }
    let other = context(&reg, 43);
    assert_ne!(
        pipeline.generate(&other, positions[0]).hash(),
        serial[0],
        "seed matters"
    );
}

/// Every crown cell that was air before trees is leaves or log afterwards,
/// including cells in neighboring columns: trees are whole across borders.
#[test]
fn trees_are_complete_across_column_borders() {
    let reg = registry();
    let ctx = context(&reg, 7);
    let full = Pipeline::overworld();
    let mut no_trees = Pipeline::overworld();
    struct Nothing;
    impl Stage for Nothing {
        fn id(&self) -> &str {
            "core:trees"
        }
        fn run(&self, _: &GenContext, _: &mut ProtoColumn) {}
    }
    assert!(no_trees.replace("core:trees", Box::new(Nothing)));
    let positions = area(3);
    let get = |cols: &[ProtoColumn], wx: i32, y: i32, wz: i32| {
        let pos = ColumnPos {
            x: wx.div_euclid(16),
            z: wz.div_euclid(16),
        };
        cols.iter()
            .find(|c| c.pos == pos)
            .map(|c| c.get(wx.rem_euclid(16) as usize, y, wz.rem_euclid(16) as usize))
    };
    let after: Vec<ProtoColumn> = positions.iter().map(|&p| full.generate(&ctx, p)).collect();
    let before: Vec<ProtoColumn> = positions
        .iter()
        .map(|&p| no_trees.generate(&ctx, p))
        .collect();
    let (log, leaves) = (ctx.config.trees.log, ctx.config.trees.leaves);
    let mut trees = 0;
    let mut border_trees = 0;
    for c in &after {
        for z in 0..16 {
            for x in 0..16 {
                for y in -64..320 {
                    if c.get(x, y, z) != log || c.get(x, y + 1, z) == log {
                        continue;
                    }
                    // Trunk top at y; the crown spans top-1..top+2 with radius 2.
                    let (wx, wz) = (c.pos.x * 16 + x as i32, c.pos.z * 16 + z as i32);
                    let top = y + 1;
                    let mut crosses = false;
                    for dy in -2i32..=1 {
                        let r: i32 = if dy >= 0 { 1 } else { 2 };
                        for dz in -r..=r {
                            for dx in -r..=r {
                                if dx.abs() == r && dz.abs() == r && dy != -1 {
                                    continue;
                                }
                                let (cx, cy, cz) = (wx + dx, top + dy, wz + dz);
                                let (Some(b), Some(a)) =
                                    (get(&before, cx, cy, cz), get(&after, cx, cy, cz))
                                else {
                                    continue; // outside the generated area
                                };
                                if cx.div_euclid(16) != c.pos.x || cz.div_euclid(16) != c.pos.z {
                                    crosses = true;
                                }
                                if b == BlockStateId::AIR {
                                    assert!(
                                        a == leaves || a == log,
                                        "crown hole at {cx},{cy},{cz}"
                                    );
                                }
                            }
                        }
                    }
                    trees += 1;
                    border_trees += usize::from(crosses);
                }
            }
        }
    }
    assert!(trees > 20, "only {trees} trees");
    assert!(
        border_trees > 0,
        "no tree crosses a border; the test proves nothing"
    );
}

#[test]
fn stages_can_be_replaced_and_inserted() {
    let mut p = Pipeline::overworld();
    assert_eq!(
        p.ids(),
        vec![
            "core:surface",
            "core:terrain",
            "core:caves",
            "core:ores",
            "core:trees"
        ]
    );
    struct Marker;
    impl Stage for Marker {
        fn id(&self) -> &str {
            "test:marker"
        }
        fn run(&self, _: &GenContext, c: &mut ProtoColumn) {
            c.set(0, 300, 0, BlockStateId(2));
        }
    }
    p.insert_after("core:caves", Box::new(Marker));
    assert_eq!(p.ids()[3], "test:marker");
    assert!(!p.replace("nope", Box::new(Marker)));
    let reg = registry();
    let col = p.generate(&context(&reg, 1), ColumnPos::default());
    assert_eq!(col.get(0, 300, 0), BlockStateId(2));
}

#[test]
fn config_names_blocks_through_the_registry() {
    let reg = registry();
    let src = build().read("worldgen/overworld.toml").expect("config");
    let broken = src.replace("core:coal_ore", "core:unobtainium");
    assert_eq!(
        WorldgenConfig::from_toml(&broken, &reg).err(),
        Some(ConfigError::UnknownBlock("core:unobtainium".into()))
    );
}

#[test]
fn generated_world_is_plausible() {
    let reg = registry();
    let ctx = context(&reg, 1234);
    let p = Pipeline::overworld();
    let id = |n: &str| reg.state_id(n).expect(n);
    let (stone, water, air) = (id("core:stone"), id("core:water"), BlockStateId::AIR);
    let mut counts = std::collections::HashMap::new();
    let mut cave_air = 0usize;
    for pos in area(3) {
        let c = p.generate(&ctx, pos);
        for z in 0..16 {
            for x in 0..16 {
                let h = c.surface[z * 16 + x].height;
                assert!((-63..304).contains(&h));
                for y in -64..320 {
                    let b = c.get(x, y, z);
                    *counts.entry(b).or_insert(0usize) += 1;
                    if b == air && y < h - 6 {
                        cave_air += 1;
                    }
                }
            }
        }
        let column = c.into_column(&reg).expect("bounds match");
        assert!(column.height(0, 0) > -64);
    }
    for b in [
        stone,
        water,
        air,
        id("core:oak_leaves"),
        id("core:coal_ore"),
        id("core:iron_ore"),
        id("core:grass_block"),
        id("core:sand"),
    ] {
        assert!(
            counts.get(&b).copied().unwrap_or(0) > 0,
            "no {:?}",
            reg.name(b)
        );
    }
    assert!(
        cave_air > 1000,
        "caves: only {cave_air} air blocks underground"
    );
}
