//! Dedicated server binary: region-threaded simulation, networking, persistence.
//!
//! Spec:
//! - `docs/architecture/server-regions.md`
//! - `docs/architecture/auth-and-assets.md` ("Offline mode"), `docs/legal.md` rule 7
//!
//! Status: world, join and column streaming (T2-02); entities, edits and
//! regions follow in T2-03, T2-04 and T2-06.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::process::ExitCode;

use auth::AdmissionPolicy;
use clap::Parser;
use server::{DEFAULT_PORT, DEFAULT_VIEW_DISTANCE, Server, ServerConfig};

#[derive(Parser, Debug)]
#[command(about = "Dedicated game server")]
struct Args {
    /// Admit players without a verified account (development and private tests
    /// only; legal.md rule 7). Binds to localhost unless `--bind` names a
    /// loopback or private LAN address.
    #[arg(long)]
    offline: bool,
    /// Address to listen on. Default: 0.0.0.0 online, 127.0.0.1 offline.
    #[arg(long)]
    bind: Option<IpAddr>,
    /// UDP port (QUIC).
    #[arg(long, default_value_t = DEFAULT_PORT)]
    port: u16,
    /// The world folder (created if missing).
    #[arg(long, value_name = "DIR", default_value = "world")]
    world: PathBuf,
    /// The build to run: an id from `builds/` or a build file (ADR-0017).
    #[arg(long, value_name = "ID|FILE", default_value = world::modset::DEFAULT_BUILD)]
    build: String,
    /// The folder of mod folders (default: `mods/` next to the executable, else the source tree).
    #[arg(long, value_name = "DIR")]
    mods_dir: Option<PathBuf>,
    /// Seed of a new world (an existing world keeps its seed).
    #[arg(long)]
    seed: Option<u64>,
    /// Streamed column radius; the client renders 2 less (≥ 10, so render distance ≥ 8).
    #[arg(long, default_value_t = DEFAULT_VIEW_DISTANCE)]
    view_distance: i32,
    /// Spawn this many test entities around the world spawn (T2-07; they
    /// wander with the engine's pathfinding; real mobs are stage 4).
    #[arg(long, default_value_t = 0)]
    test_walkers: usize,
    /// Write tick, memory and CPU statistics to this file when stopping
    /// (load runs, T2-05/T2-09); a line is also printed every 10 s.
    #[arg(long, value_name = "FILE")]
    stats_out: Option<PathBuf>,
    /// Seconds after the first player joins that the statistics skip
    /// (joining and column streaming).
    #[arg(long, default_value_t = 30)]
    stats_warmup: u64,
}

/// Loopback or private-network address (RFC 1918, IPv6 unique local / link local).
fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback() || v4.is_private() || v4.is_link_local(),
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

fn configure(args: &Args) -> Result<ServerConfig, String> {
    let ip = match (args.bind, args.offline) {
        (Some(ip), true) if !is_local(ip) => {
            return Err(format!(
                "offline mode may not listen on {ip}: it is not a loopback or private LAN \
                 address (an unspecified address like 0.0.0.0 also exposes public interfaces). \
                 Offline mode is for development and private tests only (docs/legal.md rule 7)."
            ));
        }
        (Some(ip), _) => ip,
        (None, true) => IpAddr::V4(Ipv4Addr::LOCALHOST),
        (None, false) => IpAddr::V4(Ipv4Addr::UNSPECIFIED),
    };
    if args.view_distance < 10 {
        return Err("--view-distance must be at least 10 (render distance ≥ 8, PRD §14)".into());
    }
    let mut config = ServerConfig::new(
        SocketAddr::new(ip, args.port),
        AdmissionPolicy {
            allow_offline: args.offline,
        },
        args.world.clone(),
        world::modset::locate(&args.build, args.mods_dir.as_deref())
            .map_err(|e| format!("build {}: {e}", args.build))?,
    );
    config.seed = args.seed;
    config.view_distance = args.view_distance;
    config.test_walkers = args.test_walkers;
    Ok(config)
}

