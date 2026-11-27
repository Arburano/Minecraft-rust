//! Headless bot clients and the load harness (T2-05).
//!
//! A bot is a real game client without a window: a [`Session`] in headless
//! mode (prediction, reconciliation, edits; no light, meshes or rendering;
//! only the columns around it are decoded). All bots share one network
//! runtime and are driven by one loop: `update` every few milliseconds, a
//! fixed tick every 50 ms. Behaviors are scripted: walking, flying, and
//! building (placing and digging blocks next to themselves). No chat.

use std::fmt::Write as _;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use glam::DVec3;
use sim::physics::PlayerInput;
use world::BlockStateId;
use world::coords::BlockPos;

use crate::session::{Session, SessionConfig};

/// What a bot does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Behavior {
    /// Walks, turns now and then, jumps when blocked.
    Walk,
    /// Flies around at a varying height.
    Fly,
    /// Walks slowly and keeps placing and digging a block next to itself.
    Build,
}

impl Behavior {
    fn name(self) -> &'static str {
        match self {
            Self::Walk => "walk",
            Self::Fly => "fly",
            Self::Build => "build",
        }
    }
}

/// Harness settings.
#[derive(Clone, Debug)]
pub struct BotConfig {
    /// The server.
    pub addr: SocketAddr,
    /// Bots to join.
    pub count: usize,
    /// Share of walkers, flyers and builders (relative weights).
    pub mix: [u32; 3],
    /// Bots joined per second.
    pub join_rate: f64,
    /// Seconds after the last join before measuring.
    pub warmup: Duration,
    /// Measured seconds.
    pub measure: Duration,
    /// Bots turn back towards their start beyond this distance (blocks),
    /// so they stay near each other.
    pub roam: f64,
    /// The `core` mod's data folder.
    pub data: PathBuf,
    /// Name prefix (names are `<prefix><number>`).
    pub prefix: String,
    /// Groups of bots, each with its own home on a square grid around the
    /// spawn (`group_spacing` apart), so a group sees only itself once the
    /// bots have flown there.
    pub groups: usize,
    /// Distance between group homes (blocks; more than the view radius
    /// keeps groups out of each other's view).
    pub group_spacing: f64,
}

impl BotConfig {
    /// Defaults for `count` bots against `addr`.
    #[must_use]
    pub fn new(addr: SocketAddr, count: usize, data: PathBuf) -> Self {
        Self {
            addr,
            count,
            mix: [6, 2, 2],
            join_rate: 20.0,
            warmup: Duration::from_secs(20),
            measure: Duration::from_secs(60),
            roam: 48.0,
            data,
            prefix: "bot".into(),
            groups: 1,
            group_spacing: 400.0,
        }
    }
}

/// One bot's results.
#[derive(Clone, Debug, Default)]
pub struct BotReport {
    /// Player name.
    pub name: String,
    /// Behavior.
    pub behavior: &'static str,
    /// Received kbit/s during the measurement (QUIC UDP payload).
    pub rx_kbit: f64,
    /// Sent kbit/s during the measurement.
    pub tx_kbit: f64,
    /// Corrections applied by the bot's prediction.
    pub corrections: u64,
    /// Edits sent.
    pub edits: u64,
    /// Edits the server refused.
    pub rejected: u64,
    /// Other players and entities in view at the end.
    pub others: usize,
    /// Distance from its home at the end (blocks, horizontal).
    pub from_home: f64,
    /// Columns received while measuring (column streaming).
    pub columns: u64,
    /// Why it stopped early, if it did.
    pub error: Option<String>,
}

/// The harness result.
#[derive(Clone, Debug, Default)]
pub struct Report {
    /// Per bot.
    pub bots: Vec<BotReport>,
    /// Bots that could not join, with the reason.
    pub join_errors: Vec<String>,
    /// Measured time.
    pub measured: Duration,
    /// Process memory at the end, bytes.
    pub memory: Option<u64>,
}

