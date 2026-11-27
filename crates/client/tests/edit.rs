//! T1-12: block edits keep meshes, light and render batches equal to a fresh
//! build of the edited world.

use std::sync::Arc;
use std::time::{Duration, Instant};

use client::edit::{BatchSync, Editor, Placer, batch_of, dig, pick, place};
use client::loading::{LoadConfig, Loader, Viewer};
use engine_core::jobs::JobPool;
use glam::DVec3;
use light::{WorldLight, light_column, props_from_registry};
use mesh::reference::covered_faces_sorted;
use mesh::{MeshChunkBuilder, SectionNeighborhood};
use sim::physics::{PhysicsConfig, PlayerBody};
use world::coords::{BlockPos, MeshChunkPos, SectionPos};
use world::{BlockStateId, ColumnPos, Registry, Section, WorldBounds, WorldMap};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

const BOUNDS: WorldBounds = WorldBounds {
    min_section_y: 0,
    section_count: 8,
};

struct World {
    reg: Arc<Registry>,
    loader: Loader,
    map: WorldMap,
    light: WorldLight,
    pool: JobPool,
    viewer: Viewer,
}

fn loaded_world() -> World {
    let mods = world::modset::source_build(world::modset::DEFAULT_BUILD).expect("build");
    let b = mods.registry_builder().expect("blocks");
    let reg = Arc::new(b.build().expect("registry"));
    let src = mods.read("worldgen/overworld.toml").expect("config");
    let ctx = Arc::new(GenContext::new(
        11,
        WorldgenConfig::from_toml(&src, &reg).expect("config"),
        BOUNDS,
    ));
    let mut loader = Loader::new(
        LoadConfig {
            radius: 2,
            ..LoadConfig::default()
        },
        Arc::clone(&reg),
        Arc::new(Pipeline::overworld()),
        ctx,
    );
    loader.track_mesh_changes();
    let mut w = World {
        map: WorldMap::new(Arc::clone(&reg), BOUNDS),
        reg,
        loader,
        light: WorldLight::new(),
        pool: JobPool::new(4).expect("pool"),
        viewer: Viewer {
            pos: [8.0, 80.0, 8.0],
            velocity: [0.0; 3],
        },
    };
    let start = Instant::now();
    let ready = |w: &World| {
        (-2..=2).all(|x| (-2..=2).all(|z| w.loader.is_ready(&w.map, ColumnPos { x, z })))
    };
    while !ready(&w) {
        w.loader.tick(&w.viewer, &mut w.map, &mut w.light, &w.pool);
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "world not ready"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    w
}

fn fresh_mesh(map: &WorldMap, reg: &Registry, m: MeshChunkPos) -> Vec<mesh::reference::UnitFace> {
    let n = SectionNeighborhood::from_fn(m, |sp| {
        map.column(ColumnPos { x: sp.x, z: sp.z })
            .and_then(|c| c.section(sp.y))
            .cloned()
            .unwrap_or_else(Section::new)
    });
    covered_faces_sorted(&MeshChunkBuilder::new().build(&n, |b| reg.props(b).transparent))
}

fn assert_meshes_fresh(w: &World, pos: BlockPos) {
    for &m in mesh::affected_mesh_chunks(pos).as_slice() {
        let stored = w.loader.meshes().get(&m).expect("meshed");
        assert_eq!(
            covered_faces_sorted(stored),
            fresh_mesh(&w.map, &w.reg, m),
            "mesh chunk {m:?} after an edit at {pos:?}"
        );
    }
}

fn assert_light_fresh(w: &World, around: ColumnPos) {
    let props = props_from_registry(&w.reg);
    let mut fresh = WorldLight::new();
    light_column(&w.map, &mut fresh, around, &props).expect("light");
    for y in 0..BOUNDS.section_count as i32 {
        let sp = SectionPos {
            x: around.x,
            y,
            z: around.z,
        };
        assert_eq!(w.light.get(sp), fresh.get(sp), "light of {sp:?}");
    }
}

/// The topmost block (the heightmap gives the first open cell above it).
fn surface(map: &WorldMap, x: i32, z: i32) -> BlockPos {
    BlockPos::new(x, map.height(x, z).expect("loaded") - 1, z)
}

#[test]
fn dig_and_place_keep_meshes_and_light_exact() {
    let mut w = loaded_world();
    let mut editor = Editor::new(&w.map);
    let stone = w.reg.state_id("core:stone").expect("stone");
    let torch = w.reg.state_id("core:torch").expect("torch");
    // Edits on mesh chunk borders (x = 0 and 31, z = 0) and inside.
    let spots = [(0, 0), (31, 5), (7, 9), (-1, -1)];
    for (x, z) in spots {
        let top = surface(&w.map, x, z);
        editor
            .set_block(&mut w.map, &mut w.loader, top, BlockStateId::AIR)
            .expect("dig");
        assert_meshes_fresh(&w, top);
        let above = BlockPos::new(x, top.y + 3, z);
        editor
            .set_block(&mut w.map, &mut w.loader, above, stone)
            .expect("place");
        assert_meshes_fresh(&w, above);
        let lamp = BlockPos::new(x, top.y - 4, z);
        editor
            .set_block(&mut w.map, &mut w.loader, lamp, torch)
            .expect("torch");
        assert_meshes_fresh(&w, lamp);
    }
    while editor.pending_light() > 0 {
        editor
            .process_light(&w.map, &mut w.light, 1)
            .expect("relight");
    }
    for (x, z) in spots {
        assert_light_fresh(&w, ColumnPos::of_block(BlockPos::new(x, 0, z)));
    }
}

#[test]
fn ticks_after_edits_do_not_bring_stale_meshes_back() {
    let mut w = loaded_world();
    let mut editor = Editor::new(&w.map);
    let glass = w.reg.state_id("core:glass").expect("glass");
    let p = surface(&w.map, 3, 3);
    editor
        .set_block(&mut w.map, &mut w.loader, p, glass)
        .expect("edit");
    for _ in 0..50 {
        w.loader.tick(&w.viewer, &mut w.map, &mut w.light, &w.pool);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_meshes_fresh(&w, p);
}

#[test]
fn batch_sync_repacks_the_edited_batch() {
    let mut w = loaded_world();
    let mut sync = BatchSync::new();
    sync.collect(&mut w.loader);
    let mut first = 0;
    sync.upload(|_, _| first += 1);
    assert!(first > 0, "the initial load fills batches");

    let mut editor = Editor::new(&w.map);
    let p = surface(&w.map, 10, 10);
    editor
        .set_block(&mut w.map, &mut w.loader, p, BlockStateId::AIR)
        .expect("dig");
    sync.collect(&mut w.loader);
    let mut uploaded = Vec::new();
    sync.upload(|b, packed| uploaded.push((b, packed.len())));
    let m = p.mesh_chunk();
    assert!(uploaded.iter().any(|&(b, _)| b == batch_of(m)));
    let expect: usize = w
        .loader
        .meshes()
        .iter()
        .filter(|(k, _)| batch_of(**k) == batch_of(m))
        .map(|(_, q)| q.len())
        .sum();
    let got = uploaded
        .iter()
        .find(|(b, _)| *b == batch_of(m))
        .map(|x| x.1);
    assert_eq!(got, Some(expect));
}

#[test]
fn pick_dig_and_place_from_the_player_view() {
    let mut w = loaded_world();
    let mut editor = Editor::new(&w.map);
    let cfg = PhysicsConfig::default();
    let water = w.reg.state_id("core:water").expect("water");
    let swim = [water];
    let ground = surface(&w.map, 4, 4);
    let player = PlayerBody {
        pos: DVec3::new(4.5, f64::from(ground.y) + 1.0, 4.5),
        on_ground: true,
        ..Default::default()
    };
    // Look straight down: the hit is the block under the feet.
    let hit = pick(&w.map, player.eye(&cfg), DVec3::NEG_Y, cfg.reach, &swim).expect("hit");
    assert_eq!(hit.block, ground);
    // Placing there would put a block into the player.
    let dirt = w.reg.state_id("core:dirt").expect("dirt");
    let placed = place(
        &mut editor,
        &mut w.map,
        &mut w.loader,
        &hit,
        dirt,
        Placer {
            body: &player,
            cfg: &cfg,
            swim: &swim,
        },
    )
    .expect("place");
    assert_eq!(placed, None, "no block inside the player");
    dig(&mut editor, &mut w.map, &mut w.loader, &hit).expect("dig");
    assert_eq!(w.map.get_block(ground), Some(BlockStateId::AIR));
    // A player standing aside may place it back.
    let aside = PlayerBody {
        pos: player.pos + DVec3::new(3.0, 0.0, 0.0),
        ..player
    };
    let below = pick(&w.map, player.eye(&cfg), DVec3::NEG_Y, cfg.reach, &swim).expect("hit");
    let placed = place(
        &mut editor,
        &mut w.map,
        &mut w.loader,
        &below,
        dirt,
        Placer {
            body: &aside,
            cfg: &cfg,
            swim: &swim,
        },
    )
    .expect("place");
    assert_eq!(placed, Some(ground));
    assert_eq!(w.map.get_block(ground), Some(dirt));
}
