//! Stage 1 exit measurement (T1-15) and frame spike attribution (T1-25): a
//! scripted route through a fixed-seed world, logging every frame time, the
//! process memory and, for every frame over 33 ms, where the frame went.
//! `client --bench-route SECONDS [--bench-mode fly|walk]`.
//!
//! Routes: `fly` (T1-15) flies 12 blocks above the ground at ≈ 5.5 blocks/s
//! at 16 chunks, the stress case; `walk` sprints on the ground (≈ 5.6
//! blocks/s, jumping over steps, turning when stuck) at 12 chunks, the exit
//! criterion read literally.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use client::session::{Session, UpdateCost};
use render::terrain::profiles::QualityProfile;
use sim::physics::PlayerInput;

use crate::view::SyncCost;

/// World seed of the route (fixed, so every run sees the same terrain).
pub const SEED: u64 = 12_345;
/// Seconds of loading before measuring starts.
pub const WARMUP: Duration = Duration::from_secs(30);
/// Side of the square route in blocks.
const SIDE: f64 = 1000.0;
/// Fly speed multiplier: ≈ 5.5 blocks/s, the sprinting pace.
const SPEED: f64 = 0.5;
/// A frame longer than this is a spike (the exit criterion).
pub const SPIKE_MS: f32 = 33.0;
/// Spikes recorded in full; later ones are only counted (no allocation while
/// measuring).
const SPIKE_CAPACITY: usize = 8192;
/// Walking: ticks between progress checks, and the progress that counts as
/// not stuck.
const STUCK_TICKS: u32 = 60;
const STUCK_BLOCKS: f64 = 2.0;

/// Which route to take.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum Route {
    /// Fly 12 blocks above the ground at 16 chunks (T1-15, the stress case).
    #[default]
    Fly,
    /// Sprint on the ground at 12 chunks (the exit criterion literally).
    Walk,
}

impl Route {
    /// The quality profile the route runs at.
    #[must_use]
    pub fn quality(self) -> QualityProfile {
        match self {
            Self::Fly => QualityProfile::High,
            Self::Walk => QualityProfile::Default,
        }
    }

    fn describe(self) -> &'static str {
        match self {
            Self::Fly => "flown 12 blocks above the ground at ≈ 5.5 blocks/s",
            Self::Walk => "walked (sprinting) on the ground at ≈ 5.6 blocks/s",
        }
    }
}

/// Where one frame's time went, measured inside `App::frame`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FrameCost {
    /// `Session::update` and its parts.
    pub update: UpdateCost,
    /// Fixed 20 TPS ticks run in this frame.
    pub ticks: Duration,
    /// Number of those ticks.
    pub tick_count: u32,
    /// Building the UI (egui).
    pub ui: Duration,
    /// Terrain and light uploads.
    pub sync: SyncCost,
    /// Camera, culling and draw lists.
    pub prepare: Duration,
    /// Waiting for the next surface texture.
    pub acquire: Duration,
    /// Render passes, egui tessellation and buffers.
    pub encode: Duration,
    /// `queue.submit`.
    pub submit: Duration,
    /// `queue.present`.
    pub present: Duration,
}

/// Names of the phases [`FrameCost::phases`] reports, in order.
pub const PHASES: [&str; 15] = [
    "generation",
    "saved_reads",
    "relight",
    "saves",
    "autosave",
    "ticks",
    "ui",
    "upload_batches",
    "upload_light",
    "prepare",
    "acquire",
    "encode",
    "submit",
    "present",
    "outside",
];

impl FrameCost {
    /// Milliseconds per phase (names in [`PHASES`]) for a frame interval of
    /// `frame_ms`; "outside" is the part no engine phase accounts for (event
    /// handling, the OS, the driver outside our calls).
    #[must_use]
    pub fn phases(&self, frame_ms: f32) -> [f32; 15] {
        let ms = |d: Duration| d.as_secs_f32() * 1000.0;
        let u = &self.update;
        let other_update = u
            .total
            .saturating_sub(u.loader + u.light + u.saves + u.autosave);
        let mut p = [
            // Loader work that is not reading saves, plus the small rest of
            // `update` (spawn placement), which has no phase of its own.
            ms(u.loader.saturating_sub(u.source) + other_update),
            ms(u.source),
            ms(u.light),
            ms(u.saves),
            ms(u.autosave),
            ms(self.ticks),
            ms(self.ui),
            ms(self.sync.batch_time),
            ms(self.sync.light_time),
            ms(self.prepare),
            ms(self.acquire),
            ms(self.encode),
            ms(self.submit),
            ms(self.present),
            0.0,
        ];
        let engine: f32 = p.iter().sum();
        p[14] = (frame_ms - engine).max(0.0);
        p
    }
}

