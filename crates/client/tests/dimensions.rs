//! T4-02 part 2: one server runs two dimensions; a client travels between
//! them, keeps nothing of the dimension it left, and a returning player
//! joins the dimension it left in.
//!
//! The second dimension comes from a test module (`moon_test`) written into
//! a copy of the source mods: it requires the `dimensions` capability,
//! provides `worldgen:moon` (the classic engine) and defines `moon` with
//! another height and half the player gravity.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use auth::AdmissionPolicy;
use client::session::{Session, SessionConfig};
use server::{Server, ServerConfig};
use sim::physics::PlayerInput;
use world::{ModSet, WorldBounds};

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("client-dims-{tag}-{}", std::process::id()));
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

/// The moon: blocks from -64 to 256 (20 sections, the overworld has 24).
const MOON_BOUNDS: WorldBounds = WorldBounds {
    min_section_y: -4,
    section_count: 20,
};

/// A copy of the source mods plus a module adding the `moon` dimension;
/// `capability` decides whether it requires `dimensions`.
fn mods_root(tag: &str, capability: bool) -> PathBuf {
    let root = temp(&format!("{tag}-mods"));
    let (source, _) = world::modset::source_tree();
    copy_dir(&source, &root);
    let dir = root.join("moon_test");
    std::fs::create_dir_all(dir.join("data/dimensions")).expect("mkdir");
    std::fs::create_dir_all(dir.join("data/worldgen")).expect("mkdir");
    let requires = if capability {
        "\n[requires]\ncapabilities = [\"dimensions\"]\n"
    } else {
        ""
    };
    std::fs::write(
        dir.join("mod.toml"),
        format!(
            "[mod]\nid = \"moon_test\"\nversion = \"0.1.0\"\nprovides = [\"worldgen:moon\"]\n\n\
             [dependencies]\ncore = \"^0.1\"\n{requires}"
        ),
    )
    .expect("write");
    std::fs::write(
        dir.join("data/dimensions/moon.toml"),
        "id = \"moon\"\ngenerator = \"worldgen/moon.toml\"\nheight = [-64, 256]\nsky = \"fixed\"\n\
         [physics]\nplayer_gravity_scale = 0.5\npressure = 0.0\n",
    )
    .expect("write");
    std::fs::copy(
        source.join("overworld/data/worldgen/overworld.toml"),
        dir.join("data/worldgen/moon.toml"),
    )
    .expect("copy generator");
    root
}

fn build(root: &Path) -> ModSet {
    ModSet::resolve(
        "moon_build",
        &["core", "movement", "overworld", "moon_test"],
        root,
    )
    .expect("build")
}

fn server_config(dir: &Path, mods: ModSet) -> ServerConfig {
    let mut config = ServerConfig::new(
        "127.0.0.1:0".parse().expect("addr"),
        AdmissionPolicy {
            allow_offline: true,
        },
        dir.to_owned(),
        mods,
    );
    config.seed = Some(11);
    config.view_distance = 10;
    config.threads = 2;
    config
}

fn session_config(mods: ModSet) -> SessionConfig {
    let mut c = SessionConfig::new(mods);
    c.radius = 8;
    c.threads = 2;
    c
}

/// Runs frames (`update` every 5 ms, a tick every 50 ms) until `done` or `limit`.
fn run(s: &mut Session, limit: Duration, mut done: impl FnMut(&Session) -> bool) -> bool {
    let start = Instant::now();
    let mut next_tick = start;
    let input = PlayerInput::default();
    while start.elapsed() < limit {
        s.update().expect("session update");
        if done(s) {
            return true;
        }
        if Instant::now() >= next_tick {
            s.fixed_tick(&input);
            next_tick += Duration::from_millis(50);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

/// Every column the client holds has the bounds of its dimension.
fn only_columns_of(s: &Session, bounds: WorldBounds) {
    assert_eq!(s.map().bounds(), bounds);
    let mut n = 0;
    for c in s.map().column_positions() {
        let col = s.map().column(c).expect("listed column");
        assert_eq!(col.bounds(), bounds, "column {c:?} of another dimension");
        n += 1;
    }
    assert!(n > 0, "no columns of the new dimension");
}

const LIMIT: Duration = Duration::from_secs(90);

#[test]
fn a_player_travels_between_two_dimensions_and_rejoins_where_it_left() {
    let root = mods_root("travel", true);
    let dir = temp("travel-world");
    let server = Server::start(server_config(&dir, build(&root))).expect("server");
    // Sorted ids are the wire indices.
    assert_eq!(server.dimensions(), ["moon", "overworld"]);
    let (moon, overworld) = (0u8, 1u8);

    let mut s = Session::connect(
        server.local_addr(),
        "Traveler",
        session_config(build(&root)),
    )
    .expect("join");
    // New players start in the overworld.
    assert_eq!(s.dimension(), overworld);
    assert!(
        run(&mut s, LIMIT, Session::is_playing),
        "never played in the overworld"
    );
    only_columns_of(&s, WorldBounds::default());
    let gravity = s.physics().gravity;

    // Offline servers accept the travel command in chat.
    s.send_chat("/dimension moon");
    assert!(
        run(&mut s, LIMIT, |s| s.dimension() == moon && s.is_playing()),
        "never played on the moon"
    );
    assert_eq!(s.travels(), 1);
    only_columns_of(&s, MOON_BOUNDS);
    assert!((s.physics().gravity - gravity * 0.5).abs() < 1e-12);
    // Without a position the player arrives flying high above the same x
    // and z (the server's choice, sent in the new spawn).
    assert!(s.player().flying);
    s.close().expect("close");

    // The player file records the dimension; the player joins there again.
    let id = auth::Identity::offline("Traveler")
        .expect("name")
        .id()
        .to_string();
    let deadline = Instant::now() + Duration::from_secs(10);
    let saved = loop {
        let saved = server::players::load(&dir, &id).and_then(|p| p.dimension);
        if saved.is_some() || Instant::now() > deadline {
            break saved;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(saved.as_deref(), Some("moon"));
    let mut s = Session::connect(
        server.local_addr(),
        "Traveler",
        session_config(build(&root)),
    )
    .expect("rejoin");
    assert_eq!(s.dimension(), moon);
    assert!(
        run(&mut s, LIMIT, Session::is_playing),
        "never played on rejoin"
    );
    only_columns_of(&s, MOON_BOUNDS);

    // The server API moves players too (what a portal mod would call).
    server.travel(&id, "overworld", None);
    assert!(
        run(&mut s, LIMIT, |s| s.dimension() == overworld
            && s.is_playing()),
        "never came back to the overworld"
    );
    only_columns_of(&s, WorldBounds::default());
    assert_eq!(s.physics().gravity, gravity);
    s.close().expect("close");
    let deadline = Instant::now() + Duration::from_secs(10);
    while server::players::load(&dir, &id).is_some_and(|p| p.dimension.is_some())
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(50));
    }
    // The overworld is the default: the file names no dimension.
    assert_eq!(
        server::players::load(&dir, &id).and_then(|p| p.dimension),
        None
    );
    // The moon's columns are stored apart from the overworld's.
    assert!(dir.join("dimensions/moon").is_dir());
    drop(server);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn several_dimensions_need_the_capability() {
    let root = mods_root("nocap", false);
    let dir = temp("nocap-world");
    let Err(e) = Server::start(server_config(&dir, build(&root))) else {
        panic!("a build with two dimensions and no `dimensions` capability started");
    };
    let msg = e.to_string();
    assert!(msg.contains("dimensions") && msg.contains("moon"), "{msg}");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&root);
}