impl Report {
    /// The text summary (in the style of the T1-15 route summary).
    #[must_use]
    pub fn summary(&self) -> String {
        let mut s = String::new();
        let ok: Vec<&BotReport> = self.bots.iter().filter(|b| b.error.is_none()).collect();
        let pct = |mut v: Vec<f64>, p: f64| {
            if v.is_empty() {
                return 0.0;
            }
            v.sort_by(f64::total_cmp);
            v[((v.len() - 1) as f64 * p).round() as usize]
        };
        let rx: Vec<f64> = ok.iter().map(|b| b.rx_kbit).collect();
        let tx: Vec<f64> = ok.iter().map(|b| b.tx_kbit).collect();
        let _ = writeln!(s, "bots load run");
        let _ = writeln!(
            s,
            "bots: {} joined, {} failed to join, {} stopped early",
            self.bots.len(),
            self.join_errors.len(),
            self.bots.len() - ok.len()
        );
        for b in ["walk", "fly", "build"] {
            let _ = write!(
                s,
                "{}{b} {}",
                if b == "walk" { "mix: " } else { ", " },
                self.bots.iter().filter(|x| x.behavior == b).count()
            );
        }
        let _ = writeln!(s);
        let _ = writeln!(s, "measured: {:.0} s", self.measured.as_secs_f64());
        let _ = writeln!(
            s,
            "received per bot: p50 {:.1} kbit/s, p95 {:.1}, max {:.1}, mean {:.1}",
            pct(rx.clone(), 0.5),
            pct(rx.clone(), 0.95),
            pct(rx.clone(), 1.0),
            rx.iter().sum::<f64>() / rx.len().max(1) as f64
        );
        let _ = writeln!(
            s,
            "sent per bot: p50 {:.1} kbit/s, p95 {:.1}, max {:.1}",
            pct(tx.clone(), 0.5),
            pct(tx.clone(), 0.95),
            pct(tx, 1.0)
        );
        let _ = writeln!(
            s,
            "others in view per bot: p50 {:.0}, max {:.0}",
            pct(ok.iter().map(|b| b.others as f64).collect(), 0.5),
            pct(ok.iter().map(|b| b.others as f64).collect(), 1.0)
        );
        let _ = writeln!(
            s,
            "distance from home: p50 {:.0} blocks, max {:.0}",
            pct(ok.iter().map(|b| b.from_home).collect(), 0.5),
            pct(ok.iter().map(|b| b.from_home).collect(), 1.0)
        );
        let _ = writeln!(
            s,
            "corrections: {}; edits: {} sent, {} refused",
            ok.iter().map(|b| b.corrections).sum::<u64>(),
            ok.iter().map(|b| b.edits).sum::<u64>(),
            ok.iter().map(|b| b.rejected).sum::<u64>()
        );
        let _ = writeln!(
            s,
            "bot process memory: {}",
            self.memory
                .map_or("n/a".into(), |m| format!("{} MB", m / (1 << 20)))
        );
        for e in self
            .join_errors
            .iter()
            .chain(self.bots.iter().filter_map(|b| b.error.as_ref()))
            .take(10)
        {
            let _ = writeln!(s, "error: {e}");
        }
        s
    }

    /// One CSV line per bot.
    #[must_use]
    pub fn csv(&self) -> String {
        let mut s = String::from(
            "name,behavior,rx_kbit,tx_kbit,corrections,edits,refused,others,from_home,columns,error\n",
        );
        for b in &self.bots {
            let _ = writeln!(
                s,
                "{},{},{:.2},{:.2},{},{},{},{},{:.1},{},{}",
                b.name,
                b.behavior,
                b.rx_kbit,
                b.tx_kbit,
                b.corrections,
                b.edits,
                b.rejected,
                b.others,
                b.from_home,
                b.columns,
                b.error.as_deref().unwrap_or("").replace(',', ";")
            );
        }
        s
    }
}

/// The offset of group `g`'s home from the spawn: a square grid centered
/// on the spawn (group 0 at the spawn when there is one group).
fn group_home(g: usize, config: &BotConfig) -> DVec3 {
    let n = config.groups.max(1);
    let side = (n as f64).sqrt().ceil() as usize;
    let (gx, gz) = ((g % side) as f64, (g / side) as f64);
    let mid = (side as f64 - 1.0) / 2.0;
    DVec3::new(
        (gx - mid.floor()) * config.group_spacing,
        0.0,
        (gz - mid.floor()) * config.group_spacing,
    )
}

