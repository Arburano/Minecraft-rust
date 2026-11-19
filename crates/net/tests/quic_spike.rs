//! T0-11 integration test: QUIC spike under emulated loss and delay.
//!
//! Acceptance (tasks/stage-0/T0-11-net-quic-spike.md): at 2% loss and
//! RTT 100 ms the nearest 3×3 columns arrive ≤ 1 s, and deltas are not
//! blocked by column streaming. Loss is emulated at the transport level by
//! `net::lossy::LossyRelay` (the task's alternative to `tc netem`).

use std::time::{Duration, Instant};

use net::lossy::LossyRelay;
use net::spike::{
    ClientParams, ClientReport, ServerParams, ServerStats, run_client, serve_connection,
};
use quinn::rustls::pki_types::PrivateKeyDer;

/// Run one server + one client, optionally through a lossy relay.
/// Scenarios are serialized: parallel full-rate transfers on the same
/// runtime distort the timing measurements.
async fn run_scenario(
    loss: f64,
    rtt: Duration,
    seed: u64,
    observe: Duration,
) -> anyhow::Result<(ClientReport, ServerStats)> {
    static SCENARIO_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = SCENARIO_LOCK.lock().await;
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let cert = certified.cert.der().clone();
    let key = PrivateKeyDer::Pkcs8(certified.signing_key.serialize_der().into());

    let server = net::server_endpoint("127.0.0.1:0".parse()?, cert.clone(), key)?;
    let server_addr = server.local_addr()?;
    let epoch = Instant::now();
    let server_params = ServerParams {
        epoch,
        ..ServerParams::default()
    };
    let server_task = tokio::spawn(async move {
        let incoming = server.accept().await.expect("server: incoming");
        let conn = incoming.await?;
        serve_connection(conn, &server_params).await
    });

    let relay = if loss > 0.0 || !rtt.is_zero() {
        Some(LossyRelay::spawn(server_addr, rtt / 2, loss, seed).await?)
    } else {
        None
    };
    let target = relay.as_ref().map_or(server_addr, |r| r.local_addr());

    let client = net::client_endpoint(cert)?;
    let client_params = ClientParams {
        observe,
        epoch,
        ..ClientParams::default()
    };
    let report = run_client(&client, target, &client_params).await?;
    let stats = server_task.await??;
    if let Some(relay) = relay {
        relay.abort();
    }
    Ok((report, stats))
}

/// Acceptance criterion: 2% loss, RTT 100 ms → nearest 3×3 columns ≤ 1 s,
/// deltas keep flowing at ~20 Hz while columns stream.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn nearest_columns_under_loss_and_deltas_not_blocked() -> anyhow::Result<()> {
    let observe = Duration::from_secs(5);
    let (report, stats) = run_scenario(0.02, Duration::from_millis(100), 42, observe).await?;

    // Nearest 3×3 within 1 s of connect start (handshake included).
    let first_3x3 = report
        .time_to_first_3x3
        .expect("nearest 3x3 columns must arrive within the window");
    assert!(
        first_3x3 <= Duration::from_secs(1),
        "nearest 3x3 took {first_3x3:?}, budget is 1 s"
    );

    // Column streaming continued past the first 3×3 (625 × 8 KB cannot all
    // arrive in 5 s at 2% loss / 100 ms RTT, and need not).
    assert!(
        report.columns.len() > 9,
        "only {} columns arrived",
        report.columns.len()
    );

    // Deltas were not blocked by column streaming: ~20 Hz × 5 s × 98%
    // delivery ≈ 98; require ≥ 60 with margin for scheduler noise.
    let streaming_deltas = report.deltas.iter().filter(|d| d.during_streaming).count();
    assert!(
        streaming_deltas >= 60,
        "only {streaming_deltas} deltas during streaming"
    );
    // Every 1-second bucket of the window carries deltas (no stall).
    for bucket in 0..observe.as_secs() {
        let from = Duration::from_secs(bucket);
        let to = Duration::from_secs(bucket + 1);
        let count = report
            .deltas
            .iter()
            .filter(|d| d.arrived_at >= from && d.arrived_at < to)
            .count();
        assert!(count >= 5, "second {bucket}: only {count} deltas");
    }
    // Latency stays near the 50 ms one-way path delay, not seconds of
    // queueing behind 5 MB of columns.
    let p95 = report
        .delta_latency_percentile(95.0, true)
        .expect("streaming delta samples");
    assert!(p95 <= Duration::from_millis(250), "delta p95 {p95:?}");

    // Duplicated intents survive loss: ≥ 90% of ticks reach the server.
    assert!(report.intents_sent > 0);
    assert!(
        stats.intent_ticks_unique * 10 >= report.intents_sent * 9,
        "intents: {} unique of {} sent",
        stats.intent_ticks_unique,
        report.intents_sent
    );
    Ok(())
}

/// Without loss every column arrives intact (payloads verified by the
/// client) and no delta is lost.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn lossless_full_transfer() -> anyhow::Result<()> {
    let (report, stats) = run_scenario(0.0, Duration::ZERO, 42, Duration::from_secs(30)).await?;
    assert_eq!(report.columns.len(), 625);
    assert_eq!(report.column_bytes_received, 625 * 8 * 1024);
    assert_eq!(stats.columns_sent, 625);
    assert_eq!(report.deltas_lost, 0);
    assert!(report.time_to_first_3x3.is_some());
    // Distance order: with 16 streams in flight, completion order within
    // the first flight is arbitrary, but the nearest 3×3 must complete
    // long before far columns: all 9 are among the first 20 arrivals.
    let first20 = &report.columns[..20];
    for x in -1..=1 {
        for z in -1..=1 {
            assert!(first20.contains(&(x, z)), "3x3 member ({x},{z}) late");
        }
    }
    Ok(())
}

/// The server rejects an incompatible handshake version with a clean error,
/// not a panic.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_rejects_wrong_version() -> anyhow::Result<()> {
    let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])?;
    let cert = certified.cert.der().clone();
    let key = PrivateKeyDer::Pkcs8(certified.signing_key.serialize_der().into());
    let server = net::server_endpoint("127.0.0.1:0".parse()?, cert.clone(), key)?;
    let server_addr = server.local_addr()?;
    let server_task = tokio::spawn(async move {
        let incoming = server.accept().await.expect("server: incoming");
        let conn = incoming.await?;
        serve_connection(conn, &ServerParams::default()).await
    });

    let client = net::client_endpoint(cert)?;
    let conn = client.connect(server_addr, "localhost")?.await?;
    let (mut send, _recv) = conn.open_bi().await?;
    send.write_all(&999u32.to_le_bytes()).await?;
    send.finish()?;
    // Garbage datagrams must not upset the server either.
    let _ = conn.send_datagram(bytes::Bytes::from_static(&[0xFF; 3]));
    let closed = conn.closed().await;
    assert!(matches!(
        closed,
        quinn::ConnectionError::ApplicationClosed(_)
    ));

    let err = server_task.await?.expect_err("version mismatch expected");
    assert!(matches!(
        err,
        net::spike::SpikeError::VersionMismatch { peer: 999, .. }
    ));
    Ok(())
}
