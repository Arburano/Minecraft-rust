//! Times the phases of one world-path edit (diagnostics for T1-09).
use std::sync::Arc;
use std::time::Instant;

use light::{WorldLight, light_column, props_from_registry, snapshot_from_world};
use world::coords::BlockPos;
use world::registry::blocks_from_dir;
use world::{ColumnPos, RegistryBuilder, WorldBounds, WorldMap};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

fn main() {
    let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data");
    let mut b = RegistryBuilder::new();
    b.add_mod(
        "core",
        blocks_from_dir(&data.join("blocks")).expect("blocks"),
    );
    let reg = Arc::new(b.build().expect("registry"));
    let props = props_from_registry(&reg);
    let src = std::fs::read_to_string(data.join("worldgen/overworld.toml")).expect("cfg");
    let ctx = GenContext::new(
        7,
        WorldgenConfig::from_toml(&src, &reg).expect("cfg"),
        WorldBounds::default(),
    );
    let p = Pipeline::overworld();
    let mut map = WorldMap::new(Arc::clone(&reg), WorldBounds::default());
    for x in -3..=3 {
        for z in -3..=3 {
            map.insert_column(
                p.generate(&ctx, ColumnPos { x, z })
                    .into_column(&reg)
                    .expect("b"),
            )
            .expect("b");
        }
    }
    let mut light = WorldLight::new();
    for x in -2..=2 {
        for z in -2..=2 {
            light_column(&map, &mut light, ColumnPos { x, z }, &props).expect("lit");
        }
    }
    let pos = BlockPos::new(3, map.height(3, 3).expect("h"), 3);
    let center = pos.section();
    let n = 200;
    let t = Instant::now();
    for _ in 0..n {
        std::hint::black_box(snapshot_from_world(&map, &light, center, &props, false).expect("s"));
    }
    let build = t.elapsed().as_secs_f64() * 1e6 / f64::from(n);
    let t = Instant::now();
    for _ in 0..n {
        std::hint::black_box(snapshot_from_world(&map, &light, center, &props, true).expect("s"));
    }
    let build_import = t.elapsed().as_secs_f64() * 1e6 / f64::from(n);
    let snap = snapshot_from_world(&map, &light, center, &props, true).expect("s");
    let t = Instant::now();
    for _ in 0..n {
        for dy in 0..3 {
            for dz in 0..3 {
                for dx in 0..3 {
                    std::hint::black_box(snap.section_light(dx, dy, dz));
                }
            }
        }
    }
    let export = t.elapsed().as_secs_f64() * 1e6 / f64::from(n);
    println!(
        "build {build:.0} us, build+import+freeze {build_import:.0} us (import+freeze {:.0}), export 27 {export:.0} us",
        build_import - build
    );
}
