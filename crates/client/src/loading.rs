//! Chunk loading queue with priorities (T1-10).
//!
//! Columns around the viewer go through `generate → light → mesh` as jobs on
//! the T0-02 pool; the main thread only schedules, cancels and applies
//! results, within a time budget per tick (no frame spikes). Jobs never touch
//! the `WorldMap`: they get copies of what they read (the map is owned by the
//! main thread, AGENTS.md rule 3).
//!
//! Priority (`docs/process/roadmap.md`, stage 1 additions): distance to the
//! viewer, discounted ahead of the movement direction (more when moving
//! faster); mesh chunks also weigh the vertical distance (height).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};

use engine_core::jobs::{CancelToken, JobPool, Priority};
use light::{SectionLight, WorldLight, light_column, props_from_registry};
use mesh::{MeshChunkBuilder, MeshChunkQuads, SectionNeighborhood};
use world::coords::{MeshChunkPos, SectionPos};
use world::{Column, ColumnPos, ColumnState, Registry, Section, WorldMap};
use worldgen::{GenContext, Pipeline};

/// Work closer than this priority score (blocks) runs in the `Visible` pool
/// class; the rest in `Prefetch`.
const NEAR_BLOCKS: f64 = 48.0;

/// The viewer: position in blocks and velocity in blocks per second.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Viewer {
    /// Position (x, y, z) in blocks.
    pub pos: [f64; 3],
    /// Velocity (x, y, z) in blocks per second.
    pub velocity: [f64; 3],
}

impl Viewer {
    /// The column the viewer stands in.
    #[must_use]
    pub fn column(&self) -> ColumnPos {
        ColumnPos {
            x: (self.pos[0] / 16.0).floor() as i32,
            z: (self.pos[2] / 16.0).floor() as i32,
        }
    }

    /// Horizontal movement bias in `0..=0.5`: how strongly targets ahead are
    /// preferred (0 when standing, 0.5 from 10 blocks/s).
    fn bias(&self) -> (f64, [f64; 2]) {
        let (vx, vz) = (self.velocity[0], self.velocity[2]);
        let speed = (vx * vx + vz * vz).sqrt();
        if speed < 1e-6 {
            return (0.0, [0.0, 0.0]);
        }
        ((speed / 20.0).min(0.5), [vx / speed, vz / speed])
    }

    /// Load priority of a column (lower = sooner).
    #[must_use]
    pub fn column_priority(&self, c: ColumnPos) -> f64 {
        let dx = f64::from(c.x) * 16.0 + 8.0 - self.pos[0];
        let dz = f64::from(c.z) * 16.0 + 8.0 - self.pos[2];
        self.score(dx, 0.0, dz)
    }

    /// Mesh priority of a 32³ mesh chunk (lower = sooner), including height.
    #[must_use]
    pub fn mesh_priority(&self, m: MeshChunkPos) -> f64 {
        let dx = f64::from(m.x) * 32.0 + 16.0 - self.pos[0];
        let dy = f64::from(m.y) * 32.0 + 16.0 - self.pos[1];
        let dz = f64::from(m.z) * 32.0 + 16.0 - self.pos[2];
        self.score(dx, dy, dz)
    }

    fn score(&self, dx: f64, dy: f64, dz: f64) -> f64 {
        let dist = (dx * dx + dy * dy + dz * dz).sqrt();
        let horizontal = (dx * dx + dz * dz).sqrt();
        let (bias, dir) = self.bias();
        if horizontal < 1e-6 || bias == 0.0 {
            return dist;
        }
        let cos = (dx * dir[0] + dz * dir[1]) / horizontal;
        dist * (1.0 - bias * cos)
    }
}

/// Loading parameters.
#[derive(Clone, Copy, Debug)]
pub struct LoadConfig {
    /// Columns within this Chebyshev radius are loaded.
    pub radius: i32,
    /// Columns are unloaded (and their jobs cancelled) beyond `radius + margin`.
    pub unload_margin: i32,
    /// Main-thread time per tick for applying finished results.
    pub apply_budget: Duration,
    /// Main-thread time per tick in total; job submission (which copies the
    /// job inputs) stops when it is used up and continues next tick.
    pub tick_budget: Duration,
    /// Jobs in flight at most (bounds queued work and main-thread copying).
    pub max_in_flight: usize,
}

impl Default for LoadConfig {
    fn default() -> Self {
        Self {
            radius: 12,
            unload_margin: 2,
            apply_budget: Duration::from_millis(1),
            tick_budget: Duration::from_millis(2),
            max_in_flight: 32,
        }
    }
}

