//! Landscape engines (worldgen.md, "Engines"): the `climate` engine of the
//! `overworld_plus` module is deterministic, varied, continuous and keeps
//! trees exact across column borders; configs are checked.

use std::collections::HashSet;
use std::sync::Arc;

use engine_core::jobs::JobPool;
use world::{BlockStateId, ColumnPos, Registry, WorldBounds};
use worldgen::{
    ConfigError, Engine, GenContext, Pipeline, ProtoColumn, Stage, TreeShape, WorldgenConfig,
};

fn plus() -> world::ModSet {
    world::modset::source_build("classic_plus").expect("build")
}

fn registry() -> Registry {
    plus()
        .registry_builder()
        .expect("blocks")
        .build()
        .expect("registry")
}

fn source() -> String {
    plus().read("worldgen/overworld.toml").expect("config")
}

fn context(reg: &Registry, seed: u64) -> GenContext {
    let config = WorldgenConfig::from_toml(&source(), reg).expect("valid config");
    GenContext::new(seed, config, WorldBounds::default())
}

#[test]
fn builds_pick_their_engine() {
    let reg = registry();
    assert!(matches!(context(&reg, 1).config.engine, Engine::Climate(_)));
    let classic = world::modset::source_build("classic").expect("classic");
    let creg = classic.registry_builder().expect("b").build().expect("r");
    let src = classic.read("worldgen/overworld.toml").expect("config");
    let cfg = WorldgenConfig::from_toml(&src, &creg).expect("classic config");
    assert_eq!(cfg.engine.name(), "classic");
    // The classic build does not carry the advanced module's blocks.
    assert!(creg.state_id("overworld_plus:spruce_log").is_none());
}

#[test]
fn climate_columns_are_deterministic() {
    let reg = registry();
    let ctx = Arc::new(context(&reg, 99));
    let pipeline = Arc::new(Pipeline::overworld());
    let positions: Vec<ColumnPos> = (-2..2)
        .flat_map(|x| (-2..2).map(move |z| ColumnPos { x, z }))
        .collect();
    let serial: Vec<u64> = positions
        .iter()
        .map(|&p| pipeline.generate(&ctx, p).hash())
        .collect();
    let again = Arc::new(context(&reg, 99));
    let pool = JobPool::new(3).expect("pool");
    let parallel: Vec<u64> = pipeline
        .generate_parallel(&again, &pool, &positions)
        .iter()
        .map(ProtoColumn::hash)
        .collect();
    assert_eq!(serial, parallel, "positions are already sorted by (x, z)");
    let other = context(&reg, 100);
    assert_ne!(serial[0], pipeline.generate(&other, positions[0]).hash());
}

#[test]
fn climate_landscape_is_varied_and_continuous() {
    let reg = registry();
    let ctx = context(&reg, 2026);
    let sea = ctx.config.sea_level;
    let (mut ocean, mut mountain, mut biomes) = (0, 0, HashSet::new());
    let mut steepest = 0;
    // 12 km × 12 km, every 48 blocks, plus the next block for the slope.
    for gz in -125..125 {
        for gx in -125..125 {
            let (x, z) = (gx * 48, gz * 48);
            let s = ctx.surface(x, z);
            let next = ctx.surface(x + 1, z);
            steepest = steepest.max((s.height - next.height).abs());
            biomes.insert(s.biome);
            if s.height < sea - 10 {
                ocean += 1;
            }
            if s.height > sea + 60 {
                mountain += 1;
            }
        }
    }
    let n = 250 * 250;
    assert!(ocean * 20 > n, "oceans: {ocean} of {n}");
    assert!(mountain * 100 > n, "mountains: {mountain} of {n}");
    assert!(biomes.len() >= 7, "only {} biomes", biomes.len());
    // Continuous height: no cliffs taller than a few blocks between neighbors.
    assert!(steepest <= 6, "a {steepest}-block step between neighbors");
}

