//! Simulated entities, simulation LOD and asynchronous pathfinding (T2-07,
//! `simulation-and-ai.md`).
//!
//! Only test entities exist here ([`Walker`]: a player-sized body that
//! wanders to random goals); mobs and their behaviors are gameplay in
//! `mods/core` (stage 4). What the engine provides and this module tests:
//!
//! - **Simulation LOD:** physics and path following run every tick for every
//!   live entity, so nothing visibly freezes; *decisions* (choosing a goal,
//!   asking for a path, giving up when stuck) run every tick within
//!   [`NEAR_BLOCKS`] of the nearest player and every 2nd / 4th / 8th tick
//!   farther away ([`decision_interval`]).
//! - **Asynchronous pathfinding:** a decision copies the walkability around
//!   the entity into a [`Grid`] (the job never touches the region's map), the
//!   node runs [`astar`] on the job pool, and the path is applied in a later
//!   tick. Each region may ask for at most [`MAX_PATH_REQUESTS_PER_TICK`] new
//!   paths per tick and have at most [`MAX_PATHS_IN_FLIGHT`] waiting.

use std::collections::{BinaryHeap, HashMap};

use bevy_ecs::prelude::Component;
use glam::DVec3;
use protocol::NetId;
use sim::physics::{PlayerBody, PlayerInput};
use world::coords::BlockPos;
use world::registry::Collision;
use world::{BlockStateId, Registry, WorldMap};

/// Full AI within this distance (blocks) of the nearest player.
pub const NEAR_BLOCKS: f64 = 48.0;
/// New path requests per region per tick at most.
pub const MAX_PATH_REQUESTS_PER_TICK: usize = 4;
/// Path requests a region may have waiting at most.
pub const MAX_PATHS_IN_FLIGHT: usize = 32;
/// Horizontal radius (blocks) of the area a path may use.
pub const PATH_RADIUS: i32 = 12;
/// Vertical half-height (blocks) of that area.
pub const PATH_HEIGHT: i32 = 6;
/// A* gives up after expanding this many cells.
pub const MAX_EXPANSIONS: usize = 4000;
/// The entity type number of test walkers on the wire (players are 0).
pub const WALKER_KIND: u16 = 1;
/// The datapack name of test walkers (`entities/limits.toml`).
pub const WALKER_NAME: &str = "test:walker";

/// Per-region limits of simulated entities by kind (`mods/core/data/
/// entities/limits.toml`, T2-10). Entities over the limit are kept stored
/// (dormant), not simulated, until there is room.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntityLimits {
    /// For kinds without an entry.
    #[serde(default)]
    pub default: KindLimit,
    /// By kind name.
    #[serde(default)]
    pub kind: std::collections::BTreeMap<String, KindLimit>,
}

/// The limit of one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KindLimit {
    /// Simulated entities of this kind per region at most.
    pub max_per_region: usize,
}

impl Default for KindLimit {
    fn default() -> Self {
        Self {
            max_per_region: 2000,
        }
    }
}

impl EntityLimits {
    /// Reads `entities/limits.toml` under a mod's data folder; a missing file
    /// gives the defaults.
    pub fn load(data: &std::path::Path) -> Result<Self, String> {
        match std::fs::read_to_string(data.join("entities/limits.toml")) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("entities/limits.toml: {e}")),
            Err(_) => Ok(Self::default()),
        }
    }

    /// The limit of a kind.
    #[must_use]
    pub fn of(&self, kind: &str) -> usize {
        self.kind
            .get(kind)
            .map_or(self.default.max_per_region, |k| k.max_per_region)
    }
}

/// Ticks between AI decisions at `distance` blocks from the nearest player:
/// 1 within [`NEAR_BLOCKS`], then 2, 4 and 8 (the table in
/// `simulation-and-ai.md`).
#[must_use]
pub fn decision_interval(distance: f64) -> u64 {
    if distance <= NEAR_BLOCKS {
        1
    } else if distance <= NEAR_BLOCKS * 2.0 {
        2
    } else if distance <= NEAR_BLOCKS * 3.0 {
        4
    } else {
        8
    }
}

