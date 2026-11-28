//! T4-08: the `zones` capability on the real world.
//!
//! A test mod requires `zones` and ships two zones around the world spawn:
//! a no-build town and, further east, a wall nobody may enter. Members
//! (`alice`) are exempt from both. The server must refuse the edits and the
//! steps, and the zone API must change the rules between ticks and save them
//! in the world.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use glam::DVec3;
use protocol::{Angle16, Buttons, Intent, ServerEvent};
use server::capabilities::{SubsystemContext, SubsystemRegistry};
use server::game::{Edit, ToConn, ToWorld, World, WorldSetup};
use server::storage::Storage;
use sim::physics::PhysicsConfig;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};
use world::coords::BlockPos;
use world::storage::WorldStore;
use world::zones::{Area, ZoneDef, ZoneRules};
use world::{Capability, ModSet, WorldBounds};
use worldgen::{GenContext, WorldgenConfig};

/// Where the town ends (x and z, inclusive) and the wall starts (x).
const TOWN: [i32; 2] = [-16, 31];
const WALL_X: i32 = 48;

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("server-zones-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("mkdir");
    dir
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("read_dir").flatten() {
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            std::fs::copy(&path, &target).expect("copy");
        }
    }
}

/// The source tree's mods plus `zones_rules`.
fn mods(tag: &str) -> ModSet {
    let root = temp(&format!("{tag}-mods"));
    copy_dir(&world::modset::source_tree().0, &root);
    let dir = root.join("zones_rules");
    std::fs::create_dir_all(dir.join("data/zones")).expect("mkdir");
    std::fs::write(
        dir.join("mod.toml"),
        "[mod]\nid = \"zones_rules\"\nversion = \"0.1.0\"\n\n[dependencies]\ncore = \"^0.1\"\n\n\
         [requires]\ncapabilities = [\"zones\"]\n",
    )
    .expect("write");
    std::fs::write(
        dir.join("data/zones/town.toml"),
        format!(
            "id = \"zones_rules:town\"\nname = \"Town\"\nmembers = [\"alice\"]\n\
             areas = [{{ x = [{a}, {b}], z = [{a}, {b}] }}]\nrules = {{ build = false }}\n",
            a = TOWN[0],
            b = TOWN[1]
        ),
    )
    .expect("write");
    std::fs::write(
        dir.join("data/zones/wall.toml"),
        format!(
            "id = \"zones_rules:wall\"\nmembers = [\"alice\"]\n\
             areas = [{{ x = [{WALL_X}, 80], z = [-300, 300] }}]\nrules = {{ entry = false }}\n"
        ),
    )
    .expect("write");
    ModSet::resolve(
        "rpg_test",
        &["core", "movement", "overworld", "zones_rules"],
        &root,
    )
    .expect("build")
}

fn world(tag: &str, mods: &ModSet, dir: &Path) -> World {
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
    let subsystems = SubsystemRegistry::builtin()
        .start(&SubsystemContext {
            mods,
            world_dir: dir,
        })
        .unwrap_or_else(|e| panic!("{tag}: {e}"));
    assert!(subsystems.capabilities().contains(Capability::Zones));
    let store = WorldStore::open(dir, &registry, WorldBounds::default()).expect("store");
    let storage = Storage::start(store, Arc::clone(&registry)).expect("storage");
    World::new(
        WorldSetup {
            registry,
            context: ctx,
            physics,
            dir: dir.to_path_buf(),
            view_distance: 4,
            sim_distance: 3,
            threads: 2,
            region_threads: 2,
            entity_limits: Default::default(),
            subsystems,
        },
        storage,
    )
    .expect("world")
}

struct Game {
    w: World,
    tx: crossbeam_channel::Sender<ToWorld>,
    inbox: crossbeam_channel::Receiver<ToWorld>,
    rx: Vec<(String, UnboundedReceiver<ToConn>)>,
    acks: Vec<(String, u32, bool)>,
}

