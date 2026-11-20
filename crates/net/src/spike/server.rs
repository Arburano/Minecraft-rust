//! Spike server: one connection, 625 columns by distance on prioritised
//! unidirectional streams, 20 Hz entity deltas as datagrams, duplicated
//! intents received as datagrams.

use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use quinn::Connection;
use tokio::task::JoinSet;

use super::SpikeError;
use super::wire::{self, COLUMN_BYTES, DELTA_HEADER_LEN, GRID_SIDE, Intent, PROTOCOL_VERSION};

/// Entity-delta payload bytes: ~20 nearby entities × 6 bytes (the traffic
/// estimate in networking-protocol.md), random content in the spike.
pub const DELTA_PAYLOAD_BYTES: usize = 120;

/// Parameters of the spike server.
#[derive(Debug, Clone)]
pub struct ServerParams {
    /// Grid side; `side²` columns are sent (25 → 625).
    pub grid_side: i32,
    /// Payload bytes per column.
    pub column_bytes: usize,
    /// Interval between entity deltas (50 ms → 20 Hz).
    pub delta_interval: Duration,
    /// Maximum concurrent chunk streams per client
    /// (networking-protocol.md suggests 16).
    pub max_chunk_streams: usize,
    /// Shared time epoch for delta send timestamps.
    pub epoch: Instant,
}

impl Default for ServerParams {
    fn default() -> Self {
        Self {
            grid_side: GRID_SIDE,
            column_bytes: COLUMN_BYTES,
            delta_interval: Duration::from_millis(50),
            max_chunk_streams: 16,
            epoch: Instant::now(),
        }
    }
}

/// What the server did during one connection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ServerStats {
    /// Columns fully written and acknowledged by the client.
    pub columns_sent: u64,
    /// Entity-delta datagrams handed to QUIC.
    pub deltas_sent: u64,
    /// Intent datagrams received (including duplicates).
    pub intents_received: u64,
    /// Distinct intent tick numbers received.
    pub intent_ticks_unique: u64,
}

/// Serve one spike connection until the peer closes it.
///
/// The client closing the connection (or the connection timing out) is a
/// normal end: the stats collected so far are returned. Only unexpected
/// transport errors and version mismatches produce `Err`.
pub async fn serve_connection(
    conn: Connection,
    params: &ServerParams,
) -> Result<ServerStats, SpikeError> {
    // Control channel: minimal version handshake.
    let (mut ctrl_send, mut ctrl_recv) = conn.accept_bi().await?;
    let mut hello = [0u8; 4];
    ctrl_recv.read_exact(&mut hello).await?;
    let peer_version = u32::from_le_bytes(hello);
    if peer_version != PROTOCOL_VERSION {
        conn.close(1u32.into(), b"unsupported protocol version");
        return Err(SpikeError::VersionMismatch {
            peer: peer_version,
            ours: PROTOCOL_VERSION,
        });
    }
    ctrl_send.write_all(&PROTOCOL_VERSION.to_le_bytes()).await?;
    ctrl_send.finish()?;

    let stats = Arc::new(Counters::default());

    let intents = tokio::spawn(count_intents(conn.clone(), Arc::clone(&stats)));
    let deltas = tokio::spawn(send_deltas(
        conn.clone(),
        params.delta_interval,
        params.epoch,
        Arc::clone(&stats),
    ));

    let column_result = send_columns(&conn, params, &stats).await;

    if column_result.is_ok() {
        // All columns delivered; keep sending deltas until the client goes.
        conn.closed().await;
    }
    intents.abort();
    deltas.abort();

    let result = match column_result {
        Ok(()) => Ok(()),
        Err(e) if e.is_peer_closed() => Ok(()),
        Err(e) => Err(e),
    };
    result.map(|()| stats.snapshot())
}

#[derive(Default)]
struct Counters {
    columns_sent: AtomicU64,
    deltas_sent: AtomicU64,
    intents_received: AtomicU64,
    intent_ticks_unique: AtomicU64,
}

impl Counters {
    fn snapshot(&self) -> ServerStats {
        ServerStats {
            columns_sent: self.columns_sent.load(Ordering::Relaxed),
            deltas_sent: self.deltas_sent.load(Ordering::Relaxed),
            intents_received: self.intents_received.load(Ordering::Relaxed),
            intent_ticks_unique: self.intent_ticks_unique.load(Ordering::Relaxed),
        }
    }
}

async fn count_intents(conn: Connection, stats: Arc<Counters>) {
    let mut seen = HashSet::new();
    while let Ok(datagram) = conn.read_datagram().await {
        stats.intents_received.fetch_add(1, Ordering::Relaxed);
        if let Some(intent) = Intent::decode(&datagram) {
            if seen.insert(intent.tick) {
                stats.intent_ticks_unique.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
}

async fn send_deltas(conn: Connection, interval: Duration, epoch: Instant, stats: Arc<Counters>) {
    let mut ticker = tokio::time::interval(interval);
    let mut seq = 0u64;
    let mut buf = vec![0u8; DELTA_HEADER_LEN + DELTA_PAYLOAD_BYTES];
    loop {
        ticker.tick().await;
        seq += 1;
        let sent_us = epoch.elapsed().as_micros() as u64;
        let header = wire::DeltaHeader { seq, sent_us };
        buf[..DELTA_HEADER_LEN].copy_from_slice(&header.encode());
        wire::fill_payload(&mut buf[DELTA_HEADER_LEN..], seq);
        if conn.send_datagram(Bytes::copy_from_slice(&buf)).is_err() {
            return;
        }
        stats.deltas_sent.fetch_add(1, Ordering::Relaxed);
    }
}

async fn send_columns(
    conn: &Connection,
    params: &ServerParams,
    stats: &Arc<Counters>,
) -> Result<(), SpikeError> {
    let cells = wire::columns_by_distance(params.grid_side);
    let mut streams = JoinSet::new();
    for (rank, (x, z)) in cells.into_iter().enumerate() {
        // Keep at most `max_chunk_streams` streams in flight so as not to
        // clog the congestion control queue (networking-protocol.md).
        while streams.len() >= params.max_chunk_streams {
            match streams.join_next().await {
                Some(Ok(Ok(()))) => {}
                Some(Ok(Err(e))) => return Err(e),
                Some(Err(join_err)) if join_err.is_cancelled() => {}
                Some(Err(join_err)) => return Err(SpikeError::TaskPanic(join_err)),
                None => break,
            }
        }
        let conn = conn.clone();
        let stats = Arc::clone(stats);
        let column_bytes = params.column_bytes;
        streams.spawn(async move {
            let mut send = conn.open_uni().await?;
            // Nearer columns get higher priority (rank 0 = nearest).
            send.set_priority(-(rank as i32))?;
            let header = wire::ColumnHeader {
                x,
                z,
                len: column_bytes as u32,
            };
            send.write_all(&header.encode()).await?;
            let mut payload = vec![0u8; column_bytes];
            wire::fill_payload(&mut payload, wire::column_seed(x, z));
            send.write_all(&payload).await?;
            send.finish()?;
            // Wait for the client to acknowledge all bytes so the window
            // really limits in-flight data, not just buffered writes.
            send.stopped().await?;
            stats.columns_sent.fetch_add(1, Ordering::Relaxed);
            Ok::<(), SpikeError>(())
        });
    }
    while let Some(joined) = streams.join_next().await {
        match joined {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(e),
            Err(join_err) if join_err.is_cancelled() => {}
            Err(join_err) => return Err(SpikeError::TaskPanic(join_err)),
        }
    }
    Ok(())
}