/// A test entity that wanders around.
#[derive(Component, Clone, Debug)]
pub struct Walker {
    /// Network identity (also the replication key).
    pub net_id: NetId,
    /// Physical state (player-sized box, the player physics).
    pub body: PlayerBody,
    /// Facing.
    pub yaw: f64,
    /// Placed on the ground yet (spawned before its column was live).
    pub placed: bool,
    /// The path being followed (cells to stand in).
    pub path: Vec<BlockPos>,
    /// Next waypoint in `path`.
    pub next: usize,
    /// A path request is waiting for its job.
    pub waiting: bool,
    /// Tick of the waiting request (latency statistics).
    pub requested_at: u64,
    /// Tick of the next decision.
    pub next_decision: u64,
    /// Ticks without progress towards the next waypoint.
    pub stuck: u32,
    /// Distance to the next waypoint at the last tick.
    last_gap: f64,
    rng: u64,
}

impl Walker {
    /// A walker at `pos`; `placed: false` puts it on the ground once its
    /// column is live.
    #[must_use]
    pub fn new(net_id: NetId, pos: DVec3, placed: bool) -> Self {
        Self {
            net_id,
            body: PlayerBody {
                pos,
                ..PlayerBody::default()
            },
            yaw: 0.0,
            placed,
            path: Vec::new(),
            next: 0,
            waiting: false,
            requested_at: 0,
            next_decision: 0,
            stuck: 0,
            last_gap: f64::MAX,
            rng: net_id.0.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1,
        }
    }

    fn random(&mut self) -> u64 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    /// The cell the feet are in.
    #[must_use]
    pub fn cell(&self) -> BlockPos {
        BlockPos::new(
            self.body.pos.x.floor() as i32,
            (self.body.pos.y + 0.01).floor() as i32,
            self.body.pos.z.floor() as i32,
        )
    }

    /// A random goal within the path radius, around the current cell.
    pub fn pick_goal(&mut self) -> BlockPos {
        let c = self.cell();
        let span = (2 * (PATH_RADIUS - 2) + 1) as u64;
        let dx = (self.random() % span) as i32 - (PATH_RADIUS - 2);
        let dz = (self.random() % span) as i32 - (PATH_RADIUS - 2);
        BlockPos::new(c.x + dx, c.y, c.z + dz)
    }

    /// The input that follows the path this tick (no decision involved):
    /// walk towards the next waypoint, jump to climb, stop at the end.
    pub fn steer(&mut self) -> PlayerInput {
        while let Some(&w) = self.path.get(self.next) {
            let (dx, dz) = (
                f64::from(w.x) + 0.5 - self.body.pos.x,
                f64::from(w.z) + 0.5 - self.body.pos.z,
            );
            let gap = (dx * dx + dz * dz).sqrt();
            if gap < 0.35 && (f64::from(w.y) - self.body.pos.y).abs() < 1.0 {
                self.next += 1;
                self.last_gap = f64::MAX;
                continue;
            }
            if gap < self.last_gap - 0.01 {
                self.stuck = 0;
            } else {
                self.stuck += 1;
            }
            self.last_gap = gap;
            // yaw 0 looks towards −Z; positive turns towards −X.
            self.yaw = (-dx).atan2(-dz);
            return PlayerInput {
                forward: 1.0,
                jump: f64::from(w.y) > self.body.pos.y + 0.4 && self.body.on_ground,
                yaw: self.yaw,
                ..PlayerInput::default()
            };
        }
        PlayerInput {
            yaw: self.yaw,
            ..PlayerInput::default()
        }
    }

    /// Whether the path is done (or there is none).
    #[must_use]
    pub fn idle(&self) -> bool {
        self.next >= self.path.len()
    }
}

/// A copy of which cells are solid around a point, for one path job.
#[derive(Clone, Debug)]
pub struct Grid {
    /// The minimum corner.
    pub origin: BlockPos,
    /// Size in cells (x, y, z).
    pub size: [i32; 3],
    solid: Vec<bool>,
}

impl Grid {
    /// Copies the cells within [`PATH_RADIUS`] / [`PATH_HEIGHT`] of `center`.
    /// Unloaded cells count as solid (as for physics).
    #[must_use]
    pub fn snapshot(map: &WorldMap, registry: &Registry, center: BlockPos) -> Self {
        let origin = BlockPos::new(
            center.x - PATH_RADIUS,
            center.y - PATH_HEIGHT,
            center.z - PATH_RADIUS,
        );
        let size = [
            2 * PATH_RADIUS + 1,
            2 * PATH_HEIGHT + 1,
            2 * PATH_RADIUS + 1,
        ];
        let mut solid = Vec::with_capacity((size[0] * size[1] * size[2]) as usize);
        let mut props: HashMap<BlockStateId, bool> = HashMap::new();
        for y in 0..size[1] {
            for z in 0..size[2] {
                for x in 0..size[0] {
                    let p = BlockPos::new(origin.x + x, origin.y + y, origin.z + z);
                    let s = map.get_block(p).is_none_or(|b| {
                        *props
                            .entry(b)
                            .or_insert_with(|| registry.props(b).collision != Collision::None)
                    });
                    solid.push(s);
                }
            }
        }
        Self {
            origin,
            size,
            solid,
        }
    }

