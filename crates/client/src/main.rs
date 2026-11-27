//! Game client binary: window, in-game launcher menu, renderer, embedded server for singleplayer.
//!
//! Spec:
//! - `docs/architecture/client-menu-and-updater.md`
//! - `docs/architecture/auth-and-assets.md` ("Offline mode")
//!
//! Status (T1-13): window, main menu (the in-game launcher, ADR-0010),
//! single-player worlds with saving, settings, About screen. T2-02:
//! `--connect ADDR` joins an offline server (the menu entry is T2-08).

mod app;
mod bench;
mod menu;
mod view;

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use auth::Identity;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "Game client")]
struct Args {
    /// Play offline under this name (no Microsoft login; development and
    /// private tests only, docs/legal.md rule 7).
    #[arg(long, value_name = "NAME")]
    offline_name: Option<String>,
    /// Settings, worlds and caches (default: the per-user data folder).
    #[arg(long, value_name = "DIR")]
    data_dir: Option<PathBuf>,
    /// The `core` mod's data folder (default: next to the executable, else the source tree).
    #[arg(long, value_name = "DIR")]
    mods_dir: Option<PathBuf>,
    /// Print the time from process start to the first menu frame, then exit.
    #[arg(long)]
    measure_startup: bool,
    /// Open this world (created if missing) right away.
    #[arg(long, value_name = "WORLD")]
    autoplay: Option<String>,
    /// Save a BMP screenshot of frame N (`file.bmp:N`; in a world, once it is playable), then exit.
    #[arg(long, value_name = "FILE:N")]
    screenshot: Option<String>,
    /// Start without resource and shader packs (if a pack broke the game).
    #[arg(long)]
    safe_mode: bool,
    /// Stage 1 exit measurement (T1-15): fly the scripted route for SECONDS
    /// after a warm-up, write frame times and memory to <data>/bench, exit.
    #[arg(long, value_name = "SECONDS")]
    bench_route: Option<u32>,
    /// Prefer the GPU whose name contains this text (e.g. `vega`, `1660`).
    #[arg(long, value_name = "NAME")]
    gpu: Option<String>,
    /// Join this server right away, e.g. `127.0.0.1:47777` (needs
    /// `--offline-name`; offline servers only until T0-10).
    #[arg(long, value_name = "ADDR")]
    connect: Option<std::net::SocketAddr>,
}

fn mods_dir(arg: Option<PathBuf>) -> PathBuf {
    arg.or_else(|| {
        let exe = std::env::current_exe().ok()?;
        let p = exe.parent()?.join("mods/core/data");
        p.is_dir().then_some(p)
    })
    .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data"))
}

/// The identity to play with. Until Microsoft login exists (T0-10), offline
/// mode is the only way to play and must be chosen explicitly.
fn identity(args: &Args) -> Result<Identity, String> {
    match &args.offline_name {
        Some(name) => Identity::offline(name).map_err(|e| e.to_string()),
        None => Err(
            "Microsoft login is not available yet (waiting for API approval, T0-10); \
             start with --offline-name <NAME> for development"
                .into(),
        ),
    }
}

fn main() -> ExitCode {
    let start = Instant::now();
    let _profiler = engine_core::profile::start();
    let args = Args::parse();
    // A name on the command line must be valid; otherwise the menu asks.
    if args.offline_name.is_some()
        && let Err(e) = identity(&args)
    {
        eprintln!("error: {e}");
        return ExitCode::FAILURE;
    }
    let screenshot = args
        .screenshot
        .as_deref()
        .map(|s| match s.rsplit_once(':') {
            Some((p, n)) if n.parse::<u32>().is_ok() => (PathBuf::from(p), n.parse().unwrap_or(1)),
            _ => (PathBuf::from(s), 1),
        });
    let opts = app::Options {
        data_dir: args
            .data_dir
            .clone()
            .unwrap_or_else(client::settings::default_data_dir),
        mods_dir: mods_dir(args.mods_dir.clone()),
        offline_name: args.offline_name.clone(),
        measure_startup: args.measure_startup,
        start,
        autoplay: args.autoplay.clone(),
        screenshot,
        safe_mode: args.safe_mode,
        bench: args.bench_route,
        gpu: args.gpu.clone(),
        connect: args.connect,
    };
    let event_loop = match winit::event_loop::EventLoop::new() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("error: no display: {e}");
            return ExitCode::FAILURE;
        }
    };
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);
    let mut app = app::App::new(opts);
    match event_loop.run_app(&mut app) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offline_is_opt_in() {
        let none = Args::try_parse_from(["client"]).expect("valid args");
        assert!(identity(&none).is_err());
        let named = Args::try_parse_from(["client", "--offline-name", "dev_01"]).expect("valid");
        let id = identity(&named).expect("valid name");
        assert!(id.is_offline());
        assert_eq!(id.id(), Identity::offline("DEV_01").expect("valid").id());
        let bad = Args::try_parse_from(["client", "--offline-name", "x"]).expect("valid args");
        assert!(identity(&bad).is_err());
    }
}