/// Counters for tests and profiling.
#[derive(Clone, Copy, Debug, Default)]
pub struct LoadStats {
    /// Jobs submitted.
    pub submitted: u64,
    /// Jobs cancelled because their target left the load area.
    pub cancelled: u64,
    /// Results applied.
    pub applied: u64,
    /// Longest main-thread apply batch.
    pub max_apply: Duration,
    /// Longest whole `tick` (scheduling, copying job inputs, applying).
    pub max_tick: Duration,
    /// Longest candidate scan and sort.
    pub max_scan: Duration,
    /// Longest single job submission (copying its inputs).
    pub max_submit: Duration,
    /// Longest unload/cancel step.
    pub max_unload: Duration,
}

enum JobOutput {
    /// The job could not produce its result; the target may be retried.
    Failed(Target),
    Generated(Column),
    Lit(ColumnPos, Vec<(SectionPos, SectionLight)>),
    Meshed(MeshChunkPos, MeshChunkQuads),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Target {
    Generate(ColumnPos),
    Light(ColumnPos),
    Mesh(MeshChunkPos),
}

/// Loads the world around a viewer.
pub struct Loader {
    config: LoadConfig,
    registry: Arc<Registry>,
    pipeline: Arc<Pipeline>,
    context: Arc<GenContext>,
    in_flight: HashMap<Target, CancelToken>,
    lit: HashSet<ColumnPos>,
    meshes: HashMap<MeshChunkPos, MeshChunkQuads>,
    finished: VecDeque<JobOutput>,
    stats: LoadStats,
}

impl Loader {
    /// A loader generating with `pipeline` and `context`.
    #[must_use]
    pub fn new(
        config: LoadConfig,
        registry: Arc<Registry>,
        pipeline: Arc<Pipeline>,
        context: Arc<GenContext>,
    ) -> Self {
        Self {
            config,
            registry,
            pipeline,
            context,
            in_flight: HashMap::new(),
            lit: HashSet::new(),
            meshes: HashMap::new(),
            finished: VecDeque::new(),
            stats: LoadStats::default(),
        }
    }

    /// Counters.
    #[must_use]
    pub fn stats(&self) -> LoadStats {
        self.stats
    }

    /// Meshes ready for upload.
    #[must_use]
    pub fn meshes(&self) -> &HashMap<MeshChunkPos, MeshChunkQuads> {
        &self.meshes
    }

    /// Whether a column has blocks, light and all its mesh chunks.
    #[must_use]
    pub fn is_ready(&self, map: &WorldMap, c: ColumnPos) -> bool {
        if !self.lit.contains(&c) {
            return false;
        }
        let b = map.bounds();
        let (y0, y1) = (
            b.min_section_y.div_euclid(2),
            (b.min_section_y + b.section_count as i32 - 1).div_euclid(2),
        );
        (y0..=y1).all(|my| {
            self.meshes.contains_key(&MeshChunkPos {
                x: c.x.div_euclid(2),
                y: my,
                z: c.z.div_euclid(2),
            })
        })
    }

    /// One main-thread step: unload and cancel what left the area, apply
    /// finished results within the budget, submit new jobs by priority.
    pub fn tick(
        &mut self,
        viewer: &Viewer,
        map: &mut WorldMap,
        light: &mut WorldLight,
        pool: &JobPool,
    ) {
        let start = Instant::now();
        let center = viewer.column();
        // Columns within the radius become ready: their mesh chunks need light
        // one column further (2×2 cores) and blocks two columns further (4×4),
        // so the kept area is radius + 2 + margin.
        let keep = self.config.radius + 2 + self.config.unload_margin;
        let inside =
            |c: ColumnPos, lim: i32| (c.x - center.x).abs() <= lim && (c.z - center.z).abs() <= lim;

        // 1. Cancel jobs and unload columns outside the kept area.
        self.in_flight.retain(|t, token| {
            let c = match *t {
                Target::Generate(c) | Target::Light(c) => c,
                Target::Mesh(m) => ColumnPos {
                    x: m.x * 2,
                    z: m.z * 2,
                },
            };
            let keep_it = inside(c, keep + 1);
            if !keep_it {
                token.cancel();
                self.stats.cancelled += 1;
            }
            keep_it
        });
        let gone: Vec<ColumnPos> = self
            .lit
            .iter()
            .copied()
            .filter(|&c| !inside(c, keep))
            .collect();
        for c in gone {
            self.lit.remove(&c);
        }
        let loaded: Vec<ColumnPos> = map
            .column_positions()
            .filter(|&c| !inside(c, keep))
            .collect();
        for c in loaded {
            map.remove_column(c);
            light.remove_column(c);
        }
        self.meshes.retain(|m, _| {
            inside(
                ColumnPos {
                    x: m.x * 2,
                    z: m.z * 2,
                },
                keep + 1,
            )
        });

        self.stats.max_unload = self.stats.max_unload.max(start.elapsed());
        // 2. Apply finished results within the budget.
        self.finished.extend(
            pool.drain_completed()
                .filter_map(|j| j.downcast::<JobOutput>()),
        );
        let apply_start = Instant::now();
        while let Some(out) = self.finished.pop_front() {
            self.apply(out, map, light, &inside, keep);
            self.stats.applied += 1;
            if apply_start.elapsed() >= self.config.apply_budget {
                break;
            }
        }
        self.stats.max_apply = self.stats.max_apply.max(apply_start.elapsed());
        self.schedule(viewer, map, pool, start);
        self.stats.max_tick = self.stats.max_tick.max(start.elapsed());
    }

