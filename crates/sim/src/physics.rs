//! Player physics (`simulation-and-ai.md`, "Player physics"): an AABB moved
//! per fixed 20 TPS tick with axis-wise sweeping against block collision
//! boxes (Y, then X, then Z), step-up, jumping, ground/air friction and a
//! swimming placeholder, plus the abilities of the movement profile
//! ([`crate::abilities`]). All tunable numbers come from the build's movement
//! provider (`movement/profile.toml`, the `movement` slot; ADR-0013), not
//! from the engine.
//!
//! The same code runs on the client (prediction) and later on the server
//! (authority); it is deterministic: the same inputs from the same state give
//! bit-identical positions (plain `f64` arithmetic, fixed evaluation order).

use glam::DVec3;
use serde::Deserialize;
use world::coords::BlockPos;
use world::registry::Collision;
use world::{BlockStateId, Registry, WorldMap};

use crate::abilities::{self, Ability, AbilityState, Condition, Effect};

/// Movement numbers, per tick (blocks, blocks/tick, blocks/tick²).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PhysicsConfig {
    /// Collision box width (x and z).
    pub width: f64,
    /// Collision box height.
    pub height: f64,
    /// Eye height above the feet.
    pub eye_height: f64,
    /// Highest ledge the player walks onto without jumping.
    pub step_height: f64,
    /// Downward acceleration in air.
    pub gravity: f64,
    /// Vertical velocity multiplier per tick in air.
    pub vertical_drag: f64,
    /// Vertical velocity of a jump.
    pub jump_velocity: f64,
    /// Horizontal forward boost of a sprint jump.
    pub sprint_jump_boost: f64,
    /// Horizontal velocity multiplier per tick on the ground.
    pub ground_friction: f64,
    /// Horizontal velocity multiplier per tick in air.
    pub air_friction: f64,
    /// Horizontal acceleration on the ground.
    pub walk_accel: f64,
    /// Horizontal acceleration in air.
    pub air_accel: f64,
    /// Acceleration multiplier while sprinting.
    pub sprint_multiplier: f64,
    /// Velocity multiplier per tick in a fluid (all axes).
    pub fluid_drag: f64,
    /// Acceleration in a fluid.
    pub fluid_accel: f64,
    /// Downward acceleration in a fluid.
    pub fluid_gravity: f64,
    /// Upward acceleration while holding jump in a fluid.
    pub swim_up: f64,
    /// Block interaction distance from the eye.
    pub reach: f64,
    /// Whether the player may fly (double-tap jump).
    pub can_fly: bool,
    /// Horizontal acceleration while flying (at fly speed ×1).
    pub fly_accel: f64,
    /// Vertical acceleration while flying up or down (at fly speed ×1).
    pub fly_vertical_accel: f64,
    /// Vertical velocity multiplier per tick while flying (no gravity).
    pub fly_vertical_drag: f64,
    /// Acceleration multiplier while sprinting in flight.
    pub fly_sprint_multiplier: f64,
    /// Lowest and highest fly speed multiplier the mouse wheel can set.
    pub fly_speed_range: [f64; 2],
    /// Blocks the player swims in (block ids, e.g. `core:water`).
    pub swim_blocks: Vec<String>,
    /// Extra moves built from engine primitives (`[[abilities]]`).
    pub abilities: Vec<Ability>,
}

impl Default for PhysicsConfig {
    /// Fallback numbers; the `core` datapack file sets the real ones.
    fn default() -> Self {
        Self {
            width: 0.6,
            height: 1.8,
            eye_height: 1.62,
            step_height: 0.6,
            gravity: 0.08,
            vertical_drag: 0.98,
            jump_velocity: 0.42,
            sprint_jump_boost: 0.2,
            ground_friction: 0.546,
            air_friction: 0.91,
            walk_accel: 0.1,
            air_accel: 0.02,
            sprint_multiplier: 1.3,
            fluid_drag: 0.8,
            fluid_accel: 0.02,
            fluid_gravity: 0.02,
            swim_up: 0.04,
            reach: 4.5,
            can_fly: true,
            fly_accel: 0.05,
            fly_vertical_accel: 0.15,
            fly_vertical_drag: 0.6,
            fly_sprint_multiplier: 2.0,
            fly_speed_range: [0.25, 8.0],
            swim_blocks: Vec::new(),
            abilities: Vec::new(),
        }
    }
}

/// Physics configuration errors.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PhysicsConfigError {
    /// The TOML could not be parsed.
    #[error("physics config: {0}")]
    Parse(String),
    /// A swim block is not in the registry.
    #[error("physics config: unknown block {0}")]
    UnknownBlock(String),
    /// An ability is invalid.
    #[error("movement profile: {0}")]
    Ability(String),
}

impl PhysicsConfig {
    /// Parses the datapack file; missing fields keep the defaults.
    pub fn from_toml(source: &str) -> Result<Self, PhysicsConfigError> {
        let cfg: Self =
            toml::from_str(source).map_err(|e| PhysicsConfigError::Parse(e.to_string()))?;
        abilities::validate(&cfg.abilities).map_err(PhysicsConfigError::Ability)?;
        Ok(cfg)
    }

