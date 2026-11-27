//! T2-05 smoke test: 50 headless bots (walking, flying, building) join an
//! in-process server, play for a while and leave without errors; the
//! harness produces its summary.

use std::time::Duration;

use auth::AdmissionPolicy;
use client::bots::{BotConfig, run};
use server::metrics::Percentiles;
use server::{Server, ServerConfig};

#[test]
fn fifty_bots_play_without_errors() {
    let data = world::modset::source_build(world::modset::DEFAULT_BUILD).expect("build");
    let dir = std::env::temp_dir().join(format!("client-bots-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut sc = ServerConfig::new(
        "127.0.0.1:0".parse().expect("addr"),
        AdmissionPolicy {
            allow_offline: true,
        },
        dir.clone(),
        data.clone(),
    );
    sc.seed = Some(7);
    sc.view_distance = 10;
    sc.threads = 2;
    let server = Server::start(sc).expect("server");
    let log = server.tick_log();

    let mut config = BotConfig::new(server.local_addr(), 50, data);
    config.join_rate = 25.0;
    config.warmup = Duration::from_secs(5);
    config.measure = Duration::from_secs(10);
    let mut from = 0;
    let report = run(&config, |_, phase| {
        if phase == "measuring" && from == 0 {
            from = log.count();
        }
    });
    let summary = report.summary();
    print!("{summary}");
    let ticks: Vec<_> = log.since(from).into_iter().map(|t| t.0).collect();
    println!(
        "server tick while measuring: {}",
        Percentiles::of(&ticks).line()
    );

    assert!(report.join_errors.is_empty(), "{:?}", report.join_errors);
    assert_eq!(report.bots.len(), 50);
    for b in &report.bots {
        assert!(b.error.is_none(), "{}: {:?}", b.name, b.error);
        assert!(
            b.rx_kbit > 0.0 && b.tx_kbit > 0.0,
            "{} exchanged data",
            b.name
        );
    }
    assert!(
        report
            .bots
            .iter()
            .any(|b| b.behavior == "build" && b.edits > 0)
    );
    assert!(ticks.len() > 100, "the server ticked while measuring");
    assert!(summary.contains("received per bot") && report.csv().lines().count() == 51);
    server.shutdown();
    let _ = std::fs::remove_dir_all(dir);
}