/// One frame over [`SPIKE_MS`].
#[derive(Clone, Copy, Debug)]
struct Spike {
    frame: u32,
    ms: f32,
    cost: FrameCost,
}

/// The running measurement.
pub struct Bench {
    route: Route,
    duration: Duration,
    started: Option<Instant>,
    frames: Vec<f32>,
    ram: Vec<u64>,
    last_ram: Instant,
    segment_start: Option<[f64; 2]>,
    flying_requested: bool,
    /// Horizontal distance flown while measuring.
    distance: f64,
    last_pos: Option<[f64; 2]>,
    spikes: Vec<Spike>,
    /// Spikes after the first [`SPIKE_CAPACITY`] (counted, not recorded).
    spikes_dropped: u32,
    /// Walking: ticks since the last progress check and where it was.
    stuck_ticks: u32,
    stuck_from: Option<[f64; 2]>,
}

impl Bench {
    /// A measurement of `seconds` on `route` after the warm-up.
    #[must_use]
    pub fn new(seconds: u32, route: Route) -> Self {
        Self {
            route,
            duration: Duration::from_secs(u64::from(seconds)),
            started: None,
            frames: Vec::with_capacity(seconds as usize * 70),
            ram: Vec::new(),
            last_ram: Instant::now(),
            segment_start: None,
            flying_requested: false,
            distance: 0.0,
            last_pos: None,
            spikes: Vec::with_capacity(SPIKE_CAPACITY),
            spikes_dropped: 0,
            stuck_ticks: 0,
            stuck_from: None,
        }
    }

    /// The route's input for this tick (and yaw changes on the session).
    pub fn input(&mut self, s: &mut Session, playing_since: Option<Instant>) -> PlayerInput {
        let Some(since) = playing_since else {
            return PlayerInput::default();
        };
        if self.route == Route::Walk {
            return self.walk(s, since);
        }
        if !s.is_flying() {
            // Jump, then double-tap: the toggle comes while rising.
            let toggle = self.flying_requested;
            self.flying_requested = true;
            return PlayerInput {
                jump: !toggle,
                toggle_fly: toggle,
                ..PlayerInput::default()
            };
        }
        s.set_fly_speed(SPEED);
        if since.elapsed() < Duration::from_secs(5) {
            return PlayerInput::default();
        }
        let p = s.player().pos;
        self.track_route(s, [p.x, p.z]);
        // Keep 12 blocks above the ground (mountains are flown over).
        let ground = f64::from(
            s.map()
                .height(p.x.floor() as i32, p.z.floor() as i32)
                .unwrap_or(64),
        );
        let target = ground.max(62.0) + 12.0;
        PlayerInput {
            forward: 1.0,
            jump: p.y < target,
            descend: p.y > target + 4.0,
            ..PlayerInput::default()
        }
    }

    /// Distance while measuring, and a 90° turn at the end of each side.
    fn track_route(&mut self, s: &mut Session, p: [f64; 2]) {
        let dist =
            |a: [f64; 2], b: [f64; 2]| ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt();
        if self.started.is_some() {
            if let Some(l) = self.last_pos {
                self.distance += dist(p, l);
            }
            self.last_pos = Some(p);
        }
        let start = *self.segment_start.get_or_insert(p);
        if dist(p, start) >= SIDE {
            s.look(std::f64::consts::FRAC_PI_2, 0.0);
            self.segment_start = Some(p);
        }
    }

