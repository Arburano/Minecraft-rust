//! Joins a server without a window and reports the login metrics (T2-02):
//! connect + handshake time, time to control (the nearest columns lit and
//! meshed), and how fast the rest of the view streams in.
//!
//! ```text
//! cargo run --release -p server -- --offline --bind 192.168.1.10
//! cargo run --release -p client --example net_join -- 192.168.1.10:47777 [NAME] [RUNS]
//! ```

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use auth::Identity;
use client::session::{ConnectParams, Session, SessionConfig};
use sim::physics::PlayerInput;

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(addr) = args.next().and_then(|a| a.parse::<SocketAddr>().ok()) else {
        eprintln!("usage: net_join <server addr:port> [name] [runs]");
        std::process::exit(2);
    };
    let name = args.next().unwrap_or_else(|| "Measure".into());
    let runs: u32 = args.next().and_then(|r| r.parse().ok()).unwrap_or(3);
    let data = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data");
    let identity = match Identity::offline(&name) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };
    for run in 1..=runs {
        let mut config = SessionConfig::new(data.clone());
        config.threads = std::thread::available_parallelism().map_or(3, |n| n.get().max(2) - 1);
        let params = ConnectParams {
            addr,
            identity: identity.clone(),
            timeout: Duration::from_secs(10),
        };
        let mut s = match Session::connect(&params, config) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("run {run}: cannot join: {e}");
                std::process::exit(1);
            }
        };
        let start = Instant::now();
        let mut next_tick = start;
        let mut all_at = None;
        while start.elapsed() < Duration::from_secs(60) {
            if let Err(e) = s.update() {
                eprintln!("run {run}: {e}");
                std::process::exit(1);
            }
            if Instant::now() >= next_tick {
                s.fixed_tick(&PlayerInput::default());
                next_tick += Duration::from_millis(50);
            }
            let cols = s.columns_received().unwrap_or(0);
            let view = 2 * u64::try_from(s.loader().radius() + 2).unwrap_or(0) + 1;
            if all_at.is_none() && cols >= view * view {
                all_at = Some(start.elapsed());
            }
            if s.time_to_control().is_some() && all_at.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        let ms = |d: Option<Duration>| {
            d.map_or("—".to_string(), |d| {
                format!("{:.0} ms", d.as_secs_f64() * 1000.0)
            })
        };
        println!(
            "run {run}: connect + handshake {}, in control after {}, whole view ({} columns) after {} (from the handshake)",
            ms(s.connect_time()),
            ms(s.time_to_control()),
            s.columns_received().unwrap_or(0),
            ms(all_at),
        );
        if let Err(e) = s.close() {
            eprintln!("run {run}: leaving: {e}");
        }
        std::thread::sleep(Duration::from_millis(500));
    }
}