    fn apply(
        &mut self,
        out: JobOutput,
        map: &mut WorldMap,
        light: &mut WorldLight,
        inside: &impl Fn(ColumnPos, i32) -> bool,
        keep: i32,
    ) {
        {
            match out {
                JobOutput::Failed(t) => {
                    self.in_flight.remove(&t);
                }
                JobOutput::Generated(column) => {
                    let pos = column.pos();
                    self.in_flight.remove(&Target::Generate(pos));
                    if inside(pos, keep) {
                        let _ = map.insert_column(column);
                        if let Some(s) = map.column_state_mut(pos) {
                            *s = ColumnState::Generating;
                        }
                    }
                }
                JobOutput::Lit(pos, sections) => {
                    self.in_flight.remove(&Target::Light(pos));
                    if inside(pos, keep) && map.column(pos).is_some() {
                        for (sp, l) in sections {
                            light.insert(sp, l);
                        }
                        self.lit.insert(pos);
                        if let Some(s) = map.column_state_mut(pos) {
                            *s = ColumnState::Lit;
                        }
                    }
                }
                JobOutput::Meshed(m, quads) => {
                    self.in_flight.remove(&Target::Mesh(m));
                    if inside(
                        ColumnPos {
                            x: m.x * 2,
                            z: m.z * 2,
                        },
                        keep + 1,
                    ) {
                        self.meshes.insert(m, quads);
                    }
                }
            }
        }
    }

    fn schedule(&mut self, viewer: &Viewer, map: &WorldMap, pool: &JobPool, tick_start: Instant) {
        // 3. Submit new work by priority.
        let center = viewer.column();
        let r = self.config.radius;
        let inside =
            |c: ColumnPos, lim: i32| (c.x - center.x).abs() <= lim && (c.z - center.z).abs() <= lim;
        let scan_start = Instant::now();
        let mut candidates: Vec<(f64, Target)> = Vec::new();
        for x in center.x - r - 2..=center.x + r + 2 {
            for z in center.z - r - 2..=center.z + r + 2 {
                let c = ColumnPos { x, z };
                let p = viewer.column_priority(c);
                if map.column(c).is_none() {
                    candidates.push((p, Target::Generate(c)));
                } else if inside(c, r + 1)
                    && !self.lit.contains(&c)
                    && neighbors(c).all(|n| map.column(n).is_some())
                {
                    candidates.push((p, Target::Light(c)));
                }
            }
        }
        let b = map.bounds();
        let (my0, my1) = (
            b.min_section_y.div_euclid(2),
            (b.min_section_y + b.section_count as i32 - 1).div_euclid(2),
        );
        for mx in (center.x - r).div_euclid(2)..=(center.x + r).div_euclid(2) {
            for mz in (center.z - r).div_euclid(2)..=(center.z + r).div_euclid(2) {
                // The 2×2 core columns must be lit and the 4×4 around generated.
                let core_lit = (0..2).all(|i| {
                    (0..2).all(|k| {
                        self.lit.contains(&ColumnPos {
                            x: mx * 2 + i,
                            z: mz * 2 + k,
                        })
                    })
                });
                if !core_lit {
                    continue;
                }
                let blocks = (-1..3).all(|i| {
                    (-1..3).all(|k| {
                        map.column(ColumnPos {
                            x: mx * 2 + i,
                            z: mz * 2 + k,
                        })
                        .is_some()
                    })
                });
                if !blocks {
                    continue;
                }
                for my in my0..=my1 {
                    let m = MeshChunkPos {
                        x: mx,
                        y: my,
                        z: mz,
                    };
                    if self.meshes.contains_key(&m) {
                        continue;
                    }
                    // All-air core: no geometry, no job (sky above the terrain).
                    let empty = (0..2).all(|i| {
                        (0..2).all(|k| {
                            (0..2).all(|j| {
                                map.column(ColumnPos {
                                    x: mx * 2 + i,
                                    z: mz * 2 + k,
                                })
                                .and_then(|c| c.section(my * 2 + j))
                                .is_none_or(Section::all_air)
                            })
                        })
                    });
                    if empty {
                        self.meshes.insert(m, MeshChunkQuads::default());
                    } else {
                        candidates.push((viewer.mesh_priority(m), Target::Mesh(m)));
                    }
                }
            }
        }
        candidates.retain(|(_, t)| !self.in_flight.contains_key(t));
        candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
        self.stats.max_scan = self.stats.max_scan.max(scan_start.elapsed());
        for (score, target) in candidates {
            if self.in_flight.len() >= self.config.max_in_flight
                || tick_start.elapsed() >= self.config.tick_budget
            {
                break;
            }
            let t = Instant::now();
            // Pool class by distance, not by job kind: near work of every kind
            // runs before far work of any kind (meshes must not wait for the
            // whole radius to generate).
            let class = if score < NEAR_BLOCKS {
                Priority::Visible
            } else {
                Priority::Prefetch
            };
            self.submit(target, map, pool, class);
            self.stats.max_submit = self.stats.max_submit.max(t.elapsed());
        }
    }