    /// The profile in a dimension whose `player_gravity_scale` is `scale`
    /// (T4-02): gravity in air and in fluids scales; ×1 changes nothing.
    #[must_use]
    pub fn with_gravity_scale(mut self, scale: f64) -> Self {
        if scale != 1.0 {
            self.gravity *= scale;
            self.fluid_gravity *= scale;
        }
        self
    }

    /// Box height and eye height for an ability state.
    fn box_size(&self, state: &AbilityState) -> (f64, f64) {
        let i = usize::from(state.box_from);
        if i > 0
            && let Some(a) = self.abilities.get(i - 1)
            && let Some(&Effect::BoxSize { height, eye_height }) = a
                .effects
                .iter()
                .find(|e| matches!(e, Effect::BoxSize { .. }))
        {
            return (height, eye_height);
        }
        (self.height, self.eye_height)
    }

    /// Resolves `swim_blocks` against a registry.
    pub fn swim_ids(&self, registry: &Registry) -> Result<Vec<BlockStateId>, PhysicsConfigError> {
        self.swim_blocks
            .iter()
            .map(|n| {
                registry
                    .state_id(n)
                    .ok_or_else(|| PhysicsConfigError::UnknownBlock(n.clone()))
            })
            .collect()
    }
}

/// What physics asks about blocks at integer cells.
pub trait BlockQuery {
    /// Whether the cell has a full collision box. Unloaded cells must report
    /// `true` so the player never falls into an unloaded column.
    fn collides(&self, pos: BlockPos) -> bool;
    /// Whether the cell is a fluid the player swims in.
    fn is_fluid(&self, pos: BlockPos) -> bool;
}

/// [`BlockQuery`] over the client's world map.
pub struct MapBlocks<'a> {
    /// The world.
    pub map: &'a WorldMap,
    /// Swim blocks ([`PhysicsConfig::swim_ids`]).
    pub swim: &'a [BlockStateId],
}

impl BlockQuery for MapBlocks<'_> {
    fn collides(&self, pos: BlockPos) -> bool {
        match self.map.get_block(pos) {
            // Below and above the world: solid below, open above.
            None if self.map.column(world::ColumnPos::of_block(pos)).is_some() => {
                pos.y < self.map.bounds().min_y()
            }
            None => true,
            // Model shapes are not resolved yet: a `Model` collision counts as
            // a full cube (T1-12 report).
            Some(id) => self.map.registry().props(id).collision != Collision::None,
        }
    }

    fn is_fluid(&self, pos: BlockPos) -> bool {
        self.map
            .get_block(pos)
            .is_some_and(|id| self.swim.contains(&id))
    }
}

/// One tick of player intent.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerInput {
    /// Forward (+1) / backward (−1).
    pub forward: f64,
    /// Right (+1) / left (−1).
    pub strafe: f64,
    /// Jump held.
    pub jump: bool,
    /// Sprint held.
    pub sprint: bool,
    /// Descend held (flying: move down).
    pub descend: bool,
    /// Toggle flight this tick (double-tapped jump).
    pub toggle_fly: bool,
    /// Fly speed multiplier (mouse wheel); values ≤ 0 mean ×1.
    pub fly_speed: f64,
    /// View yaw in radians; 0 looks towards −Z, positive turns towards −X.
    pub yaw: f64,
}

/// Simulated player state.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerBody {
    /// Feet position: center of the box's bottom face.
    pub pos: DVec3,
    /// Velocity in blocks per tick.
    pub vel: DVec3,
    /// Standing on something after the last tick.
    pub on_ground: bool,
    /// Inside a swim block after the last tick.
    pub in_fluid: bool,
    /// Flying (no gravity; ends on landing).
    pub flying: bool,
    /// Ability cooldowns, charges, box and buttons (all zero without abilities).
    pub abilities: AbilityState,
}

/// An axis-aligned box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    /// Minimum corner.
    pub min: DVec3,
    /// Maximum corner.
    pub max: DVec3,
}

impl Aabb {
    fn offset(self, d: DVec3) -> Self {
        Self {
            min: self.min + d,
            max: self.max + d,
        }
    }

    /// Cells overlapping the box (half-open on the max side).
    fn cells(self) -> (BlockPos, BlockPos) {
        let lo = self.min.floor();
        let hi = (self.max - DVec3::splat(1e-9)).floor();
        (
            BlockPos::new(lo.x as i32, lo.y as i32, lo.z as i32),
            BlockPos::new(hi.x as i32, hi.y as i32, hi.z as i32),
        )
    }
}

impl PlayerBody {
    /// The collision box.
    #[must_use]
    pub fn aabb(&self, cfg: &PhysicsConfig) -> Aabb {
        let h = cfg.width / 2.0;
        let (height, _) = cfg.box_size(&self.abilities);
        Aabb {
            min: DVec3::new(self.pos.x - h, self.pos.y, self.pos.z - h),
            max: DVec3::new(self.pos.x + h, self.pos.y + height, self.pos.z + h),
        }
    }

    /// The eye position.
    #[must_use]
    pub fn eye(&self, cfg: &PhysicsConfig) -> DVec3 {
        let (_, eye) = cfg.box_size(&self.abilities);
        self.pos + DVec3::new(0.0, eye, 0.0)
    }
}

