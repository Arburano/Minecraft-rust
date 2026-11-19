//! T0-11 QUIC transport spike example.
//!
//! The server sends 625 "columns" of 8 KB each (random bytes), ordered by
//! distance on prioritised unidirectional streams, plus 20 Hz entity deltas
//! as datagrams. The client measures time to the first 3×3 columns and
//! delta latency. Loss and RTT are emulated by an in-process UDP relay.
//!
//! Run:
//! ```text
//! cargo run -p net --example quic_spike -- --loss 0.02 --rtt-ms 100 --secs 10
//! cargo run -p net --example quic_spike -- --loss 0 --rtt-ms 0 --secs 60
//! ```
//! `--cc bbr|cubic` selects the server congestion controller (default bbr,
//! see `docs/research/net-quic-spike-2026-11-19.md`), `--seed` seeds the
//! loss pattern.

use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use net::lossy::LossyRelay;
use net::spike::{ClientParams, ServerParams, run_client, serve_connection};
use quinn::rustls::pki_types::PrivateKeyDer;

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let loss: f64 = arg_value(&args, "--loss")
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(0.02);
    let rtt_ms: u64 = arg_value(&args, "--rtt-ms")
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(100);
    let secs: u64 = arg_value(&args, "--secs")
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(10);
    let seed: u64 = arg_value(&args, "--seed")
        .map(|v| v.parse())
        .transpose()?
        .unwrap_or(42);
    let cc = arg_value(&args, "--cc").unwrap_or_else(|| "bbr".to_string());
    if !(0.0..=1.0).contains(&loss) {
        bail!("--loss must be within [0, 1]");
    }
    if cc != "bbr" && cc != "cubic" {
        bail!("--cc must be bbr or cubic");
    }

    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let cert = certified.cert.der().clone();
    let key = PrivateKeyDer::Pkcs8(certified.signing_key.serialize_der().into());

    let mut transport = quinn::TransportConfig::default();
    if cc == "cubic" {
        transport.congestion_controller_factory(std::sync::Arc::new(
            quinn::congestion::CubicConfig::default(),
        ));
    } else {
        transport.congestion_controller_factory(std::sync::Arc::new(
            quinn::congestion::BbrConfig::default(),
        ));
    }
    let server =
        net::server_endpoint_with_transport("127.0.0.1:0".parse()?, cert.clone(), key, transport)?;
    let server_addr = server.local_addr()?;
    let epoch = Instant::now();
    let server_params = ServerParams {
        epoch,
        ..ServerParams::default()
    };

    let server_task = tokio::spawn(async move {
        let incoming = server.accept().await.context("server: no incoming")?;
        let conn = incoming.await?;
        let stats = serve_connection(conn, &server_params).await?;
        Ok::<_, anyhow::Error>(stats)
    });

    // Loss/delay emulation at the transport level (alternative to tc netem).
    let relay = if loss > 0.0 || rtt_ms > 0 {
        Some(LossyRelay::spawn(server_addr, Duration::from_millis(rtt_ms / 2), loss, seed).await?)
    } else {
        None
    };
    let target = relay.as_ref().map_or(server_addr, |r| r.local_addr());

    let client = net::client_endpoint(cert)?;
    let client_params = ClientParams {
        observe: Duration::from_secs(secs),
        epoch,
        ..ClientParams::default()
    };
    println!(
        "loss {:.1}%, RTT {rtt_ms} ms, cc {cc}, seed {seed}, observation {secs} s",
        loss * 100.0
    );
    let report = run_client(&client, target, &client_params).await?;
    let stats = server_task.await??;
    if let Some(relay) = relay {
        relay.abort();
    }

    println!("--- client ---");
    println!("total client time:      {:?}", report.elapsed);
    println!("time to first column:   {:?}", report.time_to_first_column);
    println!("time to first 3x3:      {:?}", report.time_to_first_3x3);
    println!(
        "columns received:       {} / 625 ({} KiB payload)",
        report.columns.len(),
        report.column_bytes_received / 1024
    );
    println!(
        "deltas received:        {} (lost {})",
        report.deltas.len(),
        report.deltas_lost
    );
    for p in [50.0, 95.0, 100.0] {
        println!(
            "delta latency p{p:<5.0}     {:?} (while streaming: {:?})",
            report.delta_latency_percentile(p, false),
            report.delta_latency_percentile(p, true),
        );
    }
    println!("--- server ---");
    println!("columns sent (acked):   {}", stats.columns_sent);
    println!("deltas sent:            {}", stats.deltas_sent);
    println!(
        "intents: {} datagrams, {} unique ticks (client sent {})",
        stats.intents_received, stats.intent_ticks_unique, report.intents_sent
    );
    Ok(())
}
