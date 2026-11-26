//! T1-15 stage 1 exit measurement: a scripted flight at walking height and
//! sprint speed through a fixed-seed world, logging every frame time and the
//! process memory. `client --bench-route SECONDS`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use client::session::Session;
use sim::physics::PlayerInput;

/// World seed of the route (fixed, so every run sees the same terrain).
pub const SEED: u64 = 12_345;
/// Seconds of loading before measuring starts.
pub const WARMUP: Duration = Duration::from_secs(30);
/// Side of the square route in blocks.
const SIDE: f64 = 1000.0;
/// Fly speed multiplier: ≈ 5.5 blocks/s, the sprinting pace.
const SPEED: f64 = 0.5;

/// The running measurement.
pub struct Bench {
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
}

impl Bench {
    /// A measurement of `seconds` after the warm-up.
    #[must_use]
    pub fn new(seconds: u32) -> Self {
        Self {
            duration: Duration::from_secs(u64::from(seconds)),
            started: None,
            frames: Vec::with_capacity(seconds as usize * 70),
            ram: Vec::new(),
            last_ram: Instant::now(),
            segment_start: None,
            flying_requested: false,
            distance: 0.0,
            last_pos: None,
        }
    }

    /// The route's input for this tick (and yaw changes on the session).
    pub fn input(&mut self, s: &mut Session, playing_since: Option<Instant>) -> PlayerInput {
        let Some(since) = playing_since else {
            return PlayerInput::default();
        };
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
        if self.started.is_some() {
            if let Some(l) = self.last_pos {
                self.distance += ((p.x - l[0]).powi(2) + (p.z - l[1]).powi(2)).sqrt();
            }
            self.last_pos = Some([p.x, p.z]);
        }
        let start = *self.segment_start.get_or_insert([p.x, p.z]);
        if ((p.x - start[0]).powi(2) + (p.z - start[1]).powi(2)).sqrt() >= SIDE {
            s.look(std::f64::consts::FRAC_PI_2, 0.0);
            self.segment_start = Some([p.x, p.z]);
        }
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

    /// Records one frame; returns true when the measurement is over.
    pub fn frame(&mut self, dt: Duration, playing_since: Option<Instant>) -> bool {
        let Some(since) = playing_since else {
            return false;
        };
        if since.elapsed() < WARMUP {
            return false;
        }
        let started = *self.started.get_or_insert_with(Instant::now);
        self.frames.push(dt.as_secs_f32() * 1000.0);
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
        let summary = format!(
            "{}route: {:.0} blocks flown at ≈ 5.5 blocks/s; {} memory samples\n",
            summarize(&self.frames, &self.ram, gpu, profile),
            self.distance,
            self.ram.len()
        );
        let path = dir.join(format!("summary-{stamp}.txt"));
        std::fs::write(&path, &summary)?;
        Ok((path, summary))
    }
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
    let spikes = frames.iter().filter(|&&f| f > 33.0).count();
    let mib = |b: u64| b as f64 / (1024.0 * 1024.0);
    let ram_peak = ram.iter().copied().max().unwrap_or(0);
    let ram_avg = ram.iter().sum::<u64>() / (ram.len().max(1) as u64);
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
}
