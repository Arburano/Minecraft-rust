//! T2-03: the client and the server simulate a player bit for bit alike.
//!
//! Both sides run `sim::physics::step` on intents converted by
//! `server::input`. This test runs a scripted 600-tick route (walking,
//! sprinting, jumping, turning, flying with wheel steps, landing) on a
//! generated world and compares the hash of every tick's state with a golden
//! value, so a debug build, a release build and any platform must agree.
//! Run it in both: `cargo test -p server --test determinism` and
//! `cargo test -p server --release --test determinism`.

use std::path::PathBuf;
use std::sync::Arc;

use protocol::Buttons;
use server::input::{apply_fly_steps, intent, physics_input, state_hash};
use sim::physics::{MapBlocks, PhysicsConfig, PlayerBody, PlayerInput, step};
use world::registry::blocks_from_dir;
use world::{ColumnPos, RegistryBuilder, WorldBounds, WorldMap};
use worldgen::{GenContext, Pipeline, WorldgenConfig};

/// FNV-1a of all 600 per-tick state hashes. Changing physics, the intent
/// conversion or world generation changes it: update it deliberately.
const GOLDEN: u32 = 0x170d_4d5d;

fn route(t: u32) -> (PlayerInput, f64, i32) {
    let yaw = f64::from(t) * 0.013;
    let mut i = PlayerInput {
        forward: 1.0,
        ..PlayerInput::default()
    };
    match t {
        0..=99 => {}
        100..=199 => i.sprint = true,
        200..=299 => {
            i.sprint = true;
            i.jump = t % 20 < 3;
            i.strafe = if t % 40 < 20 { 1.0 } else { -1.0 };
        }
        300 => i.toggle_fly = true,
        301..=449 => {
            i.jump = t < 340;
            i.sprint = true;
        }
        450..=519 => i.descend = true,
        _ => i.forward = -1.0,
    }
    let steps = if t == 310 {
        3
    } else if t == 400 {
        -2
    } else {
        0
    };
    (i, yaw, steps)
}

#[test]
fn scripted_route_matches_the_golden_hash() {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data");
    let mut b = RegistryBuilder::new();
    b.add_mod(
        "core",
        blocks_from_dir(&dir.join("blocks")).expect("blocks"),
    );
    let registry = Arc::new(b.build().expect("registry"));
    let gen_src = std::fs::read_to_string(dir.join("worldgen/overworld.toml")).expect("gen");
    let ctx = GenContext::new(
        7,
        WorldgenConfig::from_toml(&gen_src, &registry).expect("gen"),
        WorldBounds::default(),
    );
    let physics = PhysicsConfig::from_toml(
        &std::fs::read_to_string(dir.join("player/physics.toml")).expect("physics"),
    )
    .expect("physics");
    let swim = physics.swim_ids(&registry).expect("swim");
    let mut map = WorldMap::new(Arc::clone(&registry), WorldBounds::default());
    let pipeline = Pipeline::overworld();
    for x in -8..8 {
        for z in -8..8 {
            let c = pipeline
                .generate(&ctx, ColumnPos { x, z })
                .into_column(&registry)
                .expect("column");
            map.insert_column(c).expect("insert");
        }
    }
    let h = map.height(8, 8).expect("spawn column");
    let mut body = PlayerBody {
        pos: glam::DVec3::new(8.5, f64::from(h), 8.5),
        ..PlayerBody::default()
    };
    let mut fly = 1.0;
    let world = MapBlocks {
        map: &map,
        swim: &swim,
    };
    let mut all = 0x811c_9dc5u32;
    let mut flew = false;
    for t in 0..600 {
        let (input, yaw, steps) = route(t);
        let i = intent(t, &input, yaw, 0.0, steps);
        assert_eq!(i.buttons.has(Buttons::FLY_TOGGLE), input.toggle_fly);
        fly = apply_fly_steps(fly, i.fly_speed_steps, physics.fly_speed_range);
        step(&mut body, &physics_input(&i, fly), &physics, &world);
        flew |= body.flying;
        for b in state_hash(&body, fly).to_le_bytes() {
            all = (all ^ u32::from(b)).wrapping_mul(0x0100_0193);
        }
    }
    assert!(flew, "the route includes flight");
    println!("route hash {all:#010x}, final {:?}", body.pos);
    assert_eq!(
        all, GOLDEN,
        "the simulation changed (debug and release must agree)"
    );
}
