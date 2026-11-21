//! Dedicated server binary: region-threaded simulation, networking, persistence.
//!
//! Spec:
//! - `docs/architecture/server-regions.md`
//! - `docs/architecture/auth-and-assets.md` ("Offline mode"), `docs/legal.md` rule 7
//!
//! Status: command line and offline-mode admission policy (T1-08). Simulation
//! and networking are wired in stage 2.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::ExitCode;

use auth::AdmissionPolicy;
use clap::Parser;

/// Default game port.
const DEFAULT_PORT: u16 = 47_777;

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
}

/// The validated server configuration.
#[derive(Debug, PartialEq, Eq)]
struct ServerConfig {
    listen: SocketAddr,
    policy: AdmissionPolicy,
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
    Ok(ServerConfig {
        listen: SocketAddr::new(ip, args.port),
        policy: AdmissionPolicy {
            allow_offline: args.offline,
        },
    })
}

fn main() -> ExitCode {
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
    println!(
        "server: configured for {}; simulation and networking arrive in stage 2",
        config.listen
    );
    ExitCode::SUCCESS
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
}
