//! T2-06: regional multithreading on the real server.
//!
//! - Property test: 1,000 players moving around merge and split regions;
//!   none is lost or duplicated, regions never border each other, and the
//!   same inputs with the same partitioning give the same state hash,
//!   whatever the number of region threads.
//! - Players flying apart split a region and flying back merge it, while
//!   the world keeps simulating them.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::DVec3;
use protocol::{Angle16, Buttons, Intent};
use server::game::{ToConn, ToWorld, World, WorldSetup};
use server::partition::RegionId;
use server::storage::Storage;
use sim::physics::PhysicsConfig;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use world::storage::WorldStore;
use world::{ColumnPos, WorldBounds};
use worldgen::{GenContext, WorldgenConfig};

fn data() -> world::ModSet {
    world::modset::source_build(world::modset::DEFAULT_BUILD).expect("build")
}

fn world(tag: &str, view: i32, sim: i32, region_threads: usize) -> (World, PathBuf) {
    let dir = std::env::temp_dir().join(format!("server-regions-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("dir");
    let mods = data();
    let b = mods.registry_builder().expect("blocks");
    let registry = Arc::new(b.build().expect("registry"));
    let gen_src = mods.read("worldgen/overworld.toml").expect("gen");
    let ctx = Arc::new(GenContext::new(
        7,
        WorldgenConfig::from_toml(&gen_src, &registry).expect("gen"),
        WorldBounds::default(),
    ));
    let physics = PhysicsConfig::from_toml(&mods.read("movement/profile.toml").expect("physics"))
        .expect("physics");
    let store = WorldStore::open(&dir, &registry, WorldBounds::default()).expect("store");
    let storage = Storage::start(store, Arc::clone(&registry)).expect("storage");
    let w = World::new(
        WorldSetup {
            registry,
            context: ctx,
            physics,
            dir: dir.clone(),
            view_distance: view,
            sim_distance: sim,
            threads: 2,
            region_threads,
            entity_limits: Default::default(),
        },
        storage,
    )
    .expect("world");
    (w, dir)
}

/// splitmix64.
fn rng(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn column(p: DVec3) -> ColumnPos {
    ColumnPos {
        x: (p.x / 16.0).floor() as i32,
        z: (p.z / 16.0).floor() as i32,
    }
}

/// Runs the scripted partition workload and returns the state hash after
/// every step; checks the invariants on the way.
fn partition_run(region_threads: usize, seed: u64) -> Vec<u64> {
    const PLAYERS: usize = 1000;
    const SIM: i32 = 2;
    let (mut w, dir) = world(
        &format!("prop-{region_threads}-{seed}"),
        3,
        SIM,
        region_threads,
    );
    let mut s = seed;
    let mut keep: Vec<UnboundedReceiver<ToConn>> = Vec::new();
    let mut pos: HashMap<String, DVec3> = HashMap::new();
    // 1,000 players in 25 clusters over a 4,000-block square.
    for i in 0..PLAYERS {
        let cluster = (i % 25) as f64;
        let p = DVec3::new(
            (cluster % 5.0) * 800.0 + (rng(&mut s) % 300) as f64,
            80.0,
            (cluster / 5.0).floor() * 800.0 + (rng(&mut s) % 300) as f64,
        );
        let (tx, rx) = unbounded_channel();
        keep.push(rx);
        let id = format!("p{i:04}");
        w.insert_player_for_test(&id, p, tx);
        pos.insert(id, p);
    }
    let mut hashes = Vec::new();
    let mut seen_regions = HashSet::new();
    let mut max_regions = 0;
    let mut min_regions = usize::MAX;
    for step in 0..60 {
        // Random walks with long jumps: clusters drift, merge and split.
        let mut ids: Vec<String> = pos.keys().cloned().collect();
        ids.sort();
        for id in ids {
            let p = pos.get_mut(&id).expect("player");
            let r = rng(&mut s);
            let jump = if r % 10 == 0 { 120.0 } else { 12.0 };
            p.x += ((r >> 8) % 1000) as f64 / 1000.0 * 2.0 * jump - jump;
            p.z += ((r >> 24) % 1000) as f64 / 1000.0 * 2.0 * jump - jump;
            // Every 20 steps everyone is pulled towards the center, then let go.
            if step % 20 >= 10 {
                p.x += (2000.0 - p.x) * 0.15;
                p.z += (2000.0 - p.z) * 0.15;
            }
            w.teleport_for_test(&id, *p);
        }
        w.restructure();
        let regions = w.regions();
        // None lost, none duplicated.
        let mut all: Vec<&String> = regions.iter().flat_map(|(_, ids)| ids).collect();
        let n = all.len();
        all.sort();
        all.dedup();
        assert_eq!(n, PLAYERS, "step {step}: players lost or duplicated");
        assert_eq!(all.len(), PLAYERS, "step {step}: a player in two regions");
        assert_eq!(w.player_count(), PLAYERS);
        // Regions never border each other: areas of players in different
        // regions are at least one column apart.
        let mut owner: HashMap<ColumnPos, RegionId> = HashMap::new();
        for (rid, ids) in &regions {
            seen_regions.insert(*rid);
            for id in ids {
                let c = column(pos[id]);
                for dx in -(SIM + 1)..=(SIM + 1) {
                    for dz in -(SIM + 1)..=(SIM + 1) {
                        let a = ColumnPos {
                            x: c.x + dx,
                            z: c.z + dz,
                        };
                        if let Some(other) = owner.insert(a, *rid) {
                            assert_eq!(other, *rid, "step {step}: overlapping territories");
                        }
                    }
                }
            }
        }
        for (a, rid) in &owner {
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let b = ColumnPos {
                    x: a.x + dx,
                    z: a.z + dz,
                };
                if let Some(o) = owner.get(&b) {
                    assert_eq!(o, rid, "step {step}: regions border each other at {a:?}");
                }
            }
        }
        let with_players = regions.iter().filter(|(_, ids)| !ids.is_empty()).count();
        max_regions = max_regions.max(with_players);
        min_regions = min_regions.min(with_players);
        hashes.push(w.state_hash());
    }
    println!(
        "{region_threads} region threads: regions with players {min_regions}..{max_regions}, {} region IDs used",
        seen_regions.len()
    );
    assert!(max_regions > min_regions, "regions merged and split");
    assert!(
        seen_regions.len() > max_regions,
        "regions were created and retired"
    );
    drop(w);
    let _ = std::fs::remove_dir_all(dir);
    hashes
}

#[test]
fn thousand_players_merge_and_split_without_loss_and_reproducibly() {
    let a = partition_run(4, 42);
    let b = partition_run(4, 42);
    assert_eq!(a, b, "the same inputs give the same states");
    let c = partition_run(1, 42);
    assert_eq!(
        a, c,
        "the number of region threads does not change the result"
    );
}

fn intent(tick: u32, buttons: u16, yaw: f32, steps: i8) -> Intent {
    Intent {
        tick,
        buttons: Buttons(buttons),
        yaw: Angle16::from_radians(yaw),
        pitch: Angle16(0),
        fly_speed_steps: steps,
        state_hash: 0,
    }
}

/// Two players fly apart (the region splits) and back (it merges); both
/// keep moving all the time.
#[test]
fn flying_apart_splits_and_coming_back_merges() {
    let (mut w, dir) = world("fly", 3, 2, 2);
    let (tx, inbox) = crossbeam_channel::unbounded();
    let mut keep = Vec::new();
    for id in ["a", "b"] {
        let (out, rx) = unbounded_channel();
        keep.push(rx);
        tx.send(ToWorld::Join {
            id: id.into(),
            name: id.into(),
            out,
        })
        .expect("join");
    }
    // Ticks until both stand on generated ground.
    let start = Instant::now();
    let mut spawned = [false; 2];
    while !spawned.iter().all(|&s| s) {
        assert!(start.elapsed() < Duration::from_secs(60), "spawn");
        w.tick(&inbox);
        for (i, rx) in keep.iter_mut().enumerate() {
            while let Ok(m) = rx.try_recv() {
                spawned[i] |= matches!(m, ToConn::Spawn(_));
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // Let the ground around them load.
    for _ in 0..40 {
        w.tick(&inbox);
        std::thread::sleep(Duration::from_millis(10));
    }
    // "a" flies +x at full speed, then back; "b" stays.
    let mut tick = 0u32;
    let mut regions_seen = Vec::new();
    let fly = |tick: &mut u32, yaw: f32, first: bool| {
        let b = if first {
            Buttons::FLY_TOGGLE | Buttons::JUMP
        } else {
            Buttons::FORWARD | Buttons::SPRINT
        };
        let i = intent(*tick, b, yaw, if first { 10 } else { 0 });
        *tick += 1;
        i
    };
    let mut run =
        |w: &mut World, steps: u32, yaw: f32, tick: &mut u32, regions_seen: &mut Vec<usize>| {
            for k in 0..steps {
                let i = fly(tick, yaw, *tick == 0 && k == 0);
                tx.send(ToWorld::Intents {
                    id: "a".into(),
                    intents: vec![i],
                    ack_snapshot: 0,
                })
                .expect("intent");
                w.tick(&inbox);
                for rx in &mut keep {
                    while rx.try_recv().is_ok() {}
                }
                let n = w
                    .regions()
                    .iter()
                    .filter(|(_, ids)| !ids.is_empty())
                    .count();
                regions_seen.push(n);
                // Let generation keep up with the flight.
                std::thread::sleep(Duration::from_millis(10));
            }
        };
    let a0 = w.player_pos("a").expect("a");
    // yaw −π/2 looks towards +x.
    run(
        &mut w,
        120,
        -std::f32::consts::FRAC_PI_2,
        &mut tick,
        &mut regions_seen,
    );
    let a1 = w.player_pos("a").expect("a");
    let split = regions_seen.contains(&2);
    run(
        &mut w,
        160,
        std::f32::consts::FRAC_PI_2,
        &mut tick,
        &mut regions_seen,
    );
    let a2 = w.player_pos("a").expect("a");
    println!(
        "a: {a0} → {a1} → {a2}; regions with players over time: {:?}",
        regions_seen.iter().step_by(20).collect::<Vec<_>>()
    );
    assert!((a1.x - a0.x) > 100.0, "a flew away: {a0} → {a1}");
    assert!(split, "the region split while they were apart");
    assert_eq!(
        regions_seen.last(),
        Some(&1),
        "the region merged when a came back"
    );
    assert_eq!(w.player_count(), 2);
    drop(w);
    let _ = std::fs::remove_dir_all(dir);
}