impl Game {
    fn new(w: World, players: &[&str]) -> Self {
        let (tx, inbox) = crossbeam_channel::unbounded();
        let mut rx = Vec::new();
        for id in players {
            let (out, r) = unbounded_channel();
            rx.push(((*id).to_owned(), r));
            tx.send(ToWorld::Join {
                id: (*id).into(),
                name: (*id).into(),
                out,
            })
            .expect("join");
        }
        let mut g = Self {
            w,
            tx,
            inbox,
            rx,
            acks: Vec::new(),
        };
        // Until everyone spawned on generated ground, then let it load.
        let start = Instant::now();
        let mut spawned = vec![false; players.len()];
        while !spawned.iter().all(|&s| s) {
            assert!(start.elapsed() < Duration::from_secs(60), "spawn");
            g.w.tick(&g.inbox);
            for (i, (_, r)) in g.rx.iter_mut().enumerate() {
                while let Ok(m) = r.try_recv() {
                    spawned[i] |= matches!(m, ToConn::Spawn(_));
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        for _ in 0..40 {
            g.tick();
            std::thread::sleep(Duration::from_millis(10));
        }
        g
    }

    fn tick(&mut self) {
        self.w.tick(&self.inbox);
        for (id, r) in &mut self.rx {
            while let Ok(m) = r.try_recv() {
                if let ToConn::Event(ServerEvent::BlockAck { seq, accepted }) = m {
                    self.acks.push((id.clone(), seq, accepted));
                }
            }
        }
    }

    /// Digs `pos` as `id`; whether the server accepted it.
    fn dig(&mut self, id: &str, seq: u32, pos: BlockPos) -> bool {
        self.tx
            .send(ToWorld::Edit {
                id: id.into(),
                seq,
                pos,
                edit: Edit::Dig,
                allowed: true,
            })
            .expect("edit");
        self.tick();
        self.acks
            .iter()
            .rev()
            .find(|(who, s, _)| who == id && *s == seq)
            .map(|a| a.2)
            .unwrap_or_else(|| panic!("no ack for {id} #{seq}"))
    }

    fn pos(&self, id: &str) -> DVec3 {
        self.w.player_pos(id).expect("player")
    }
}

fn below(p: DVec3) -> BlockPos {
    BlockPos {
        x: p.x.floor() as i32,
        y: p.y.floor() as i32 - 1,
        z: p.z.floor() as i32,
    }
}

fn intent(tick: u32, buttons: u16, steps: i8) -> Intent {
    Intent {
        tick,
        buttons: Buttons(buttons),
        // −π/2 looks towards +x.
        yaw: Angle16::from_radians(-std::f32::consts::FRAC_PI_2),
        pitch: Angle16(0),
        fly_speed_steps: steps,
        state_hash: 0,
    }
}

#[test]
fn build_rights_are_enforced_and_the_zone_api_changes_them() {
    let mods = mods("build");
    let dir = temp("build-world");
    let mut g = Game::new(world("build", &mods, &dir), &["bob", "alice"]);
    let ground = below(g.pos("bob"));
    assert!(
        (TOWN[0]..=TOWN[1]).contains(&ground.x) && (TOWN[0]..=TOWN[1]).contains(&ground.z),
        "spawn is in the town: {ground:?}"
    );
    assert!(!g.dig("bob", 1, ground), "bob may not build in the town");
    // A member may.
    assert!(g.dig("alice", 1, ground), "alice is a member");

    // The zone API: a world zone with a higher priority opens a plot for
    // everyone around the spawn.
    let control = g.w.zone_control().expect("zones run");
    control
        .define(ZoneDef {
            id: "srv:plot".into(),
            name: "Plot".into(),
            priority: 10,
            difficulty: 0,
            areas: vec![Area {
                x: [0, 15],
                z: [0, 15],
                y: None,
            }],
            members: Vec::new(),
            rules: ZoneRules {
                build: Some(true),
                ..ZoneRules::default()
            },
            spawns: None,
            replaces: false,
        })
        .expect("define");
    g.tick(); // the change applies at the end of this tick
    let next = BlockPos {
        y: ground.y - 1,
        ..ground
    };
    assert!(g.dig("bob", 2, next), "the plot allows building");
    let saved = world::zones::read_world(&dir).expect("saved");
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].id, "srv:plot");
    // Invalid definitions are refused before they reach the world.
    let bad = ZoneDef {
        id: "no namespace".into(),
        ..saved[0].clone()
    };
    assert!(control.define(bad).is_err());

    control.remove("srv:plot").expect("remove");
    g.tick();
    let deeper = BlockPos {
        y: ground.y - 2,
        ..ground
    };
    assert!(!g.dig("bob", 3, deeper), "the town's rule is back");
    assert!(world::zones::read_world(&dir).expect("saved").is_empty());
    drop(g);

    // The world's zones survive a restart.
    control
        .define(saved[0].clone())
        .expect_err("the old world stopped");
    world::zones::write_world(&dir, &saved).expect("write");
    let mut g = Game::new(world("build-again", &mods, &dir), &["bob"]);
    let ground = below(g.pos("bob"));
    assert!(g.dig("bob", 1, ground), "the saved plot is loaded");
}

#[test]
fn entry_is_refused_at_the_border_and_members_pass() {
    let mods = mods("entry");
    let dir = temp("entry-world");
    let mut g = Game::new(world("entry", &mods, &dir), &["bob", "alice"]);
    let corrections = g.w.corrections();
    let start = Instant::now();
    let mut bob_max: f64 = f64::MIN;
    // Ticks bob has spent pushing against the border.
    let mut pushing = 0;
    for t in 0..900u32 {
        assert!(start.elapsed() < Duration::from_secs(120), "flight");
        for id in ["bob", "alice"] {
            let i = if t == 0 {
                intent(0, Buttons::FLY_TOGGLE | Buttons::JUMP, 10)
            } else {
                intent(t, Buttons::FORWARD | Buttons::SPRINT, 0)
            };
            g.tx.send(ToWorld::Intents {
                id: id.into(),
                intents: vec![i],
                ack_snapshot: 0,
            })
            .expect("intent");
        }
        g.tick();
        bob_max = bob_max.max(g.pos("bob").x);
        std::thread::sleep(Duration::from_millis(10));
        if bob_max > f64::from(WALL_X) - 2.0 {
            pushing += 1;
        }
        if g.pos("alice").x > f64::from(WALL_X) + 20.0 && pushing >= 30 {
            break;
        }
    }
    let (bob, alice) = (g.pos("bob"), g.pos("alice"));
    assert!(
        bob_max < f64::from(WALL_X),
        "bob never stands inside the wall (max x {bob_max:.2})"
    );
    assert!(
        bob.x > f64::from(WALL_X) - 2.0,
        "bob reached the border: {bob}"
    );
    assert!(alice.x > f64::from(WALL_X) + 20.0, "alice passed: {alice}");
    assert!(pushing >= 30, "bob pushed against the border");
    // After a refusal the speed is zero, so a few steps stay in the last
    // allowed cell before the next one is refused.
    let refused = g.w.zone_refusals();
    assert!(refused >= 3, "pushes were refused: {refused}");
    assert!(g.w.corrections() > corrections, "bob's client is corrected");
}
