//! Game client binary: window, in-game launcher menu, renderer, embedded server for singleplayer.
//!
//! Spec:
//! - `docs/architecture/client-menu-and-updater.md`
//! - `docs/architecture/auth-and-assets.md` ("Offline mode")
//!
//! Status (T1-13): window, main menu (the in-game launcher, ADR-0010),
//! single-player worlds with saving, settings, About screen.

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
    /// The build to play: an id from `builds/` or a build file (ADR-0017).
    #[arg(long, value_name = "ID|FILE", default_value = world::modset::DEFAULT_BUILD)]
    build: String,
    /// The folder of mod folders (default: `mods/` next to the executable, else the source tree).
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
    /// Stage 1 exit measurement (T1-15, T1-25): run the scripted route for
    /// SECONDS after a warm-up, write frame times, spikes with their causes
    /// and memory to <data>/bench, exit.
    #[arg(long, value_name = "SECONDS")]
    bench_route: Option<u32>,
    /// The bench route (T1-25): `fly` (16 chunks, the stress case) or `walk`
    /// (sprinting on the ground at 12 chunks, the exit criterion literally).
    #[arg(long, value_enum, default_value_t = bench::Route::Fly)]
    bench_mode: bench::Route,
    /// Prefer the GPU whose name contains this text (e.g. `vega`, `1660`).
    #[arg(long, value_name = "NAME")]
    gpu: Option<String>,
    /// Join this server right away (`host:port`, e.g. `127.0.0.1:47777`);
    /// needs `--offline-name` (the server must run with `--offline`).
    #[arg(long, value_name = "ADDR")]
    connect: Option<std::net::SocketAddr>,
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
    if args.connect.is_some() && args.offline_name.is_none() {
        eprintln!(
            "error: --connect needs --offline-name <NAME> until Microsoft login exists (T0-10)"
        );
        return ExitCode::FAILURE;
    }
    let screenshot = args
        .screenshot
        .as_deref()
        .map(|s| match s.rsplit_once(':') {
            Some((p, n)) if n.parse::<u32>().is_ok() => (PathBuf::from(p), n.parse().unwrap_or(1)),
            _ => (PathBuf::from(s), 1),
        });
    let located = world::modset::locate(&args.build, args.mods_dir.as_deref()).and_then(|m| {
        // ADR-0018: a build needing a capability this binary lacks fails now.
        m.check_compiled(client::capabilities(), "client")?;
        Ok(m)
    });
    let mods = match located {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: build {}: {e}", args.build);
            return ExitCode::FAILURE;
        }
    };
    let opts = app::Options {
        data_dir: args
            .data_dir
            .clone()
            .unwrap_or_else(client::settings::default_data_dir),
        mods,
        offline_name: args.offline_name.clone(),
        measure_startup: args.measure_startup,
        start,
        autoplay: args.autoplay.clone(),
        screenshot,
        safe_mode: args.safe_mode,
        bench: args.bench_route,
        bench_route: args.bench_mode,
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
