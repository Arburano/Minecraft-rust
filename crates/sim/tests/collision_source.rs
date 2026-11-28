//! Physics and raycasts never read mesh output (ADR-0016, decision 3; T1-23):
//! collisions come from the registry's collision shapes only.
//!
//! Two checks: the simulation and server crates do not depend on the mesher
//! or the renderer at all, and changing a block's visual properties (opacity
//! masks, mesher) leaves collision queries and raycasts unchanged, while
//! changing its collision shape does change them.

use std::sync::Arc;

use glam::DVec3;
use sim::physics::{BlockQuery, MapBlocks};
use sim::raycast::raycast;
use world::coords::BlockPos;
use world::{
    BlockDef, BlockProps, Collision, Column, ColumnPos, MesherKind, Registry, RegistryBuilder,
    WorldBounds, WorldMap,
};

const BOUNDS: WorldBounds = WorldBounds {
    min_section_y: 0,
    section_count: 2,
};

/// Crates that produce or consume visual meshes.
const VISUAL_CRATES: [&str; 2] = ["mesh", "render"];

fn dependencies(manifest: &str) -> Vec<String> {
    let doc: toml::Table = toml::from_str(manifest).expect("valid manifest");
    ["dependencies", "dev-dependencies", "build-dependencies"]
        .iter()
        .filter_map(|k| doc.get(*k).and_then(toml::Value::as_table))
        .flat_map(|t| t.keys().cloned())
        .collect()
}

#[test]
fn simulation_crates_do_not_depend_on_meshes() {
    let crates = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .to_path_buf();
    for name in ["sim", "server"] {
        let path = crates.join(name).join("Cargo.toml");
        let manifest = std::fs::read_to_string(&path).expect("readable manifest");
        let deps = dependencies(&manifest);
        for visual in VISUAL_CRATES {
            assert!(
                !deps.iter().any(|d| d == visual),
                "{name} depends on {visual}: physics must not read visual meshes"
            );
        }
    }
}

/// A world with a 5x3x5 slab of `t:block` at y 10..13 and one block at
/// (8, 20, 8).
fn world_with(props: BlockProps) -> WorldMap {
    let mut b = RegistryBuilder::new();
    b.add_mod("t", vec![BlockDef::simple("t:block", props)]);
    let reg: Arc<Registry> = Arc::new(b.build().expect("valid registry"));
    let id = reg.state_id("t:block").expect("registered");
    let mut map = WorldMap::new(Arc::clone(&reg), BOUNDS);
    map.insert_column(Column::new(ColumnPos { x: 0, z: 0 }, BOUNDS))
        .expect("column fits");
    for x in 6..11 {
        for z in 6..11 {
            for y in 10..13 {
                map.set_block(BlockPos::new(x, y, z), id).expect("loaded");
            }
        }
    }
    map.set_block(BlockPos::new(8, 20, 8), id).expect("loaded");
    map
}

/// Collision answers over the column plus every ray of a fan.
fn probe(map: &WorldMap) -> (Vec<bool>, Vec<Option<(BlockPos, i32)>>) {
    let q = MapBlocks { map, swim: &[] };
    let mut cells = Vec::new();
    for x in 0..16 {
        for z in 0..16 {
            for y in 0..32 {
                cells.push(q.collides(BlockPos::new(x, y, z)));
            }
        }
    }
    let origin = DVec3::new(8.5, 25.5, 8.5);
    let mut hits = Vec::new();
    for i in 0..64 {
        let a = f64::from(i) * 0.1;
        let dir = DVec3::new(a.cos() * 0.4, -1.0, a.sin() * 0.4);
        let hit = raycast(origin, dir, 30.0, |p| q.collides(p));
        hits.push(hit.map(|h| (h.block, h.normal.x + 3 * h.normal.y + 9 * h.normal.z)));
    }
    (cells, hits)
}

#[test]
fn physics_and_raycasts_ignore_visual_properties() {
    let solid = BlockProps {
        opaque: true,
        transparent: false,
        light_emission: 0,
        light_opacity: 15,
        collision: Collision::Full,
        mesher: MesherKind::Cubic,
    };
    // Same collision, different visual properties (meshing masks, light).
    let see_through = BlockProps {
        opaque: false,
        transparent: true,
        light_opacity: 0,
        light_emission: 9,
        ..solid
    };
    let base = probe(&world_with(solid));
    assert!(base.0.iter().any(|&c| c), "the slab collides");
    assert!(base.1.iter().all(Option::is_some), "every ray hits");
    assert_eq!(probe(&world_with(see_through)), base);

    // A different collision shape does change the answers.
    let ghost = BlockProps {
        collision: Collision::None,
        ..solid
    };
    let ghost = probe(&world_with(ghost));
    assert_ne!(ghost, base);
    // Rays pass through the blocks and stop only at the world floor or at
    // unloaded cells, both of which physics treats as solid.
    let is_block = |p: BlockPos| {
        ((6..11).contains(&p.x) && (10..13).contains(&p.y) && (6..11).contains(&p.z))
            || (p.x, p.y, p.z) == (8, 20, 8)
    };
    assert!(
        ghost.1.iter().flatten().all(|&(p, _)| !is_block(p)),
        "rays pass through blocks without collision"
    );
}
