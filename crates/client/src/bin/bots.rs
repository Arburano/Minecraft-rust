//! Headless bot clients for load runs (T2-05).
//!
//! ```text
//! # against a running server (start it with `server --offline --stats-out server.txt`):
//! cargo run -p client --release --bin bots -- --connect 127.0.0.1:47777 --count 200
//! # with a server in the same process (smoke runs; bots and server share the CPU):
//! cargo run -p client --release --bin bots -- --embedded /tmp/bot-world --count 50
//! ```
//!
//! Writes `bots-summary-<time>.txt` and `bots-<time>.csv` to `--out`.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use client::bots::{BotConfig, run};

#[derive(Parser, Debug)]
#[command(about = "Headless bot clients for server load runs")]
struct Args {
    /// The server (`--offline` mode) to join.
    #[arg(long, value_name = "ADDR", conflicts_with = "embedded")]
    connect: Option<SocketAddr>,
    /// Start a server in this process with its world in DIR.
    #[arg(long, value_name = "DIR")]
    embedded: Option<PathBuf>,
    /// Number of bots.
    #[arg(long, default_value_t = 50)]
    count: usize,
    /// Weights of walking, flying and building bots, e.g. `6,2,2`.
    #[arg(long, default_value = "6,2,2")]
    mix: String,
    /// Bots joined per second.
    #[arg(long, default_value_t = 20.0)]
    join_rate: f64,
    /// Seconds after the last join before measuring.
    #[arg(long, default_value_t = 20)]
    warmup: u64,
    /// Measured seconds.
    #[arg(long, default_value_t = 60)]
    seconds: u64,
    /// Bots turn back beyond this distance from where they joined (blocks).
    #[arg(long, default_value_t = 48.0)]
    roam: f64,
    /// The build to run (an id from `builds/` or a build file, ADR-0017).
    #[arg(long, value_name = "ID|FILE", default_value = world::modset::DEFAULT_BUILD)]
    build: String,
    /// The folder of mod folders (default: `mods/` next to the executable, else the source tree).
    #[arg(long, value_name = "DIR")]
    mods_dir: Option<PathBuf>,
    /// Groups of bots with homes `--group-spacing` blocks apart (e.g. 10
    /// groups of 20 for "≤ 20 moving players nearby"); they walk or fly
    /// there during the warm-up.
    #[arg(long, default_value_t = 1)]
    groups: usize,
    /// Distance between group homes in blocks (more than the view radius).
    #[arg(long, default_value_t = 400.0)]
    group_spacing: f64,
    /// Where to write the summary and the CSV.
    #[arg(long, value_name = "DIR", default_value = ".")]
    out: PathBuf,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let data = match world::modset::locate(&args.build, args.mods_dir.as_deref()) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: build {}: {e}", args.build);
            return ExitCode::FAILURE;
        }
    };
    let mix: Vec<u32> = args
        .mix
        .split(',')
        .filter_map(|v| v.trim().parse().ok())
        .collect();
    let [walk, fly, build] = mix[..] else {
        eprintln!("error: --mix needs three numbers, e.g. 6,2,2");
        return ExitCode::FAILURE;
    };
    let server = match &args.embedded {
        Some(dir) => {
            let config = server::ServerConfig::new(
                "127.0.0.1:0".parse().expect("valid address"),
                auth::AdmissionPolicy {
                    allow_offline: true,
                },
                dir.clone(),
                data.clone(),
            );
            match server::Server::start(config) {
                Ok(s) => Some(s),
                Err(e) => {
                    eprintln!("error: embedded server: {e}");
                    return ExitCode::FAILURE;
                }
            }
        }
        None => None,
    };
    let addr = match (&server, args.connect) {
        (Some(s), _) => s.local_addr(),
        (None, Some(a)) => a,
        (None, None) => {
            eprintln!("error: give --connect ADDR or --embedded DIR");
            return ExitCode::FAILURE;
        }
    };
    let mut config = BotConfig::new(addr, args.count, data);
    config.mix = [walk, fly, build];
    config.join_rate = args.join_rate;
    config.warmup = Duration::from_secs(args.warmup);
    config.measure = Duration::from_secs(args.seconds);
    config.roam = args.roam;
    config.groups = args.groups;
    config.group_spacing = args.group_spacing;
    let log = server.as_ref().map(server::Server::tick_log);
    let mut ticks_from = 0;
    let mut regions = 0;
    let report = run(&config, |playing, phase| {
        if phase == "measuring" {
            if ticks_from == 0 {
                ticks_from = log.as_ref().map_or(0, |l| l.count());
            }
            regions = log.as_ref().map_or(0, |l| l.regions());
        }
        println!("bots: {playing} playing ({phase})");
    });
    let mut summary = report.summary();
    if let Some(log) = &log {
        let times: Vec<_> = log.since(ticks_from).into_iter().map(|t| t.0).collect();
        summary.push_str(&format!(
            "embedded server tick while measuring: {}\nembedded server regions with players: {}\n",
            server::metrics::Percentiles::of(&times).line(),
            regions
        ));
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let _ = std::fs::create_dir_all(&args.out);
    let summary_path = args.out.join(format!("bots-summary-{stamp}.txt"));
    let csv_path = args.out.join(format!("bots-{stamp}.csv"));
    if let Err(e) = std::fs::write(&summary_path, &summary)
        .and_then(|()| std::fs::write(&csv_path, report.csv()))
    {
        eprintln!("error: writing results: {e}");
    }
    print!("{summary}");
    println!(
        "written: {} and {}",
        summary_path.display(),
        csv_path.display()
    );
    if let Some(s) = server {
        s.shutdown();
    }
    if report.join_errors.is_empty() && report.bots.iter().all(|b| b.error.is_none()) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}
