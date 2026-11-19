//! Spike client: measures time to the nearest 3×3 columns and entity-delta
//! latency while columns stream in, and sends duplicated intents.

use std::collections::HashSet;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use bytes::Bytes;
use quinn::{Connection, Endpoint};
use tokio::task::JoinSet;

use super::SpikeError;
use super::wire::{self, COLUMN_BYTES, COLUMN_HEADER_LEN, GRID_SIDE, Intent, PROTOCOL_VERSION};

/// Parameters of the spike client.
#[derive(Debug, Clone)]
pub struct ClientParams {
    /// Grid side the server sends (for the expected column count).
    pub grid_side: i32,
    /// Expected payload bytes per column; received bytes are verified
    /// against the deterministic payload.
    pub column_bytes: usize,
    /// Observation window: the client closes the connection after this
    /// duration even if columns are still streaming.
    pub observe: Duration,
    /// Interval between intents (50 ms → 20 Hz).
    pub intent_interval: Duration,
    /// How many copies of each intent to send (spec: 2–3).
    pub intent_duplicates: u8,
    /// Shared time epoch for delta latency measurement.
    pub epoch: Instant,
}

impl Default for ClientParams {
    fn default() -> Self {
        Self {
            grid_side: GRID_SIDE,
            column_bytes: COLUMN_BYTES,
            observe: Duration::from_secs(10),
            intent_interval: Duration::from_millis(50),
            intent_duplicates: 2,
            epoch: Instant::now(),
        }
    }
}

/// One received delta sample.
#[derive(Debug, Clone, Copy)]
pub struct DeltaSample {
    /// When it arrived, relative to the start of `run_client`.
    pub arrived_at: Duration,
    /// One-way latency from the server's send timestamp.
    pub latency: Duration,
    /// Whether columns were still streaming when it arrived.
    pub during_streaming: bool,
}

/// Measurements of one spike run.
#[derive(Debug, Clone, Default)]
pub struct ClientReport {
    /// Time from connect start until all nearest 3×3 columns arrived.
    pub time_to_first_3x3: Option<Duration>,
    /// Time until the first complete column arrived.
    pub time_to_first_column: Option<Duration>,
    /// Columns received completely, in arrival order.
    pub columns: Vec<(i32, i32)>,
    /// Total column payload bytes received.
    pub column_bytes_received: usize,
    /// Delta samples in arrival order.
    pub deltas: Vec<DeltaSample>,
    /// Delta sequence numbers lost (gaps below the maximum seen).
    pub deltas_lost: u64,
    /// Distinct intent ticks sent.
    pub intents_sent: u64,
    /// Total time the client ran.
    pub elapsed: Duration,
}

impl ClientReport {
    /// Percentile over delta latencies (nearest-rank), optionally only
    /// samples taken while columns were streaming.
    pub fn delta_latency_percentile(&self, p: f64, streaming_only: bool) -> Option<Duration> {
        let mut latencies: Vec<Duration> = self
            .deltas
            .iter()
            .filter(|d| !streaming_only || d.during_streaming)
            .map(|d| d.latency)
            .collect();
        percentile(&mut latencies, p)
    }
}

/// Nearest-rank percentile; sorts `values` in place.
pub fn percentile(values: &mut [Duration], p: f64) -> Option<Duration> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let n = values.len();
    let rank = ((p / 100.0) * n as f64).ceil() as usize;
    Some(values[rank.clamp(1, n) - 1])
}