    /// The walking route: sprint forward, jump when blocked or in water,
    /// turn 90° when stuck (less than 2 blocks of progress in 3 s).
    fn walk(&mut self, s: &mut Session, since: Instant) -> PlayerInput {
        if since.elapsed() < Duration::from_secs(5) {
            return PlayerInput::default();
        }
        let body = *s.player();
        let p = [body.pos.x, body.pos.z];
        self.track_route(s, p);
        let from = *self.stuck_from.get_or_insert(p);
        self.stuck_ticks += 1;
        if self.stuck_ticks >= STUCK_TICKS {
            if ((p[0] - from[0]).powi(2) + (p[1] - from[1]).powi(2)).sqrt() < STUCK_BLOCKS {
                s.look(std::f64::consts::FRAC_PI_2, 0.0);
                self.segment_start = Some(p);
            }
            self.stuck_ticks = 0;
            self.stuck_from = Some(p);
        }
        PlayerInput {
            forward: 1.0,
            sprint: true,
            jump: walk_jump(body.on_ground, body.in_fluid, body.vel.x, body.vel.z),
            ..PlayerInput::default()
        }
    }

    /// Records one frame interval `dt` and, if it is a spike, where the
    /// previous frame's time went (`dt` ends where that frame's work began
    /// the interval). Returns true when the measurement is over.
    pub fn frame(
        &mut self,
        dt: Duration,
        cost: &FrameCost,
        playing_since: Option<Instant>,
    ) -> bool {
        let Some(since) = playing_since else {
            return false;
        };
        if since.elapsed() < WARMUP {
            return false;
        }
        let started = *self.started.get_or_insert_with(Instant::now);
        let ms = dt.as_secs_f32() * 1000.0;
        if ms > SPIKE_MS {
            if self.spikes.len() < SPIKE_CAPACITY {
                self.spikes.push(Spike {
                    frame: self.frames.len() as u32,
                    ms,
                    cost: *cost,
                });
            } else {
                self.spikes_dropped += 1;
            }
        }
        self.frames.push(ms);
        if self.last_ram.elapsed() >= Duration::from_secs(1) {
            self.last_ram = Instant::now();
            if let Some(m) = memory_stats::memory_stats() {
                self.ram.push(m.physical_mem as u64);
            }
        }
        started.elapsed() >= self.duration
    }

    /// Writes `frames-*.csv` and `summary-*.txt` into `dir` and returns the summary.
    pub fn finish(
        &self,
        dir: &Path,
        gpu: &str,
        profile: &str,
    ) -> std::io::Result<(PathBuf, String)> {
        std::fs::create_dir_all(dir)?;
        let stamp = client::worlds::now();
        let mut csv = String::from("frame,ms\n");
        for (i, f) in self.frames.iter().enumerate() {
            let _ = writeln!(csv, "{i},{f:.3}");
        }
        std::fs::write(dir.join(format!("frames-{stamp}.csv")), csv)?;
        std::fs::write(dir.join(format!("spikes-{stamp}.csv")), self.spikes_csv())?;
        let summary = format!(
            "{}route: {}, {:.0} blocks {}; {} memory samples\n{}",
            summarize(&self.frames, &self.ram, gpu, profile),
            route_name(self.route),
            self.distance,
            self.route.describe(),
            self.ram.len(),
            self.causes(),
        );
        let path = dir.join(format!("summary-{stamp}.txt"));
        std::fs::write(&path, &summary)?;
        Ok((path, summary))
    }

    /// One row per spike: its length, each phase in ms and the counters.
    fn spikes_csv(&self) -> String {
        let mut csv = String::from("frame,ms");
        for name in PHASES {
            let _ = write!(csv, ",{name}_ms");
        }
        csv.push_str(
            ",cause,saved_reads,unloaded_columns,ticks,batches,quads,light_sections,removed_sections\n",
        );
        for sp in &self.spikes {
            let phases = sp.cost.phases(sp.ms);
            let _ = write!(csv, "{},{:.3}", sp.frame, sp.ms);
            for v in phases {
                let _ = write!(csv, ",{v:.3}");
            }
            let c = &sp.cost;
            let _ = writeln!(
                csv,
                ",{},{},{},{},{},{},{},{}",
                PHASES[dominant(&phases)],
                c.update.source_calls,
                c.update.unloaded,
                c.tick_count,
                c.sync.batches,
                c.sync.quads,
                c.sync.light_sections,
                c.sync.removed_sections,
            );
        }
        csv
    }

    /// The summary's spike section.
    fn causes(&self) -> String {
        causes(
            self.spikes.iter().map(|s| (s.ms, s.cost.phases(s.ms))),
            self.spikes_dropped,
        )
    }
}

