//! Prints biome shares, sea share and tree count for an area (diagnostics).
use world::{ColumnPos, WorldBounds};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

fn main() {
    let mods = world::modset::source_build(world::modset::DEFAULT_BUILD).expect("build");
    let b = mods.registry_builder().expect("blocks");
    let reg = b.build().expect("registry");
    let cfg = WorldgenConfig::from_toml(&mods.read("worldgen/overworld.toml").expect("cfg"), &reg)
        .expect("cfg");
    let seed: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(7);
    let n: i32 = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    let ctx = GenContext::new(seed, cfg, WorldBounds::default());
    let p = Pipeline::overworld();
    let (mut biomes, mut sea, mut logs) = ([0usize; 8], 0usize, 0usize);
    let log = ctx.config.trees.log;
    for x in -n..n {
        for z in -n..n {
            let c = p.generate(&ctx, ColumnPos { x, z });
            for s in c.surface.iter() {
                biomes[usize::from(s.biome)] += 1;
                if s.height <= ctx.config.sea_level {
                    sea += 1;
                }
            }
            for zz in 0..16 {
                for xx in 0..16 {
                    for y in -64..320 {
                        if c.get(xx, y, zz) == log && c.get(xx, y + 1, zz) != log {
                            logs += 1;
                        }
                    }
                }
            }
        }
    }
    let total = (4 * n * n * 256) as f64;
    println!(
        "seed {seed}: biomes {:?} (of {}), sea {:.0}%, trees {logs}",
        &biomes[..ctx.config.biomes.len()],
        total,
        sea as f64 / total * 100.0
    );
}
