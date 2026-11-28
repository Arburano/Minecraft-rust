//! T4-01 (ADR-0018): optional subsystems start only for the build's
//! capabilities, and the errors name the build, the mod and the capability.
//!
//! A test subsystem for `zones` stands in for a real one: it starts a thread
//! named `cap-zones` and counts world ticks. A build without `zones` must not
//! call its factory, start its thread, tick it or announce it in `Welcome`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use auth::AdmissionPolicy;
use net::messages::{read_header, read_message, write_header, write_message};
use protocol::{ClientControl, Hello, ServerControl};
use server::capabilities::{Subsystem, SubsystemRegistry};
use server::{Server, ServerConfig};
use world::{Capability, ModSet};

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("server-caps-{tag}-{}", std::process::id()));
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

/// The source tree's mods plus test mods that require capabilities.
fn mods_root(tag: &str) -> PathBuf {
    let root = temp(&format!("{tag}-mods"));
    copy_dir(&world::modset::source_tree().0, &root);
    for (id, caps) in [
        ("zones_test", "\"zones\""),
        ("grids_test", "\"moving_grids\""),
        ("dims_test", "\"dimensions\""),
        ("inst_test", "\"instances\""),
    ] {
        let dir = root.join(id);
        std::fs::create_dir_all(dir.join("data")).expect("mkdir");
        std::fs::write(
            dir.join("mod.toml"),
            format!(
                "[mod]\nid = \"{id}\"\nversion = \"0.1.0\"\n\n[dependencies]\ncore = \"^0.1\"\n\n\
                 [requires]\ncapabilities = [{caps}]\n"
            ),
        )
        .expect("write");
    }
    root
}

fn build(root: &Path, id: &str, extra: &[&str]) -> ModSet {
    let mut ids = vec!["core", "movement", "overworld"];
    ids.extend_from_slice(extra);
    ModSet::resolve(id, &ids, root).expect("build")
}

/// What the test subsystem did.
#[derive(Default)]
struct Probe {
    started: AtomicU64,
    ticks: AtomicU64,
    stopped: AtomicBool,
}

struct ZonesTest {
    probe: Arc<Probe>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Subsystem for ZonesTest {
    fn tick(&mut self, _tick: u64) {
        self.probe.ticks.fetch_add(1, Ordering::Relaxed);
    }

    fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.probe.stopped.store(true, Ordering::Relaxed);
    }
}

fn registry(probe: &Arc<Probe>) -> SubsystemRegistry {
    let mut r = SubsystemRegistry::builtin();
    let probe = Arc::clone(probe);
    r.register(
        Capability::Zones,
        Arc::new(move |_ctx| {
            probe.started.fetch_add(1, Ordering::Relaxed);
            let stop = Arc::new(AtomicBool::new(false));
            let flag = Arc::clone(&stop);
            let thread = std::thread::Builder::new()
                .name("cap-zones".into())
                .spawn(move || {
                    while !flag.load(Ordering::Relaxed) {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                })
                .map_err(|e| e.to_string())?;
            Ok(Box::new(ZonesTest {
                probe: Arc::clone(&probe),
                stop,
                thread: Some(thread),
            }) as Box<dyn Subsystem>)
        }),
    );
    r
}

fn config(mods: ModSet, world: PathBuf, subsystems: SubsystemRegistry) -> ServerConfig {
    let mut c = ServerConfig::new(
        "127.0.0.1:0".parse().expect("addr"),
        AdmissionPolicy {
            allow_offline: true,
        },
        world,
        mods,
    );
    c.seed = Some(3);
    c.view_distance = 2;
    c.threads = 1;
    c.region_threads = 1;
    c.lan_announce = false;
    c.subsystems = subsystems;
    c
}

/// Threads of this process with a name (Linux; `None` elsewhere).
fn thread_named(name: &str) -> Option<bool> {
    let tasks = std::fs::read_dir("/proc/self/task").ok()?;
    Some(
        tasks.flatten().any(|t| {
            std::fs::read_to_string(t.path().join("comm")).is_ok_and(|c| c.trim() == name)
        }),
    )
}

/// The capability names the server sends in `Welcome`.
fn welcome_capabilities(addr: std::net::SocketAddr) -> Vec<String> {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let endpoint =
            net::client_endpoint_offline("0.0.0.0:0".parse().expect("addr")).expect("endpoint");
        let conn = endpoint
            .connect(addr, "localhost")
            .expect("connect")
            .await
            .expect("connected");
        let (mut send, mut recv) = conn.open_bi().await.expect("stream");
        write_header(&mut send).await.expect("header");
        write_message(
            &mut send,
            &ClientControl::Hello(Hello {
                client_version: "test".into(),
                player_name: "Caps".into(),
                session_jwt: None,
            }),
        )
        .await
        .expect("hello");
        read_header(&mut recv)
            .await
            .expect("header")
            .check()
            .expect("version");
        let Ok(Some(ServerControl::Welcome(w))) = read_message(&mut recv).await else {
            panic!("expected Welcome");
        };
        conn.close(0u32.into(), b"done");
        w.capabilities
    })
}