fn route_name(r: Route) -> &'static str {
    match r {
        Route::Fly => "fly",
        Route::Walk => "walk",
    }
}

/// Jump when blocked on the ground (horizontal speed below 0.1 blocks per
/// tick, about a third of sprinting) or to swim up in water.
#[must_use]
fn walk_jump(on_ground: bool, in_fluid: bool, vx: f64, vz: f64) -> bool {
    in_fluid || (on_ground && vx.hypot(vz) < 0.1)
}

/// Index of the longest phase.
#[must_use]
fn dominant(phases: &[f32; 15]) -> usize {
    let mut best = 0;
    for (i, &v) in phases.iter().enumerate() {
        if v > phases[best] {
            best = i;
        }
    }
    best
}

/// "Spike causes": per dominant phase, how many spikes and their average
/// length, longest first by count; plus the mean of every phase over spikes.
#[must_use]
fn causes(spikes: impl Iterator<Item = (f32, [f32; 15])>, dropped: u32) -> String {
    let mut count = [0u32; 15];
    let mut length = [0f32; 15];
    let mut mean = [0f32; 15];
    let mut n = 0u32;
    for (ms, phases) in spikes {
        let d = dominant(&phases);
        count[d] += 1;
        length[d] += ms;
        for (m, v) in mean.iter_mut().zip(phases) {
            *m += v;
        }
        n += 1;
    }
    let mut out = format!("spikes attributed: {n}");
    if dropped > 0 {
        let _ = write!(out, " (+{dropped} counted, not recorded)");
    }
    out.push('\n');
    if n == 0 {
        return out;
    }
    let mut order: Vec<usize> = (0..15).filter(|&i| count[i] > 0).collect();
    order.sort_by(|&a, &b| count[b].cmp(&count[a]).then(a.cmp(&b)));
    out.push_str("spike causes (longest phase):");
    for i in order {
        let _ = write!(
            out,
            " {} {} (avg {:.1} ms);",
            PHASES[i],
            count[i],
            length[i] / count[i] as f32
        );
    }
    out.push_str("\nmean phase over spikes, ms:");
    for (i, m) in mean.iter().enumerate() {
        let _ = write!(out, " {} {:.2};", PHASES[i], m / n as f32);
    }
    out.push('\n');
    out
}