/// Clips movement `d` along `axis` so `b` does not enter a colliding cell.
fn clip_axis(world: &impl BlockQuery, b: Aabb, axis: usize, d: f64) -> f64 {
    if d == 0.0 {
        return 0.0;
    }
    let mut swept = b;
    if d > 0.0 {
        swept.max[axis] += d;
    } else {
        swept.min[axis] += d;
    }
    let (lo, hi) = swept.cells();
    let mut d = d;
    for y in lo.y..=hi.y {
        for z in lo.z..=hi.z {
            for x in lo.x..=hi.x {
                let p = BlockPos::new(x, y, z);
                if !world.collides(p) {
                    continue;
                }
                let cmin = DVec3::new(f64::from(x), f64::from(y), f64::from(z));
                let cmax = cmin + DVec3::ONE;
                // Must overlap strictly on the two other axes.
                let overlaps = (0..3)
                    .filter(|&a| a != axis)
                    .all(|a| b.max[a] > cmin[a] && b.min[a] < cmax[a]);
                if !overlaps {
                    continue;
                }
                if d > 0.0 && b.max[axis] <= cmin[axis] {
                    d = d.min(cmin[axis] - b.max[axis]);
                } else if d < 0.0 && b.min[axis] >= cmax[axis] {
                    d = d.max(cmax[axis] - b.min[axis]);
                }
            }
        }
    }
    d
}

/// Moves `b` by `d` axis by axis (Y, X, Z); returns the allowed movement.
fn sweep(world: &impl BlockQuery, mut b: Aabb, d: DVec3) -> DVec3 {
    let mut out = DVec3::ZERO;
    for axis in [1, 0, 2] {
        let m = clip_axis(world, b, axis, d[axis]);
        out[axis] = m;
        let mut o = DVec3::ZERO;
        o[axis] = m;
        b = b.offset(o);
    }
    out
}

fn touches_fluid(world: &impl BlockQuery, b: Aabb) -> bool {
    let (lo, hi) = b.cells();
    (lo.y..=hi.y).any(|y| {
        (lo.z..=hi.z).any(|z| (lo.x..=hi.x).any(|x| world.is_fluid(BlockPos::new(x, y, z))))
    })
}

fn collides_any(world: &impl BlockQuery, b: Aabb) -> bool {
    let (lo, hi) = b.cells();
    (lo.y..=hi.y).any(|y| {
        (lo.z..=hi.z).any(|z| (lo.x..=hi.x).any(|x| world.collides(BlockPos::new(x, y, z))))
    })
}

/// A wall within 0.05 blocks on a horizontal side.
fn touching_wall(world: &impl BlockQuery, b: Aabb) -> bool {
    [0usize, 2].iter().any(|&axis| {
        [0.05, -0.05]
            .iter()
            .any(|&d| clip_axis(world, b, axis, d) != d)
    })
}

/// Runs the profile's abilities for this tick (after the intent, before the
/// movement); returns the gravity factor for the tick.
fn run_abilities(
    body: &mut PlayerBody,
    input: &PlayerInput,
    cfg: &PhysicsConfig,
    world: &impl BlockQuery,
    (forward, right): (DVec3, DVec3),
) -> f64 {
    let mut s = body.abilities;
    let now = (u8::from(input.jump) * abilities::JUMP)
        | (u8::from(input.sprint) * abilities::SPRINT)
        | (u8::from(input.descend) * abilities::DESCEND);
    if body.on_ground || body.in_fluid {
        s.used = [0; abilities::MAX_ABILITIES];
    }
    for c in &mut s.cooldown {
        *c = c.saturating_sub(1);
    }
    let moving = input.forward != 0.0 || input.strafe != 0.0;
    let mut wall = None;
    let (mut active, mut gravity, mut want_box) = (0u8, 1.0, None);
    for (i, a) in cfg.abilities.iter().enumerate() {
        if !a.trigger.holds(now, s.prev_buttons)
            || s.cooldown[i] > 0
            || (a.charges > 0 && s.used[i] >= a.charges)
        {
            continue;
        }
        let ok = a.conditions.iter().all(|c| match c {
            Condition::OnGround => body.on_ground,
            Condition::InAir => !body.on_ground && !body.in_fluid && !body.flying,
            Condition::InFluid => body.in_fluid,
            Condition::TouchingWall => {
                *wall.get_or_insert_with(|| touching_wall(world, body.aabb(cfg)))
            }
            Condition::Moving => moving,
            Condition::Falling => body.vel.y < 0.0,
            Condition::NotFlying => !body.flying,
        });
        if !ok {
            continue;
        }
        active |= 1 << i;
        s.cooldown[i] = a.cooldown;
        if a.charges > 0 {
            s.used[i] += 1;
        }
        for e in &a.effects {
            match *e {
                Effect::Impulse {
                    forward: f,
                    up,
                    right: r,
                } => body.vel += forward * f + DVec3::new(0.0, up, 0.0) + right * r,
                Effect::SetVelocity {
                    vertical,
                    forward: speed,
                } => {
                    if let Some(h) = speed {
                        body.vel = DVec3::new(forward.x * h, body.vel.y, forward.z * h);
                    }
                    if let Some(v) = vertical {
                        body.vel.y = v;
                    }
                }
                Effect::HoldSurface { slide, climb } => {
                    if climb > 0.0 {
                        body.vel.y = climb;
                    } else if body.vel.y < -slide {
                        body.vel.y = -slide;
                    }
                }
                Effect::BoxSize { .. } => want_box = Some(i),
                Effect::GravityScale { scale } => gravity *= scale,
            }
        }
    }
    match want_box {
        Some(i) => s.box_from = i as u8 + 1,
        None if s.box_from != 0 => {
            // Back to the normal box only when it fits.
            let mut normal = *body;
            normal.abilities.box_from = 0;
            if !collides_any(world, normal.aabb(cfg)) {
                s.box_from = 0;
            }
        }
        None => {}
    }
    s.active = active;
    s.prev_buttons = now;
    body.abilities = s;
    gravity
}