#[test]
fn a_build_without_capabilities_starts_no_subsystem() {
    let root = mods_root("none");
    let probe = Arc::new(Probe::default());
    let server = Server::start(config(
        build(&root, "classic", &[]),
        temp("none-world"),
        registry(&probe),
    ))
    .expect("start");
    std::thread::sleep(Duration::from_millis(300));
    assert!(server.capabilities().is_empty());
    assert_eq!(
        probe.started.load(Ordering::Relaxed),
        0,
        "factory not called"
    );
    assert_eq!(probe.ticks.load(Ordering::Relaxed), 0, "no tick system");
    assert_ne!(thread_named("cap-zones"), Some(true), "no thread");
    assert!(
        welcome_capabilities(server.local_addr()).is_empty(),
        "no message"
    );
    server.shutdown();
    assert!(!probe.stopped.load(Ordering::Relaxed));
}

#[test]
fn a_required_capability_starts_its_subsystem() {
    let root = mods_root("zones");
    let probe = Arc::new(Probe::default());
    let world = temp("zones-world");
    let server = Server::start(config(
        build(&root, "rpg_test", &["zones_test"]),
        world.clone(),
        registry(&probe),
    ))
    .expect("start");
    assert_eq!(server.capabilities().to_string(), "zones");
    assert_eq!(probe.started.load(Ordering::Relaxed), 1);
    let deadline = Instant::now() + Duration::from_secs(5);
    while probe.ticks.load(Ordering::Relaxed) < 3 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        probe.ticks.load(Ordering::Relaxed) >= 3,
        "ticked with the world"
    );
    assert_ne!(thread_named("cap-zones"), Some(false), "its thread runs");
    assert_eq!(welcome_capabilities(server.local_addr()), ["zones"]);
    server.shutdown();
    assert!(
        probe.stopped.load(Ordering::Relaxed),
        "shut down with the world"
    );

    // The world now holds zones data: a build without zones refuses it.
    let meta = std::fs::read_to_string(world.join("server.toml")).expect("server.toml");
    assert!(meta.contains("capabilities = [\"zones\"]"), "{meta}");
    let Err(e) = Server::start(config(
        build(&root, "classic", &[]),
        world.clone(),
        registry(&probe),
    )) else {
        panic!("classic must refuse a world with zones data");
    };
    let e = e.to_string();
    assert!(
        e.contains("capability zones") && e.contains("build classic"),
        "{e}"
    );
}

#[test]
fn an_unimplemented_capability_is_a_clear_error() {
    let root = mods_root("missing");
    // `instances` has no subsystem in this engine version (T4-09).
    let Err(e) = Server::start(config(
        build(&root, "space_test", &["inst_test"]),
        temp("missing-world"),
        SubsystemRegistry::builtin(),
    )) else {
        panic!("instances has no subsystem in this engine version");
    };
    let e = e.to_string();
    assert!(
        e.contains("build space_test")
            && e.contains("capability instances")
            && e.contains("mod inst_test")
            && e.contains("does not implement"),
        "{e}"
    );
}

/// Without the `moving_grids` feature (`cargo test -p server
/// --no-default-features`), a build requiring it fails at load.
#[test]
fn a_capability_compiled_out_is_a_clear_error() {
    let root = mods_root("grids");
    let Err(e) = Server::start(config(
        build(&root, "industrial_test", &["grids_test"]),
        temp("grids-world"),
        SubsystemRegistry::builtin(),
    )) else {
        panic!("moving_grids has no subsystem yet, and may be compiled out");
    };
    let e = e.to_string();
    assert!(
        e.contains("build industrial_test")
            && e.contains("capability moving_grids")
            && e.contains("mod grids_test"),
        "{e}"
    );
    if cfg!(feature = "moving_grids") {
        assert!(e.contains("does not implement"), "{e}");
    } else {
        assert!(e.contains("this server binary was built without it"), "{e}");
    }
}