/// splitmix64.
fn mix(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

struct Bot {
    session: Session,
    behavior: Behavior,
    report: BotReport,
    seed: u64,
    home: DVec3,
    yaw: f64,
    last_pos: DVec3,
    still: u32,
    placed: Option<BlockPos>,
    stone: Option<BlockStateId>,
    /// Until this tick the bot walks around an obstacle (no homing).
    detour_until: u64,
    bytes_at_start: Option<(u64, u64)>,
    columns_at_start: u64,
}

impl Bot {
    fn input(&mut self, tick: u64, roam: f64) -> PlayerInput {
        let r = mix(self.seed ^ tick);
        let pos = self.session.player().pos;
        let moved = (pos - self.last_pos).length();
        self.last_pos = pos;
        self.still = if moved < 0.01 { self.still + 1 } else { 0 };
        // Turn now and then; head home when too far.
        if tick % 80 == self.seed % 80 {
            self.yaw += (r % 1000) as f64 / 1000.0 * 2.4 - 1.2;
        }
        // Blocked (a wall higher than a jump): turn aside for a while.
        if self.still > 20 {
            self.yaw += if r % 2 == 0 { 1.6 } else { -1.6 };
            self.detour_until = tick + 40 + r % 60;
            self.still = 0;
        }
        let away = pos - self.home;
        if tick >= self.detour_until && (away.x * away.x + away.z * away.z).sqrt() > roam {
            self.yaw = (away.x).atan2(away.z); // face home (yaw 0 is −Z)
        }
        let turn = self.yaw - self.session_yaw();
        self.session.look(turn, 0.0);
        let mut input = PlayerInput {
            forward: 1.0,
            ..PlayerInput::default()
        };
        let far_from_home = (away.x * away.x + away.z * away.z).sqrt() > roam * 1.5;
        // Travelling to a far home: fly there (terrain cannot block it),
        // then land and start the behavior.
        if far_from_home || (self.behavior != Behavior::Fly && self.session.is_flying()) {
            if !self.session.is_flying() {
                input.jump = true;
                input.toggle_fly = tick % 4 == 2;
            } else if far_from_home {
                // High enough for hills; climb harder when a mountain blocks.
                input.sprint = true;
                input.jump = pos.y < self.home.y + 40.0 || self.still > 2;
            } else {
                // Arrived: the double tap again ends the flight.
                input.forward = 0.0;
                input.toggle_fly = tick % 8 == 0;
            }
            return input;
        }
        match self.behavior {
            Behavior::Walk => {
                input.jump = self.still > 5 || r % 97 == 0;
            }
            Behavior::Fly => {
                if !self.session.is_flying() {
                    // Jump, then the double tap while rising.
                    input.jump = true;
                    input.toggle_fly = tick % 4 == 2;
                } else {
                    let phase = (tick / 40 + self.seed) % 4;
                    input.jump = phase == 0;
                    input.descend = phase == 2;
                    input.sprint = true;
                }
            }
            Behavior::Build => {
                input.forward = if tick % 40 < 10 { 1.0 } else { 0.0 };
                input.jump = self.still > 5 && input.forward > 0.0;
                if tick % 10 == self.seed % 10 {
                    self.build();
                }
            }
        }
        input
    }

    fn session_yaw(&self) -> f64 {
        // The view direction holds the yaw: dir = (−sin, ·, −cos).
        let d = self.session.view_dir();
        (-d.x).atan2(-d.z)
    }

    /// Places a block next to the head, or digs the one placed last.
    fn build(&mut self) {
        if let Some(p) = self.placed.take() {
            if self.session.edit_cell(p, None) {
                self.report.edits += 1;
            }
            return;
        }
        let Some(stone) = self.stone else { return };
        let pos = self.session.player().pos;
        let (s, c) = self.yaw.sin_cos();
        let at = BlockPos::new(
            (pos.x - s * 2.0).floor() as i32,
            pos.y.floor() as i32 + 1,
            (pos.z - c * 2.0).floor() as i32,
        );
        if self.session.map().get_block(at) == Some(BlockStateId::AIR)
            && self.session.edit_cell(at, Some(stone))
        {
            self.report.edits += 1;
            self.placed = Some(at);
        }
    }
}

/// Runs the harness. `progress` is called about once a second with the
/// number of bots playing.
pub fn run(config: &BotConfig, mut progress: impl FnMut(usize, &str)) -> Report {
    let mut report = Report::default();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_name("bots-net")
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            report.join_errors.push(format!("runtime: {e}"));
            return report;
        }
    };
    let total: u32 = config.mix.iter().sum::<u32>().max(1);
    let behavior = |i: usize| {
        let k = (i as u32 * 7919) % total;
        if k < config.mix[0] {
            Behavior::Walk
        } else if k < config.mix[0] + config.mix[1] {
            Behavior::Fly
        } else {
            Behavior::Build
        }
    };
    let mut bots: Vec<Bot> = Vec::with_capacity(config.count);
    let join_every = Duration::from_secs_f64(1.0 / config.join_rate.max(0.1));
    let start = Instant::now();
    let mut next_join = start;
    let mut next_tick = start;
    let mut tick = 0u64;
    let mut joined_all: Option<Instant> = None;
    let mut measure_start: Option<Instant> = None;
    let mut last_progress = start;
    loop {
        let now = Instant::now();
        // Join the next bot.
        if bots.len() + report.join_errors.len() < config.count && now >= next_join {
            let i = bots.len() + report.join_errors.len();
            let name = format!("{}{i:04}", config.prefix);
            let mut sc = SessionConfig::new(config.data.clone());
            sc.headless = true;
            sc.runtime = Some(runtime.handle().clone());
            sc.threads = 1;
            match Session::connect(config.addr, &name, sc) {
                Ok(session) => {
                    let pos = session.player().pos;
                    let stone = session.registry().state_id("core:stone");
                    let b = behavior(i);
                    bots.push(Bot {
                        session,
                        behavior: b,
                        report: BotReport {
                            name,
                            behavior: b.name(),
                            ..BotReport::default()
                        },
                        seed: mix(i as u64 + 1),
                        home: pos + group_home(i % config.groups.max(1), config),
                        yaw: (mix(i as u64) % 628) as f64 / 100.0,
                        last_pos: pos,
                        still: 0,
                        placed: None,
                        stone,
                        detour_until: 0,
                        bytes_at_start: None,
                        columns_at_start: 0,
                    });
                }
                Err(e) => report.join_errors.push(format!("{name}: {e}")),
            }
            next_join += join_every;
            if bots.len() + report.join_errors.len() == config.count {
                joined_all = Some(Instant::now());
            }
        }
        // Frames.
        for b in bots.iter_mut().filter(|b| b.report.error.is_none()) {
            if let Err(e) = b.session.update() {
                b.report.error = Some(e.to_string());
            }
        }
        if now >= next_tick {
            for b in bots.iter_mut().filter(|b| b.report.error.is_none()) {
                if b.session.is_playing() {
                    let input = b.input(tick, config.roam);
                    b.session.fixed_tick(&input);
                }
            }
            tick += 1;
            next_tick += Duration::from_millis(50);
        }
        // Measurement window.
        if let Some(all) = joined_all {
            if measure_start.is_none() && now >= all + config.warmup {
                measure_start = Some(now);
                for b in &mut bots {
                    b.bytes_at_start = b.session.net_stats().map(|n| n.bytes);
                    b.columns_at_start = b.session.net_stats().map_or(0, |n| n.columns);
                }
            }
            if let Some(m) = measure_start
                && now >= m + config.measure
            {
                report.measured = now - m;
                break;
            }
        }
        if now.duration_since(last_progress) >= Duration::from_secs(1) {
            last_progress = now;
            let playing = bots.iter().filter(|b| b.session.is_playing()).count();
            let phase = match (joined_all, measure_start) {
                (None, _) => "joining",
                (Some(_), None) => "warm-up",
                (Some(_), Some(_)) => "measuring",
            };
            progress(playing, phase);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let secs = report.measured.as_secs_f64().max(1e-9);
    for mut b in bots {
        if let (Some(n), Some(start)) = (b.session.net_stats(), b.bytes_at_start) {
            b.report.rx_kbit = (n.bytes.0 - start.0) as f64 * 8.0 / secs / 1000.0;
            b.report.tx_kbit = (n.bytes.1 - start.1) as f64 * 8.0 / secs / 1000.0;
            b.report.corrections = n.corrections;
            b.report.rejected = n.rejected_edits;
            b.report.others = n.others;
            b.report.columns = n.columns - b.columns_at_start;
        }
        let p = b.session.player().pos - b.home;
        b.report.from_home = (p.x * p.x + p.z * p.z).sqrt();
        report.bots.push(b.report);
        let _ = b.session.close();
    }
    report.memory = memory_stats::memory_stats().map(|m| m.physical_mem as u64);
    runtime.shutdown_timeout(Duration::from_secs(1));
    report
}