/// Advances the player by one tick.
pub fn step(
    body: &mut PlayerBody,
    input: &PlayerInput,
    cfg: &PhysicsConfig,
    world: &impl BlockQuery,
) {
    let mut b = body.aabb(cfg);
    body.in_fluid = touches_fluid(world, b);

    // Intent: horizontal acceleration along the view yaw.
    let (s, c) = input.yaw.sin_cos();
    let forward = DVec3::new(-s, 0.0, -c);
    let right = DVec3::new(c, 0.0, -s);
    let mut wish = forward * input.forward.clamp(-1.0, 1.0) + right * input.strafe.clamp(-1.0, 1.0);
    if wish.length_squared() > 1.0 {
        wish = wish.normalize();
    }
    let sprint = if input.sprint && input.forward > 0.0 {
        cfg.sprint_multiplier
    } else {
        1.0
    };
    if input.toggle_fly {
        body.flying = cfg.can_fly && !body.flying;
        if body.flying {
            // Start from a hover: leftover gravity must not land the player at once.
            body.vel.y = 0.0;
        }
    }
    let fly_speed = if input.fly_speed > 0.0 {
        input
            .fly_speed
            .clamp(cfg.fly_speed_range[0], cfg.fly_speed_range[1])
    } else {
        1.0
    };
    let accel = if body.flying {
        let boost = if input.sprint && input.forward > 0.0 {
            cfg.fly_sprint_multiplier
        } else {
            1.0
        };
        cfg.fly_accel * fly_speed * boost
    } else if body.in_fluid {
        cfg.fluid_accel
    } else if body.on_ground {
        cfg.walk_accel * sprint
    } else {
        cfg.air_accel * sprint
    };
    body.vel += wish * accel;
    if body.flying {
        let v = cfg.fly_vertical_accel * fly_speed;
        if input.jump {
            body.vel.y += v;
        }
        if input.descend {
            body.vel.y -= v;
        }
    } else if input.jump {
        if body.in_fluid {
            body.vel.y += cfg.swim_up;
        } else if body.on_ground {
            body.vel.y = cfg.jump_velocity;
            if sprint > 1.0 {
                body.vel += forward * cfg.sprint_jump_boost;
            }
        }
    }

    // Abilities (none in the default profile: this block is skipped).
    let mut gravity_scale = 1.0;
    if !cfg.abilities.is_empty() {
        gravity_scale = run_abilities(body, input, cfg, world, (forward, right));
        b = body.aabb(cfg);
    }

    // Move with collisions and step-up.
    let want = body.vel;
    let mut moved = sweep(world, b, want);
    let blocked_h = moved.x != want.x || moved.z != want.z;
    let falling_hit = want.y < 0.0 && moved.y != want.y;
    if blocked_h && (body.on_ground || falling_hit) && cfg.step_height > 0.0 {
        // Up by the step height, across, then back down.
        let up = sweep(world, b, DVec3::new(0.0, cfg.step_height, 0.0));
        let raised = b.offset(up);
        let across = sweep(world, raised, DVec3::new(want.x, 0.0, want.z));
        let shifted = raised.offset(across);
        let down = clip_axis(world, shifted, 1, -(up.y - want.y.min(0.0)));
        let stepped = up + across + DVec3::new(0.0, down, 0.0);
        let h = |v: DVec3| v.x * v.x + v.z * v.z;
        if h(stepped) > h(moved) {
            moved = stepped;
        }
    }
    body.pos += moved;
    body.on_ground = want.y < 0.0 && moved.y > want.y;
    if moved.x != want.x {
        body.vel.x = 0.0;
    }
    if moved.z != want.z {
        body.vel.z = 0.0;
    }
    if moved.y != want.y {
        body.vel.y = 0.0;
    }
    if body.flying && body.on_ground {
        // Landing ends the flight.
        body.flying = false;
    }

    // Gravity and friction for the next tick.
    if body.flying {
        body.vel.x *= cfg.air_friction;
        body.vel.z *= cfg.air_friction;
        body.vel.y *= cfg.fly_vertical_drag;
    } else if body.in_fluid {
        body.vel *= cfg.fluid_drag;
        body.vel.y -= cfg.fluid_gravity;
    } else {
        body.vel.y = (body.vel.y - cfg.gravity * gravity_scale) * cfg.vertical_drag;
        let f = if body.on_ground {
            cfg.ground_friction
        } else {
            cfg.air_friction
        };
        body.vel.x *= f;
        body.vel.z *= f;
    }
}