/// Run the spike client against `addr` (the server, or a lossy relay).
///
/// Measures from just before `connect`, so the handshake RTTs count toward
/// the time to the first 3×3 columns.
pub async fn run_client(
    endpoint: &Endpoint,
    addr: SocketAddr,
    params: &ClientParams,
) -> Result<ClientReport, SpikeError> {
    let start = Instant::now();
    let conn = endpoint.connect(addr, "localhost")?.await?;

    // Control channel: minimal version handshake.
    let (mut ctrl_send, mut ctrl_recv) = conn.open_bi().await?;
    ctrl_send.write_all(&PROTOCOL_VERSION.to_le_bytes()).await?;
    ctrl_send.finish()?;
    let mut welcome = [0u8; 4];
    ctrl_recv.read_exact(&mut welcome).await?;
    let peer_version = u32::from_le_bytes(welcome);
    if peer_version != PROTOCOL_VERSION {
        conn.close(1u32.into(), b"unsupported protocol version");
        return Err(SpikeError::VersionMismatch {
            peer: peer_version,
            ours: PROTOCOL_VERSION,
        });
    }

    let expected_columns = (params.grid_side * params.grid_side) as usize;
    let intent_ticks = Arc::new(AtomicU64::new(0));
    let intents = tokio::spawn(send_intents(
        conn.clone(),
        params.intent_interval,
        params.intent_duplicates,
        Arc::clone(&intent_ticks),
    ));

    let mut report = ClientReport::default();
    let mut near_seen: HashSet<(i32, i32)> = HashSet::new();
    let mut max_seq = 0u64;
    let mut reads: JoinSet<Result<(i32, i32, usize), SpikeError>> = JoinSet::new();
    let deadline = start + params.observe;

    loop {
        tokio::select! {
            () = tokio::time::sleep_until(deadline.into()) => break,
            incoming = conn.accept_uni() => {
                let mut recv = incoming?;
                let column_bytes = params.column_bytes;
                reads.spawn(async move {
                    let data = recv.read_to_end(column_bytes + COLUMN_HEADER_LEN + 1).await?;
                    let header = wire::ColumnHeader::decode(&data)
                        .ok_or(SpikeError::MalformedColumn)?;
                    if data.len() != COLUMN_HEADER_LEN + header.len as usize {
                        return Err(SpikeError::MalformedColumn);
                    }
                    // Verify the deterministic payload end to end.
                    let mut expected = vec![0u8; header.len as usize];
                    wire::fill_payload(&mut expected, wire::column_seed(header.x, header.z));
                    if data[COLUMN_HEADER_LEN..] != expected[..] {
                        return Err(SpikeError::MalformedColumn);
                    }
                    Ok((header.x, header.z, header.len as usize))
                });
            }
            datagram = conn.read_datagram() => {
                let datagram = datagram?;
                if let Some(header) = wire::DeltaHeader::decode(&datagram) {
                    let sent = Duration::from_micros(header.sent_us);
                    let now = params.epoch.elapsed();
                    report.deltas.push(DeltaSample {
                        arrived_at: start.elapsed(),
                        latency: now.saturating_sub(sent),
                        during_streaming: report.columns.len() < expected_columns,
                    });
                    max_seq = max_seq.max(header.seq);
                }
            }
            joined = reads.join_next(), if !reads.is_empty() => {
                let (x, z, len) = match joined {
                    Some(res) => res.map_err(SpikeError::TaskPanic)??,
                    None => break, // guarded by is_empty; cannot happen
                };
                if report.columns.is_empty() {
                    report.time_to_first_column = Some(start.elapsed());
                }
                report.columns.push((x, z));
                report.column_bytes_received += len;
                if x.abs() <= 1 && z.abs() <= 1 {
                    near_seen.insert((x, z));
                    if near_seen.len() == 9 && report.time_to_first_3x3.is_none() {
                        report.time_to_first_3x3 = Some(start.elapsed());
                    }
                }
                if report.columns.len() == expected_columns {
                    break;
                }
            }
        }
    }

    report.deltas_lost = max_seq.saturating_sub(report.deltas.len() as u64);
    report.elapsed = start.elapsed();
    conn.close(0u32.into(), b"spike observation done");
    intents.abort();
    report.intents_sent = intent_ticks.load(Ordering::Relaxed);
    endpoint.wait_idle().await;
    Ok(report)
}

async fn send_intents(conn: Connection, interval: Duration, duplicates: u8, ticks: Arc<AtomicU64>) {
    let mut ticker = tokio::time::interval(interval);
    let mut tick = 0u64;
    loop {
        ticker.tick().await;
        tick += 1;
        let payload = Bytes::copy_from_slice(&Intent { tick }.encode());
        for _ in 0..duplicates.max(1) {
            if conn.send_datagram(payload.clone()).is_err() {
                return;
            }
        }
        ticks.store(tick, Ordering::Relaxed);
    }
}