fn main() -> ExitCode {
    let _profiler = engine_core::profile::start();
    let args = Args::parse();
    let config = match configure(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    if config.policy.allow_offline {
        eprintln!(
            "WARNING: offline mode — players are NOT verified. Development and private tests \
             only; listening on {} (docs/legal.md rule 7).",
            config.listen
        );
    }
    let server = match Server::start(config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::FAILURE;
        }
    };
    println!(
        "server: listening on {} (QUIC); press Enter or Ctrl+D to stop",
        server.local_addr()
    );
    for notice in server.notices() {
        println!("server: note: {notice}");
    }
    let log = server.tick_log();
    let started = std::time::Instant::now();
    let reporter = {
        let log = std::sync::Arc::clone(&log);
        std::thread::spawn(move || {
            let mut from = 0;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(10));
                let ticks = log.since(from);
                from = log.count();
                if ticks.is_empty() {
                    continue;
                }
                let times: Vec<_> = ticks.iter().map(|t| t.0).collect();
                let p = server::metrics::Percentiles::of(&times);
                let proc = server::metrics::Process::now();
                println!(
                    "server: {} players, {} regions; tick {}; memory {} MB",
                    ticks.last().map_or(0, |t| t.1),
                    log.regions(),
                    p.line(),
                    proc.memory.map_or(0, |m| m / (1 << 20)),
                );
            }
        })
    };
    let _ = reporter;
    // Stop on Enter or end of input; the world and players are saved.
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    if let Some(path) = &args.stats_out {
        let text = stats_summary(&log, args.stats_warmup, started.elapsed());
        match std::fs::write(path, &text) {
            Ok(()) => println!("server: statistics written to {}", path.display()),
            Err(e) => eprintln!("error: writing {}: {e}", path.display()),
        }
        print!("{text}");
    }
    server.shutdown();
    println!("server: stopped");
    ExitCode::SUCCESS
}

/// The load-run summary: ticks with players, after the warm-up.
fn stats_summary(
    log: &server::metrics::TickLog,
    warmup: u64,
    uptime: std::time::Duration,
) -> String {
    let ticks = log.since(0);
    let first = ticks.iter().position(|t| t.1 > 0).unwrap_or(ticks.len());
    let skip = first + (warmup as usize * usize::from(server::game::TICK_RATE));
    let steady: Vec<_> = ticks
        .iter()
        .skip(skip)
        .filter(|t| t.1 > 0)
        .map(|t| t.0)
        .collect();
    let players = ticks.iter().skip(skip).map(|t| t.1).max().unwrap_or(0);
    let p = server::metrics::Percentiles::of(&steady);
    let proc = server::metrics::Process::now();
    let cores = std::thread::available_parallelism().map_or(0, |n| n.get());
    format!(
        "server load run\n\
         uptime: {:.0} s, logical CPUs: {cores}\n\
         players (max after warm-up): {players}\n\
         regions with players (last tick): {}\n\
         tick after {warmup} s warm-up: {}\n\
         memory: {}\n\
         CPU time: {}\n",
        uptime.as_secs_f64(),
        log.regions(),
        p.line(),
        proc.memory
            .map_or("n/a".into(), |m| format!("{} MB", m / (1 << 20))),
        proc.cpu.map_or("n/a".into(), |c| format!(
            "{:.1} s ({:.2} cores on average)",
            c.as_secs_f64(),
            c.as_secs_f64() / uptime.as_secs_f64().max(1e-9)
        )),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<ServerConfig, String> {
        let mut argv = vec!["server"];
        argv.extend_from_slice(args);
        configure(&Args::try_parse_from(argv).map_err(|e| e.to_string())?)
    }

    #[test]
    fn online_by_default() {
        let c = parse(&[]).expect("valid");
        assert!(!c.policy.allow_offline);
        assert_eq!(c.listen, "0.0.0.0:47777".parse().expect("addr"));
        let player = auth::Identity::offline("tester").expect("valid");
        assert!(
            c.policy.admit(&player).is_err(),
            "offline players refused without the flag"
        );
    }

    #[test]
    fn offline_binds_localhost_by_default() {
        let c = parse(&["--offline"]).expect("valid");
        assert!(c.policy.allow_offline);
        assert_eq!(c.listen, "127.0.0.1:47777".parse().expect("addr"));
    }

    #[test]
    fn offline_allows_lan_but_not_public_addresses() {
        for ok in [
            "192.168.1.10",
            "10.0.0.5",
            "172.16.3.4",
            "::1",
            "fd00::1",
            "fe80::1",
        ] {
            assert!(parse(&["--offline", "--bind", ok]).is_ok(), "{ok}");
        }
        for bad in ["0.0.0.0", "8.8.8.8", "::", "2001:db8::1"] {
            assert!(parse(&["--offline", "--bind", bad]).is_err(), "{bad}");
        }
        assert!(
            parse(&["--bind", "8.8.8.8"]).is_ok(),
            "online may bind anywhere"
        );
    }

    #[test]
    fn view_distance_keeps_render_distance_at_least_8() {
        assert!(parse(&["--view-distance", "9"]).is_err());
        assert_eq!(
            parse(&["--view-distance", "10"])
                .expect("valid")
                .view_distance,
            10
        );
    }
}