/// The exit numbers from frame times (ms) and memory samples (bytes).
#[must_use]
pub fn summarize(frames: &[f32], ram: &[u64], gpu: &str, profile: &str) -> String {
    let mut sorted = frames.to_vec();
    sorted.sort_by(f32::total_cmp);
    let n = sorted.len().max(1);
    let total: f32 = frames.iter().sum();
    let avg_fps = frames.len() as f32 * 1000.0 / total.max(1e-3);
    // "1% low": the average FPS of the slowest 1% of frames.
    let slow = &sorted[n - n.div_ceil(100).min(n)..];
    let low1 = slow.len() as f32 * 1000.0 / slow.iter().sum::<f32>().max(1e-3);
    let pct = |p: f32| {
        sorted
            .get(((n - 1) as f32 * p) as usize)
            .copied()
            .unwrap_or(0.0)
    };
    let mib = |b: u64| b as f64 / (1024.0 * 1024.0);
    let ram_peak = ram.iter().copied().max().unwrap_or(0);
    let ram_avg = ram.iter().sum::<u64>() / (ram.len().max(1) as u64);
    let spikes = frames.iter().filter(|&&f| f > SPIKE_MS).count();
    format!(
        "stage 1 exit route: {} frames over {:.0} s, {gpu}, profile {profile}\n\
         average FPS {avg_fps:.1} (target 60)\n\
         1% low FPS {low1:.1} (target >= 45)\n\
         frame time p50 {:.2} ms, p99 {:.2} ms, max {:.2} ms\n\
         frames > 33 ms: {spikes} (target 0)\n\
         process memory peak {:.0} MiB, average {:.0} MiB (target <= 1536 MiB; internal budget 1024 MiB)\n",
        frames.len(),
        total / 1000.0,
        pct(0.5),
        pct(0.99),
        sorted.last().copied().unwrap_or(0.0),
        mib(ram_peak),
        mib(ram_avg),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_numbers() {
        let mut frames = vec![16.0f32; 990];
        frames.extend([40.0f32; 10]);
        let s = summarize(&frames, &[100 << 20, 300 << 20], "test GPU", "Default");
        assert!(s.contains("frames > 33 ms: 10"), "{s}");
        assert!(s.contains("1% low FPS 25.0"), "{s}");
        assert!(s.contains("peak 300 MiB, average 200 MiB"), "{s}");
        assert!(s.contains("max 40.00 ms"), "{s}");
    }

    fn ms(v: u64) -> Duration {
        Duration::from_millis(v)
    }

    #[test]
    fn phases_split_update_and_leave_the_rest_outside() {
        let cost = FrameCost {
            update: UpdateCost {
                total: ms(60),
                loader: ms(50),
                source: ms(40),
                source_calls: 3,
                light: ms(5),
                saves: ms(1),
                unloaded: 0,
                autosave: ms(0),
            },
            ticks: ms(2),
            present: ms(3),
            ..FrameCost::default()
        };
        let p = cost.phases(100.0);
        let at = |name: &str| p[PHASES.iter().position(|&n| n == name).expect("phase")];
        // Generation = loader without reads, plus the 4 ms of update no part covers.
        assert!((at("generation") - 14.0).abs() < 1e-3, "{p:?}");
        assert!((at("saved_reads") - 40.0).abs() < 1e-3);
        assert!((at("relight") - 5.0).abs() < 1e-3);
        // 100 − (60 + 2 + 3) = 35 ms outside the engine's phases.
        assert!((at("outside") - 35.0).abs() < 1e-3, "{p:?}");
        let total: f32 = p.iter().sum();
        assert!((total - 100.0).abs() < 1e-3);
        assert_eq!(PHASES[dominant(&p)], "saved_reads");
    }

    #[test]
    fn outside_never_negative() {
        let cost = FrameCost {
            acquire: ms(50),
            ..FrameCost::default()
        };
        let p = cost.phases(40.0);
        assert_eq!(p[14], 0.0);
        assert_eq!(PHASES[dominant(&p)], "acquire");
    }

    #[test]
    fn causes_count_dominant_phases() {
        let mut a = [0f32; 15];
        a[1] = 50.0;
        let mut b = [0f32; 15];
        b[14] = 80.0;
        let s = causes([(60.0, a), (40.0, a), (90.0, b)].into_iter(), 2);
        assert!(
            s.contains("spikes attributed: 3 (+2 counted, not recorded)"),
            "{s}"
        );
        assert!(
            s.contains("spike causes (longest phase): saved_reads 2 (avg 50.0 ms); outside 1 (avg 90.0 ms);"),
            "{s}"
        );
        assert_eq!(causes(std::iter::empty(), 0), "spikes attributed: 0\n");
    }

    #[test]
    fn spikes_are_recorded_with_the_cost() {
        let mut b = Bench::new(1, Route::Walk);
        let since = Instant::now().checked_sub(WARMUP + Duration::from_secs(1));
        let cost = FrameCost {
            acquire: ms(30),
            ..FrameCost::default()
        };
        assert!(!b.frame(ms(16), &cost, since));
        assert!(!b.frame(ms(45), &cost, since));
        assert_eq!(b.frames.len(), 2);
        assert_eq!(b.spikes.len(), 1);
        assert_eq!(b.spikes[0].frame, 1);
        let csv = b.spikes_csv();
        let mut lines = csv.lines();
        let header = lines.next().expect("header");
        assert!(header.starts_with("frame,ms,generation_ms,"), "{header}");
        let row = lines.next().expect("row");
        assert!(row.starts_with("1,45.000,"), "{row}");
        // 45 − 30 ms acquire = 15 ms outside, so acquire dominates.
        assert!(row.contains(",acquire,"), "{row}");
        assert_eq!(header.split(',').count(), row.split(',').count());
    }

    #[test]
    fn walking_jumps_when_blocked_or_swimming() {
        assert!(walk_jump(true, false, 0.0, 0.02));
        assert!(!walk_jump(true, false, 0.2, 0.15));
        assert!(!walk_jump(false, false, 0.0, 0.0));
        assert!(walk_jump(false, true, 0.3, 0.0));
    }

    #[test]
    fn routes_use_their_profiles() {
        assert_eq!(Route::Walk.quality().settings().render_distance, 12);
        assert_eq!(Route::Fly.quality().settings().render_distance, 16);
    }
}