/// Render position between two ticks (`alpha` in `0..=1` of the way from
/// `prev` to `cur`), for drawing at the display rate.
#[must_use]
pub fn interpolate(prev: &PlayerBody, cur: &PlayerBody, alpha: f64) -> DVec3 {
    prev.pos.lerp(cur.pos, alpha.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A test world: solid below `floor` plus listed blocks and fluid cells.
    struct TestWorld {
        floor: i32,
        solid: Vec<BlockPos>,
        fluid: Vec<BlockPos>,
    }

    impl BlockQuery for TestWorld {
        fn collides(&self, p: BlockPos) -> bool {
            p.y < self.floor || self.solid.contains(&p)
        }
        fn is_fluid(&self, p: BlockPos) -> bool {
            self.fluid.contains(&p)
        }
    }

    fn flat() -> TestWorld {
        TestWorld {
            floor: 64,
            solid: Vec::new(),
            fluid: Vec::new(),
        }
    }

    fn run(body: &mut PlayerBody, input: PlayerInput, ticks: usize, w: &TestWorld) {
        let cfg = PhysicsConfig::default();
        for _ in 0..ticks {
            step(body, &input, &cfg, w);
        }
    }

    #[test]
    fn falls_and_lands_exactly_on_the_floor() {
        let mut b = PlayerBody {
            pos: DVec3::new(0.5, 80.0, 0.5),
            ..Default::default()
        };
        run(&mut b, PlayerInput::default(), 100, &flat());
        assert_eq!(b.pos.y, 64.0);
        assert!(b.on_ground);
        assert!(b.vel.y.abs() < 0.1);
    }

    #[test]
    fn jump_reaches_about_1_25_blocks() {
        let w = flat();
        let cfg = PhysicsConfig::default();
        let mut b = PlayerBody {
            pos: DVec3::new(0.5, 64.0, 0.5),
            on_ground: true,
            ..Default::default()
        };
        let mut top: f64 = 0.0;
        let jump = PlayerInput {
            jump: true,
            ..Default::default()
        };
        step(&mut b, &jump, &cfg, &w);
        for _ in 0..30 {
            step(&mut b, &PlayerInput::default(), &cfg, &w);
            top = top.max(b.pos.y - 64.0);
        }
        assert!((1.2..1.3).contains(&top), "jump height {top}");
        assert_eq!(b.pos.y, 64.0);
    }

    #[test]
    fn walking_speed_settles_near_4_3_blocks_per_second() {
        let mut b = PlayerBody {
            pos: DVec3::new(0.5, 64.0, 0.5),
            on_ground: true,
            ..Default::default()
        };
        let walk = PlayerInput {
            forward: 1.0,
            ..Default::default()
        };
        run(&mut b, walk, 60, &flat());
        let start = b.pos;
        run(&mut b, walk, 20, &flat());
        let speed = (b.pos - start).length();
        assert!((4.0..4.6).contains(&speed), "speed {speed} blocks/s");
        assert!(b.pos.z < 0.0, "yaw 0 walks towards -Z");
    }

    #[test]
    fn walls_stop_and_one_block_needs_a_jump() {
        let mut w = flat();
        for x in -2..=2 {
            w.solid.push(BlockPos::new(x, 64, -3));
        }
        let walk = PlayerInput {
            forward: 1.0,
            ..Default::default()
        };
        let mut b = PlayerBody {
            pos: DVec3::new(0.5, 64.0, 0.5),
            on_ground: true,
            ..Default::default()
        };
        run(&mut b, walk, 40, &w);
        assert_eq!(b.pos.z, -2.0 + 0.3, "stops against the block face");
        assert_eq!(b.pos.y, 64.0, "a full block is above the step height");
        // Hold jump until the player lands on top of the block.
        let jump = PlayerInput { jump: true, ..walk };
        let landed = (0..20).any(|_| {
            run(&mut b, jump, 1, &w);
            b.on_ground && b.pos.y == 65.0
        });
        assert!(landed, "jumped onto the block");
    }

    #[test]
    fn step_up_walks_onto_a_ledge_within_step_height() {
        let mut w = flat();
        for x in -2..=2 {
            for z in -9..=-3 {
                w.solid.push(BlockPos::new(x, 64, z));
            }
        }
        let cfg = PhysicsConfig {
            step_height: 1.0,
            ..PhysicsConfig::default()
        };
        let mut b = PlayerBody {
            pos: DVec3::new(0.5, 64.0, 0.5),
            on_ground: true,
            ..Default::default()
        };
        let walk = PlayerInput {
            forward: 1.0,
            ..Default::default()
        };
        for _ in 0..40 {
            step(&mut b, &walk, &cfg, &w);
        }
        assert_eq!(b.pos.y, 65.0);
        assert!(b.pos.z < -3.0);
    }

    #[test]
    fn swimming_sinks_slowly_and_jump_rises() {
        let mut w = flat();
        for y in 64..70 {
            for x in -2..=2 {
                for z in -2..=2 {
                    w.fluid.push(BlockPos::new(x, y, z));
                }
            }
        }
        let mut b = PlayerBody {
            pos: DVec3::new(0.5, 67.0, 0.5),
            ..Default::default()
        };
        run(&mut b, PlayerInput::default(), 5, &w);
        assert!(b.in_fluid);
        assert!(b.vel.y > -0.1, "terminal sink speed is low: {}", b.vel.y);
        let before = b.pos.y;
        run(
            &mut b,
            PlayerInput {
                jump: true,
                ..Default::default()
            },
            10,
            &w,
        );
        assert!(b.pos.y > before, "swims up");
    }

    #[test]
    fn deterministic_for_the_same_inputs() {
        let mut w = flat();
        for i in 0..40 {
            w.solid.push(BlockPos::new(
                (i * 7) % 11 - 5,
                64 + i % 2,
                (i * 5) % 13 - 6,
            ));
        }
        let script = |t: usize| PlayerInput {
            forward: if t % 50 < 40 { 1.0 } else { -0.5 },
            strafe: ((t / 17) % 3) as f64 - 1.0,
            jump: t % 23 == 0,
            sprint: t % 100 > 60,
            descend: t % 70 > 60,
            toggle_fly: t % 150 == 1,
            fly_speed: 1.0 + (t / 100) as f64 * 0.5,
            yaw: t as f64 * 0.037,
        };
        let simulate = || {
            let cfg = PhysicsConfig::default();
            let mut b = PlayerBody {
                pos: DVec3::new(0.5, 70.0, 0.5),
                ..Default::default()
            };
            let mut trace = Vec::new();
            for t in 0..600 {
                step(&mut b, &script(t), &cfg, &w);
                trace.push(b.pos.to_array().map(f64::to_bits));
            }
            trace
        };
        assert_eq!(simulate(), simulate());
    }

    #[test]
    fn flying_hovers_rises_sinks_and_lands() {
        let w = flat();
        let cfg = PhysicsConfig::default();
        let mut b = PlayerBody {
            pos: DVec3::new(0.5, 64.0, 0.5),
            on_ground: true,
            ..Default::default()
        };
        // Double-tap: jump, then toggle in the air.
        step(
            &mut b,
            &PlayerInput {
                jump: true,
                ..Default::default()
            },
            &cfg,
            &w,
        );
        step(
            &mut b,
            &PlayerInput {
                toggle_fly: true,
                ..Default::default()
            },
            &cfg,
            &w,
        );
        assert!(b.flying);
        for _ in 0..40 {
            step(&mut b, &PlayerInput::default(), &cfg, &w);
        }
        let hover = b.pos.y;
        for _ in 0..20 {
            step(&mut b, &PlayerInput::default(), &cfg, &w);
        }
        assert!((b.pos.y - hover).abs() < 1e-9, "no gravity while flying");
        assert!(hover > 64.0, "hovers where flight started: {hover}");
        run(
            &mut b,
            PlayerInput {
                jump: true,
                ..Default::default()
            },
            20,
            &w,
        );
        assert!(b.pos.y > hover + 5.0, "rises: {}", b.pos.y);
        run(
            &mut b,
            PlayerInput {
                descend: true,
                ..Default::default()
            },
            200,
            &w,
        );
        assert_eq!(b.pos.y, 64.0);
        assert!(!b.flying, "landing ends the flight");
    }

    #[test]
    fn fly_speed_multiplier_scales_horizontal_speed() {
        let w = flat();
        let speed = |mult: f64| {
            let mut b = PlayerBody {
                pos: DVec3::new(0.5, 80.0, 0.5),
                flying: true,
                ..Default::default()
            };
            let input = PlayerInput {
                forward: 1.0,
                fly_speed: mult,
                ..Default::default()
            };
            run(&mut b, input, 100, &w);
            let start = b.pos;
            run(&mut b, input, 20, &w);
            (b.pos - start).length()
        };
        let base = speed(1.0);
        assert!(
            (10.0..12.0).contains(&base),
            "fly speed ≈ 11 blocks/s: {base}"
        );
        assert!((speed(2.0) / base - 2.0).abs() < 0.01);
        assert!(
            (speed(100.0) / base - 8.0).abs() < 0.01,
            "clamped to the range"
        );
        let cfg = PhysicsConfig {
            can_fly: false,
            ..PhysicsConfig::default()
        };
        let mut b = PlayerBody::default();
        step(
            &mut b,
            &PlayerInput {
                toggle_fly: true,
                ..Default::default()
            },
            &cfg,
            &w,
        );
        assert!(!b.flying, "a profile can forbid flight");
    }

    #[test]
    fn config_parses_and_resolves() {
        let c = PhysicsConfig::from_toml("gravity = 0.1\nswim_blocks = [\"core:water\"]")
            .expect("parse");
        assert_eq!(c.gravity, 0.1);
        assert_eq!(c.width, 0.6);
        assert!(
            PhysicsConfig::from_toml("gravty = 1").is_err(),
            "typos are rejected"
        );
    }

    #[test]
    fn interpolation_is_linear() {
        let a = PlayerBody::default();
        let b = PlayerBody {
            pos: DVec3::new(2.0, 4.0, -2.0),
            ..Default::default()
        };
        assert_eq!(interpolate(&a, &b, 0.5), DVec3::new(1.0, 2.0, -1.0));
    }

    /// A fixed mixed-input trace (walk, sprint-jump, wall, step, fly, swim)
    /// hashed bit by bit: the default profile must give exactly the T1-12
    /// result. The golden value was computed with the code before abilities
    /// existed (T1-18 report).
    #[test]
    fn default_profile_trace_is_unchanged() {
        let mut w = flat();
        for x in -3..=3 {
            w.solid.push(BlockPos::new(x, 64, -6));
            w.solid.push(BlockPos::new(x, 64, -7));
            w.solid.push(BlockPos::new(x, 65, -7));
        }
        for z in 2..6 {
            w.fluid.push(BlockPos::new(0, 64, z));
            w.fluid.push(BlockPos::new(0, 65, z));
        }
        let cfg = PhysicsConfig::default();
        let mut b = PlayerBody {
            pos: DVec3::new(0.5, 70.0, 0.5),
            ..Default::default()
        };
        let mut h = 0xcbf2_9ce4_8422_2325u64;
        for t in 0..600u32 {
            let input = PlayerInput {
                forward: [1.0, 0.0, -1.0, 0.5][(t / 37 % 4) as usize],
                strafe: [0.0, 1.0, 0.0, -1.0][(t / 53 % 4) as usize],
                jump: t % 23 < 3 || (300..340).contains(&t),
                sprint: t % 90 < 45,
                descend: (340..360).contains(&t),
                toggle_fly: t == 299 || t == 420,
                fly_speed: if t > 300 { 1.5 } else { 0.0 },
                yaw: f64::from(t) * 0.031,
            };
            step(&mut b, &input, &cfg, &w);
            for v in b.pos.to_array().iter().chain(&b.vel.to_array()) {
                for byte in v.to_bits().to_le_bytes() {
                    h = (h ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3);
                }
            }
            let flags = u8::from(b.on_ground) | u8::from(b.in_fluid) << 1 | u8::from(b.flying) << 2;
            h = (h ^ u64::from(flags)).wrapping_mul(0x0100_0000_01b3);
        }
        assert_eq!(h, GOLDEN_TRACE, "got {h:#018x}");
    }

    const GOLDEN_TRACE: u64 = 0x1b52_92d4_94de_dd96;

    /// A test-only profile: the default numbers plus five abilities.
    const AGILE: &str = r#"
can_fly = false

[[abilities]]
id = "dash"
trigger = "sprint_pressed"
conditions = ["moving", "not_flying"]
cooldown = 20
animation = "dash"
effects = [{ kind = "impulse", forward = 1.2 }]

[[abilities]]
id = "double_jump"
trigger = "jump_pressed"
conditions = ["in_air"]
charges = 1
effects = [{ kind = "set_velocity", vertical = 0.42 }]

[[abilities]]
id = "wall_slide"
trigger = "jump_held"
conditions = ["in_air", "touching_wall", "falling"]
effects = [{ kind = "hold_surface", slide = 0.05 }]

[[abilities]]
id = "crawl"
trigger = "descend_held"
conditions = ["on_ground"]
effects = [{ kind = "box_size", height = 0.6, eye_height = 0.4 }]

[[abilities]]
id = "glide"
trigger = "descend_held"
conditions = ["in_air", "falling"]
effects = [{ kind = "gravity_scale", scale = 0.1 }]
"#;

    fn agile() -> PhysicsConfig {
        PhysicsConfig::from_toml(AGILE).expect("agile profile")
    }

    fn on_floor() -> PlayerBody {
        PlayerBody {
            pos: DVec3::new(0.5, 64.0, 0.5),
            on_ground: true,
            ..Default::default()
        }
    }

    #[test]
    fn dash_bursts_forward_once_per_cooldown() {
        let (cfg, w) = (agile(), flat());
        let walk = PlayerInput {
            forward: 1.0,
            ..Default::default()
        };
        let dash = PlayerInput {
            sprint: true,
            ..walk
        };
        let mut plain = on_floor();
        let mut dashed = on_floor();
        for t in 0..10 {
            step(&mut plain, &walk, &cfg, &w);
            // Sprint pressed on tick 0 and again on tick 2 (still cooling down).
            let input = if t == 0 || t == 2 { &dash } else { &walk };
            step(&mut dashed, input, &cfg, &w);
            if t == 0 {
                assert_eq!(dashed.abilities.active, 1, "dash active on the press");
            }
            if t == 2 {
                assert_eq!(dashed.abilities.active, 0, "cooling down");
            }
        }
        let gain = plain.pos.z - dashed.pos.z;
        assert!(gain > 1.0, "a dash moves farther: {gain}");
    }

    #[test]
    fn double_jump_once_until_landing() {
        let (cfg, w) = (agile(), flat());
        let mut b = on_floor();
        let jump = PlayerInput {
            jump: true,
            ..Default::default()
        };
        let none = PlayerInput::default();
        step(&mut b, &jump, &cfg, &w); // ground jump
        for _ in 0..4 {
            step(&mut b, &none, &cfg, &w);
        }
        step(&mut b, &jump, &cfg, &w); // air jump
        assert_eq!(b.abilities.active & 2, 2);
        let mut top: f64 = 0.0;
        for t in 0..40 {
            // A third press in the air does nothing.
            let input = if t == 3 { &jump } else { &none };
            step(&mut b, input, &cfg, &w);
            if t == 3 {
                assert_eq!(b.abilities.active & 2, 0, "no charge left");
            }
            top = top.max(b.pos.y - 64.0);
        }
        assert!(top > 1.6, "higher than one jump: {top}");
        assert!(b.on_ground);
        assert_eq!(b.abilities.used[1], 0, "landing restores the charge");
    }

    #[test]
    fn wall_slide_falls_slowly_only_at_a_wall() {
        let cfg = agile();
        let mut w = flat();
        w.floor = 0;
        for y in 0..80 {
            w.solid.push(BlockPos::new(0, y, -1));
        }
        let hold = PlayerInput {
            jump: true,
            ..Default::default()
        };
        let start = |x: f64| PlayerBody {
            pos: DVec3::new(x, 70.0, 0.3),
            ..Default::default()
        };
        let (mut at_wall, mut free) = (start(0.5), start(5.5));
        // The first tick is a press (no previous buttons); the double jump uses it.
        for _ in 0..20 {
            step(&mut at_wall, &hold, &cfg, &w);
            step(&mut free, &hold, &cfg, &w);
        }
        assert!(
            at_wall.vel.y >= -0.05 * cfg.vertical_drag - cfg.gravity,
            "{}",
            at_wall.vel.y
        );
        assert!(
            free.pos.y < at_wall.pos.y - 3.0,
            "free {} wall {}",
            free.pos.y,
            at_wall.pos.y
        );
    }

    #[test]
    fn crawl_fits_under_a_low_ceiling_and_stands_up_after() {
        let cfg = agile();
        let mut w = flat();
        // A 1-block-high tunnel from z = -1 to -12.
        for z in -12..=-2 {
            for x in -1..=1 {
                w.solid.push(BlockPos::new(x, 65, z));
            }
        }
        let walk = PlayerInput {
            forward: 1.0,
            ..Default::default()
        };
        let crawl = PlayerInput {
            descend: true,
            ..walk
        };
        let mut b = on_floor();
        for _ in 0..40 {
            step(&mut b, &walk, &cfg, &w);
        }
        assert!(
            b.pos.z > -1.8,
            "standing up, blocked at the tunnel: {}",
            b.pos.z
        );
        for _ in 0..20 {
            step(&mut b, &crawl, &cfg, &w);
        }
        assert!(b.pos.z < -3.0 && b.pos.z > -10.0, "crawled in: {}", b.pos.z);
        assert!((b.eye(&cfg).y - 64.4).abs() < 1e-9);
        // Releasing inside the tunnel keeps the small box (no room to stand).
        step(&mut b, &walk, &cfg, &w);
        assert_eq!(b.abilities.box_from, 4);
        for _ in 0..80 {
            step(&mut b, &walk, &cfg, &w);
        }
        assert!(b.pos.z < -12.5, "walked out: {}", b.pos.z);
        assert_eq!(b.abilities.box_from, 0, "stood up outside");
    }

    #[test]
    fn glide_slows_the_fall() {
        let (cfg, w) = (agile(), flat());
        let start = PlayerBody {
            pos: DVec3::new(0.5, 100.0, 0.5),
            ..Default::default()
        };
        let (mut glide, mut fall) = (start, start);
        let hold = PlayerInput {
            descend: true,
            ..Default::default()
        };
        for _ in 0..30 {
            step(&mut glide, &hold, &cfg, &w);
            step(&mut fall, &PlayerInput::default(), &cfg, &w);
        }
        assert!(
            glide.pos.y > fall.pos.y + 10.0,
            "glide {} fall {}",
            glide.pos.y,
            fall.pos.y
        );
    }

    #[test]
    fn profiles_are_checked() {
        let bad = |extra: &str| PhysicsConfig::from_toml(&format!("{AGILE}{extra}")).is_err();
        assert!(bad(
            "[[abilities]]\nid = \"dash\"\ntrigger = \"jump_held\"\neffects = [{ kind = \"gravity_scale\", scale = 0.5 }]\n"
        ));
        assert!(bad(
            "[[abilities]]\nid = \"x\"\ntrigger = \"jump_held\"\neffects = []\n"
        ));
        assert!(bad(
            "[[abilities]]\nid = \"y\"\ntrigger = \"teleport\"\neffects = [{ kind = \"gravity_scale\", scale = 0.5 }]\n"
        ));
        assert!(bad(
            "[[abilities]]\nid = \"z\"\ntrigger = \"jump_held\"\neffects = [{ kind = \"box_size\", height = 1.0, eye_height = 2.0 }]\n"
        ));
        let many: String = (0..4)
            .map(|i| format!("[[abilities]]\nid = \"m{i}\"\ntrigger = \"jump_held\"\neffects = [{{ kind = \"gravity_scale\", scale = 1.0 }}]\n"))
            .collect();
        assert!(bad(&many), "more than 8 abilities");
    }

    #[test]
    fn dimension_gravity_scales_the_fall() {
        let earth = PhysicsConfig::default();
        assert_eq!(earth.clone().with_gravity_scale(1.0), earth);
        let mars = earth.clone().with_gravity_scale(0.38);
        let w = flat();
        let start = PlayerBody {
            pos: DVec3::new(0.5, 80.0, 0.5),
            ..Default::default()
        };
        let (mut a, mut b) = (start, start);
        for _ in 0..10 {
            step(&mut a, &PlayerInput::default(), &earth, &w);
            step(&mut b, &PlayerInput::default(), &mars, &w);
        }
        assert!(
            b.pos.y > a.pos.y + 1.0,
            "slower fall: earth {} mars {}",
            a.pos.y,
            b.pos.y
        );
    }
}