#[test]
fn every_tree_kind_is_complete_across_column_borders() {
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
    // Forested 3×3 groups of columns, three per tree kind.
    let kinds = ctx.config.tree_kinds.len();
    let mut found = vec![0usize; kinds];
    let mut area = Vec::new();
    'search: for gz in -60..60 {
        for gx in -60..60 {
            let (cx, cz) = (gx * 6, gz * 6);
            let s = ctx.surface(cx * 16 + 8, cz * 16 + 8);
            let biome = &ctx.config.biomes[usize::from(s.biome)];
            let grows: Vec<usize> = if biome.trees.is_empty() {
                vec![0]
            } else {
                biome.trees.iter().map(|&k| usize::from(k)).collect()
            };
            if biome.trees_per_column >= 2.0 && grows.iter().any(|&k| found[k] < 3) {
                for k in grows {
                    found[k] += 1;
                }
                for z in cz - 1..=cz + 1 {
                    for x in cx - 1..=cx + 1 {
                        area.push(ColumnPos { x, z });
                    }
                }
            }
            if found.iter().all(|&n| n >= 3) {
                break 'search;
            }
        }
    }
    assert!(found.iter().all(|&n| n >= 3), "forests per kind: {found:?}");
    let after: Vec<ProtoColumn> = area.iter().map(|&p| full.generate(&ctx, p)).collect();
    let before: Vec<ProtoColumn> = area.iter().map(|&p| no_trees.generate(&ctx, p)).collect();
    let at = |cols: &[ProtoColumn], wx: i32, y: i32, wz: i32| {
        let pos = ColumnPos {
            x: wx.div_euclid(16),
            z: wz.div_euclid(16),
        };
        cols.iter()
            .find(|c| c.pos == pos)
            .map(|c| c.get(wx.rem_euclid(16) as usize, y, wz.rem_euclid(16) as usize))
    };
    let logs: HashSet<BlockStateId> = ctx.config.tree_kinds.iter().map(|t| t.log).collect();
    let (mut per_kind, mut border) = (vec![0usize; ctx.config.tree_kinds.len()], 0usize);
    for c in &after {
        for z in 0..16 {
            for x in 0..16 {
                for y in -63..319 {
                    let log = c.get(x, y, z);
                    if !logs.contains(&log) || c.get(x, y + 1, z) == log {
                        continue;
                    }
                    let Some(k) = ctx.config.tree_kinds.iter().position(|t| t.log == log) else {
                        continue;
                    };
                    let kind = &ctx.config.tree_kinds[k];
                    let (wx, wz) = (c.pos.x * 16 + x as i32, c.pos.z * 16 + z as i32);
                    let top = y + 1;
                    let mut base = y;
                    while c.get(x, base - 1, z) == log {
                        base -= 1;
                    }
                    // The crown cells of this shape: (dy from top, radius, keep corners).
                    let layers: Vec<(i32, i32, bool)> =
                        match kind.shape {
                            TreeShape::Round => (-2..=1)
                                .map(|dy| (dy, if dy >= 0 { 1 } else { 2 }, dy == -1))
                                .collect(),
                            TreeShape::Cone => std::iter::once((0, 0, false))
                                .chain((base + 2..top).rev().enumerate().map(|(i, yy)| {
                                    (yy - top, if i % 2 == 0 { 1 } else { 2 }, false)
                                }))
                                .collect(),
                        };
                    let mut crosses = false;
                    for (dy, r, corners) in layers {
                        for dz in -r..=r {
                            for dx in -r..=r {
                                if r > 0 && dx.abs() == r && dz.abs() == r && !corners {
                                    continue;
                                }
                                let (cx, cy, cz) = (wx + dx, top + dy, wz + dz);
                                let (Some(was), Some(now)) =
                                    (at(&before, cx, cy, cz), at(&after, cx, cy, cz))
                                else {
                                    continue; // outside the generated area
                                };
                                if cx.div_euclid(16) != c.pos.x || cz.div_euclid(16) != c.pos.z {
                                    crosses = true;
                                }
                                if was == BlockStateId::AIR {
                                    assert!(
                                        logs.contains(&now)
                                            || ctx
                                                .config
                                                .tree_kinds
                                                .iter()
                                                .any(|t| t.leaves == now),
                                        "crown hole at {cx},{cy},{cz} ({:?})",
                                        kind.shape
                                    );
                                }
                            }
                        }
                    }
                    per_kind[k] += 1;
                    border += usize::from(crosses);
                }
            }
        }
    }
    assert!(
        per_kind.iter().all(|&n| n > 3),
        "trees per kind: {per_kind:?}"
    );
    assert!(
        border > 0,
        "no crown crosses a border; the test proves nothing"
    );
}

#[test]
fn configs_are_checked() {
    let reg = registry();
    let src = source();
    let err = |text: &str| WorldgenConfig::from_toml(text, &reg).err();
    assert!(matches!(
        err(&src.replace("engine = \"climate\"", "engine = \"voronoi\"")),
        Some(ConfigError::Invalid(_))
    ));
    let without_table = src.replace("[climate]", "[unused]");
    assert!(err(&without_table).is_some());
    assert!(matches!(
        err(&src.replace("trees = [\"birch\"]", "trees = [\"palm\"]")),
        Some(ConfigError::Invalid(_))
    ));
    assert!(matches!(
        err(&src.replace("engine = \"climate\"", "engine = \"classic\"")),
        Some(ConfigError::Invalid(_))
    ));
}
