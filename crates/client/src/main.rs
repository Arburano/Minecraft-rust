//! Game client binary: window, in-game launcher menu, renderer, embedded server for singleplayer.
//!
//! Spec:
//! - `docs/architecture/client-menu-and-updater.md`
//! - `docs/architecture/auth-and-assets.md` ("Offline mode")
//!
//! Status: command line with the offline identity (T1-08). Window, menu and
//! settings persistence arrive in T1-13.

use std::process::ExitCode;

use auth::Identity;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "Game client")]
struct Args {
    /// Play offline under this name (no Microsoft login; development and
    /// private tests only, docs/legal.md rule 7).
    #[arg(long, value_name = "NAME")]
    offline_name: Option<String>,
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
    let _profiler = engine_core::profile::start();
    let args = Args::parse();
    match identity(&args) {
        Ok(id) => {
            println!(
                "client: offline as {} ({}); window and menu arrive in T1-13",
                id.name(),
                id.id()
            );
            ExitCode::SUCCESS
        }
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