    /// A grid from a function (tests).
    #[must_use]
    pub fn from_fn(origin: BlockPos, size: [i32; 3], solid: impl Fn(BlockPos) -> bool) -> Self {
        let mut v = Vec::with_capacity((size[0] * size[1] * size[2]) as usize);
        for y in 0..size[1] {
            for z in 0..size[2] {
                for x in 0..size[0] {
                    v.push(solid(BlockPos::new(
                        origin.x + x,
                        origin.y + y,
                        origin.z + z,
                    )));
                }
            }
        }
        Self {
            origin,
            size,
            solid: v,
        }
    }

    fn solid(&self, p: BlockPos) -> bool {
        let (x, y, z) = (
            p.x - self.origin.x,
            p.y - self.origin.y,
            p.z - self.origin.z,
        );
        if x < 0 || y < 0 || z < 0 || x >= self.size[0] || y >= self.size[1] || z >= self.size[2] {
            return true;
        }
        self.solid[((y * self.size[2] + z) * self.size[0] + x) as usize]
    }

    /// A 1×2×1 body can stand with its feet in `p`.
    #[must_use]
    pub fn standable(&self, p: BlockPos) -> bool {
        !self.solid(p)
            && !self.solid(BlockPos::new(p.x, p.y + 1, p.z))
            && self.solid(BlockPos::new(p.x, p.y - 1, p.z))
    }
}

/// A path request from a region's decision.
#[derive(Clone, Debug)]
pub struct PathRequest {
    /// The entity.
    pub net_id: NetId,
    /// Where it stands.
    pub start: BlockPos,
    /// Where it wants to go.
    pub goal: BlockPos,
    /// The walkability around it.
    pub grid: Grid,
    /// Whether a player is near (job priority).
    pub near: bool,
}

