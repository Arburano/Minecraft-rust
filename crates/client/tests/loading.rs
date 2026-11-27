//! T1-10: the loading queue fills the area, lights and meshes it correctly,
//! cancels work that left the area and always makes progress.

use std::sync::Arc;
use std::time::{Duration, Instant};

use client::loading::{LoadConfig, Loader, Viewer};
use engine_core::jobs::JobPool;
use light::{WorldLight, light_column, props_from_registry};
use world::coords::SectionPos;
use world::{ColumnPos, Registry, WorldBounds, WorldMap};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

const BOUNDS: WorldBounds = WorldBounds {
    min_section_y: 0,
    section_count: 8,
};

struct Setup {
    reg: Arc<Registry>,
    loader: Loader,
    map: WorldMap,
    light: WorldLight,
    pool: JobPool,
}

fn setup(config: LoadConfig) -> Setup {
    let mods = world::modset::source_build(world::modset::DEFAULT_BUILD).expect("build");
    let b = mods.registry_builder().expect("blocks");
    let reg = Arc::new(b.build().expect("registry"));
    let src = mods.read("worldgen/overworld.toml").expect("config");
    let ctx = Arc::new(GenContext::new(
        11,
        WorldgenConfig::from_toml(&src, &reg).expect("config"),
        BOUNDS,
    ));
    let loader = Loader::new(
        config,
        Arc::clone(&reg),
        Arc::new(Pipeline::overworld()),
        ctx,
    );
    Setup {
        map: WorldMap::new(Arc::clone(&reg), BOUNDS),
        reg,
        loader,
        light: WorldLight::new(),
        pool: JobPool::new(4).expect("pool"),
    }
}

fn area(center: ColumnPos, r: i32) -> impl Iterator<Item = ColumnPos> {
    (-r..=r).flat_map(move |dx| {
        (-r..=r).map(move |dz| ColumnPos {
            x: center.x + dx,
            z: center.z + dz,
        })
    })
}

/// Ticks until every column within `r` of the viewer is ready.
fn run_until_ready(s: &mut Setup, viewer: &Viewer, r: i32, timeout: Duration) -> Duration {
    let start = Instant::now();
    loop {
        s.loader.tick(viewer, &mut s.map, &mut s.light, &s.pool);
        if area(viewer.column(), r).all(|c| s.loader.is_ready(&s.map, c)) {
            return start.elapsed();
        }
        assert!(
            start.elapsed() < timeout,
            "area of radius {r} not ready in {timeout:?}"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn area_loads_and_light_matches_a_full_computation() {
    let mut s = setup(LoadConfig {
        radius: 3,
        unload_margin: 1,
        ..LoadConfig::default()
    });
    let viewer = Viewer {
        pos: [8.0, 70.0, 8.0],
        velocity: [0.0; 3],
    };
    let near = run_until_ready(&mut s, &viewer, 1, Duration::from_secs(120));
    let all = run_until_ready(&mut s, &viewer, 3, Duration::from_secs(120));
    eprintln!(
        "3x3 ready after {near:?}, radius 3 after {all:?}, stats {:?}",
        s.loader.stats()
    );
    let props = props_from_registry(&s.reg);
    for c in area(viewer.column(), 2) {
        let mut fresh = WorldLight::new();
        light_column(&s.map, &mut fresh, c, &props).expect("neighbors loaded");
        for sy in 0..8 {
            let sp = SectionPos {
                x: c.x,
                y: sy,
                z: c.z,
            };
            assert_eq!(s.light.get(sp), fresh.get(sp), "{sp:?}");
        }
    }
    assert!(!s.loader.meshes().is_empty());
    assert!(
        s.loader.meshes().values().any(|m| !m.is_empty()),
        "some geometry"
    );
}

#[test]
fn moving_away_cancels_and_unloads() {
    let mut s = setup(LoadConfig {
        radius: 3,
        unload_margin: 1,
        max_in_flight: 32,
        ..LoadConfig::default()
    });
    let start = Viewer {
        pos: [8.0, 70.0, 8.0],
        velocity: [0.0; 3],
    };
    // A couple of ticks: jobs are in flight, nothing finished yet.
    for _ in 0..2 {
        s.loader.tick(&start, &mut s.map, &mut s.light, &s.pool);
    }
    let far = Viewer {
        pos: [1608.0, 70.0, 8.0],
        velocity: [0.0; 3],
    };
    run_until_ready(&mut s, &far, 1, Duration::from_secs(120));
    let stats = s.loader.stats();
    assert!(
        stats.cancelled > 0,
        "jobs for the old area were cancelled: {stats:?}"
    );
    let keep = 3 + 2 + 1; // radius + mesh rings + unload margin
    for c in s.map.column_positions() {
        assert!(
            (c.x - far.column().x).abs() <= keep && (c.z - far.column().z).abs() <= keep,
            "{c:?} not unloaded"
        );
    }
    for c in area(start.column(), 3) {
        assert!(
            s.light
                .get(SectionPos {
                    x: c.x,
                    y: 3,
                    z: c.z
                })
                .is_none()
        );
    }
    assert!(
        s.loader
            .meshes()
            .keys()
            .all(|m| (m.x * 2 - far.column().x).abs() <= keep + 2)
    );
}

#[test]
fn a_tiny_apply_budget_still_makes_progress() {
    let mut s = setup(LoadConfig {
        radius: 1,
        unload_margin: 1,
        apply_budget: Duration::from_nanos(1),
        ..LoadConfig::default()
    });
    let viewer = Viewer {
        pos: [8.0, 70.0, 8.0],
        velocity: [0.0; 3],
    };
    run_until_ready(&mut s, &viewer, 1, Duration::from_secs(120));
}