    fn submit(&mut self, target: Target, map: &WorldMap, pool: &JobPool, class: Priority) {
        let token = CancelToken::new();
        let registry = Arc::clone(&self.registry);
        match target {
            Target::Generate(c) => {
                let (pipeline, ctx) = (Arc::clone(&self.pipeline), Arc::clone(&self.context));
                pool.submit(class, token.clone(), move |_| {
                    let column = pipeline.generate(&ctx, c).into_column(&registry);
                    column.map_or(JobOutput::Failed(target), JobOutput::Generated)
                });
            }
            Target::Light(c) => {
                // A private 3×3 map: light_column needs the neighbors' blocks only.
                let mut local = WorldMap::new(Arc::clone(&registry), map.bounds());
                for n in std::iter::once(c).chain(neighbors(c)) {
                    if let Some(col) = map.column(n) {
                        let _ = local.insert_column(col.clone());
                    }
                }
                pool.submit(class, token.clone(), move |_| {
                    let props = props_from_registry(&registry);
                    let mut lights = WorldLight::new();
                    if light_column(&local, &mut lights, c, &props).is_err() {
                        return JobOutput::Failed(target);
                    }
                    let b = local.bounds();
                    let sections = (0..b.section_count as i32)
                        .filter_map(|i| {
                            let sp = SectionPos {
                                x: c.x,
                                y: b.min_section_y + i,
                                z: c.z,
                            };
                            lights.get(sp).map(|l| (sp, l.clone()))
                        })
                        .collect();
                    JobOutput::Lit(c, sections)
                });
            }
            Target::Mesh(m) => {
                let neighborhood = SectionNeighborhood::from_fn(m, |sp| {
                    map.column(ColumnPos { x: sp.x, z: sp.z })
                        .and_then(|col| col.section(sp.y))
                        .cloned()
                        .unwrap_or_else(Section::new)
                });
                pool.submit(class, token.clone(), move |_| {
                    let mut builder = MeshChunkBuilder::new();
                    let quads = builder.build(&neighborhood, |b| registry.props(b).transparent);
                    JobOutput::Meshed(m, quads)
                });
            }
        }
        self.in_flight.insert(target, token);
        self.stats.submitted += 1;
    }
}

fn neighbors(c: ColumnPos) -> impl Iterator<Item = ColumnPos> {
    (-1..=1)
        .flat_map(move |dx| {
            (-1..=1).map(move |dz| ColumnPos {
                x: c.x + dx,
                z: c.z + dz,
            })
        })
        .filter(move |&n| n != c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearer_first_and_ahead_before_behind() {
        let still = Viewer {
            pos: [8.0, 64.0, 8.0],
            velocity: [0.0; 3],
        };
        assert!(
            still.column_priority(ColumnPos { x: 1, z: 0 })
                < still.column_priority(ColumnPos { x: 3, z: 0 })
        );
        assert_eq!(
            still.column_priority(ColumnPos { x: 2, z: 0 }),
            still.column_priority(ColumnPos { x: -2, z: 0 }),
            "no direction when standing"
        );
        let running = Viewer {
            velocity: [8.0, 0.0, 0.0],
            ..still
        };
        let ahead = running.column_priority(ColumnPos { x: 2, z: 0 });
        let behind = running.column_priority(ColumnPos { x: -2, z: 0 });
        assert!(ahead < behind);
        let flying = Viewer {
            velocity: [40.0, 0.0, 0.0],
            ..still
        };
        assert!(
            flying.column_priority(ColumnPos { x: 4, z: 0 })
                < running.column_priority(ColumnPos { x: 4, z: 0 }),
            "faster movement prefers the front more"
        );
        // Height: the mesh chunk at the viewer's level beats one far above.
        assert!(
            still.mesh_priority(MeshChunkPos { x: 0, y: 2, z: 0 })
                < still.mesh_priority(MeshChunkPos { x: 0, y: 6, z: 0 })
        );
    }
}