/// A* over standable cells: 4 horizontal neighbors, stepping up or down one
/// block (up needs head room above the start). Returns the cells after
/// `start` up to the reachable cell nearest the goal, or `None` if not even
/// one step is possible.
#[must_use]
pub fn astar(grid: &Grid, start: BlockPos, goal: BlockPos) -> Option<Vec<BlockPos>> {
    #[derive(PartialEq, Eq)]
    struct Open(u32, u32, BlockPos);
    impl Ord for Open {
        fn cmp(&self, o: &Self) -> std::cmp::Ordering {
            // Min-heap on f, then g; then position for determinism.
            o.0.cmp(&self.0)
                .then(o.1.cmp(&self.1))
                .then((o.2.x, o.2.y, o.2.z).cmp(&(self.2.x, self.2.y, self.2.z)))
        }
    }
    impl PartialOrd for Open {
        fn partial_cmp(&self, o: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(o))
        }
    }
    let h = |p: BlockPos| {
        ((p.x - goal.x).abs() + (p.z - goal.z).abs()) as u32 * 2 + (p.y - goal.y).unsigned_abs()
    };
    let mut open = BinaryHeap::new();
    let mut came: HashMap<BlockPos, BlockPos> = HashMap::new();
    let mut cost: HashMap<BlockPos, u32> = HashMap::new();
    cost.insert(start, 0);
    open.push(Open(h(start), 0, start));
    let mut best = (h(start), start);
    let mut expanded = 0;
    while let Some(Open(_, g, p)) = open.pop() {
        if p == goal {
            best = (0, p);
            break;
        }
        if g > *cost.get(&p).unwrap_or(&u32::MAX) {
            continue;
        }
        expanded += 1;
        if expanded > MAX_EXPANSIONS {
            break;
        }
        for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
            for dy in [0, 1, -1] {
                let n = BlockPos::new(p.x + dx, p.y + dy, p.z + dz);
                if !grid.standable(n) {
                    continue;
                }
                if dy == 1 && grid.solid(BlockPos::new(p.x, p.y + 2, p.z)) {
                    continue; // no room to jump
                }
                if dy == -1 && grid.solid(BlockPos::new(n.x, n.y + 2, n.z)) {
                    continue; // no room to step down into
                }
                let ng = g + 2 + u32::from(dy != 0);
                if ng < *cost.get(&n).unwrap_or(&u32::MAX) {
                    cost.insert(n, ng);
                    came.insert(n, p);
                    let hn = h(n);
                    if hn < best.0 {
                        best = (hn, n);
                    }
                    open.push(Open(ng + hn, ng, n));
                }
                break; // one height per direction: the first that works
            }
        }
    }
    let mut end = best.1;
    if end == start {
        return None;
    }
    let mut path = vec![end];
    while let Some(&prev) = came.get(&end) {
        if prev == start {
            break;
        }
        path.push(prev);
        end = prev;
    }
    path.reverse();
    Some(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat_with(wall: impl Fn(BlockPos) -> bool) -> Grid {
        Grid::from_fn(BlockPos::new(-10, 0, -10), [21, 8, 21], |p| {
            p.y == 0 || wall(p)
        })
    }

    #[test]
    fn limits_parse_and_default() {
        let l: EntityLimits = toml::from_str(
            "[default]\nmax_per_region = 10\n[kind.\"test:walker\"]\nmax_per_region = 3\n",
        )
        .expect("toml");
        assert_eq!(l.of(WALKER_NAME), 3);
        assert_eq!(l.of("core:item"), 10);
        assert_eq!(EntityLimits::default().of("x"), 2000);
        let core = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../mods/core/data");
        assert_eq!(
            EntityLimits::load(&core).expect("core").of(WALKER_NAME),
            1000
        );
    }

    #[test]
    fn lod_table() {
        assert_eq!(decision_interval(10.0), 1);
        assert_eq!(decision_interval(48.0), 1);
        assert_eq!(decision_interval(60.0), 2);
        assert_eq!(decision_interval(120.0), 4);
        assert_eq!(decision_interval(500.0), 8);
    }

    #[test]
    fn straight_and_around_a_wall() {
        let open = flat_with(|_| false);
        let p = astar(&open, BlockPos::new(0, 1, 0), BlockPos::new(5, 1, 0)).expect("path");
        assert_eq!(p.len(), 5);
        assert_eq!(*p.last().expect("end"), BlockPos::new(5, 1, 0));
        // A wall at x = 2 from z = −5 to 5, two blocks high.
        let wall = flat_with(|p| p.x == 2 && (-5..=5).contains(&p.z) && p.y <= 2);
        let p = astar(&wall, BlockPos::new(0, 1, 0), BlockPos::new(5, 1, 0)).expect("path");
        assert_eq!(*p.last().expect("end"), BlockPos::new(5, 1, 0));
        assert!(p.iter().all(|c| !(c.x == 2 && (-5..=5).contains(&c.z))));
        assert!(p.len() > 5, "a detour");
    }

    #[test]
    fn climbs_one_block_but_not_two() {
        let step = flat_with(|p| p.x >= 3 && p.y == 1);
        let p = astar(&step, BlockPos::new(0, 1, 0), BlockPos::new(5, 2, 0)).expect("path");
        assert_eq!(*p.last().expect("end"), BlockPos::new(5, 2, 0));
        let cliff = flat_with(|p| p.x >= 3 && p.y <= 2);
        let p = astar(&cliff, BlockPos::new(0, 1, 0), BlockPos::new(5, 3, 0));
        // Unreachable: the path ends at the foot of the cliff, nearest the goal.
        let end = *p.expect("partial").last().expect("end");
        assert_eq!(end.x, 2);
    }

    #[test]
    fn walker_follows_its_path() {
        let mut w = Walker::new(NetId(1), DVec3::new(0.5, 1.0, 0.5), true);
        w.path = vec![BlockPos::new(0, 1, -1), BlockPos::new(1, 1, -1)];
        let i = w.steer();
        assert_eq!(i.forward, 1.0);
        assert!(i.yaw.abs() < 1e-9, "−Z is yaw 0");
        w.body.pos = DVec3::new(0.5, 1.0, -0.5);
        let i = w.steer();
        assert_eq!(w.next, 1);
        assert!(
            (i.yaw + std::f64::consts::FRAC_PI_2).abs() < 1e-9,
            "+X is yaw −π/2"
        );
        w.body.pos = DVec3::new(1.5, 1.0, -0.5);
        let i = w.steer();
        assert!(w.idle() && i.forward == 0.0);
    }
}
