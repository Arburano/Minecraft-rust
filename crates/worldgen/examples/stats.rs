//! Prints biome shares, sea share, height range and tree count for an area
//! (diagnostics): `stats [seed] [radius in columns] [build]`.
use world::{ColumnPos, WorldBounds};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

fn main() {
    let build = std::env::args()
        .nth(3)
        .unwrap_or_else(|| world::modset::DEFAULT_BUILD.into());
    let mods = world::modset::source_build(&build).expect("build");
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
    let mut biomes = vec![0usize; ctx.config.biomes.len()];
    let (mut sea, mut logs, mut low, mut high) = (0usize, 0usize, i32::MAX, i32::MIN);
    let logs_of: Vec<_> = ctx.config.tree_kinds.iter().map(|t| t.log).collect();
    for x in -n..n {
        for z in -n..n {
            let c = p.generate(&ctx, ColumnPos { x, z });
            for s in c.surface.iter() {
                biomes[usize::from(s.biome)] += 1;
                low = low.min(s.height);
                high = high.max(s.height);
                if s.height <= ctx.config.sea_level {
                    sea += 1;
                }
            }
            for zz in 0..16 {
                for xx in 0..16 {
                    for y in -64..320 {
                        let b = c.get(xx, y, zz);
                        if logs_of.contains(&b) && c.get(xx, y + 1, zz) != b {
                            logs += 1;
                        }
                    }
                }
            }
        }
    }
    let total = (4 * n * n * 256) as f64;
    let names: Vec<String> = ctx
        .config
        .biomes
        .iter()
        .zip(&biomes)
        .map(|(b, n)| format!("{} {:.0}%", b.id, *n as f64 / total * 100.0))
        .collect();
    println!(
        "{build}, seed {seed}: {}; sea {:.0}%, height {low}..{high}, trees {logs}",
        names.join(", "),
        sea as f64 / total * 100.0
    );
}
