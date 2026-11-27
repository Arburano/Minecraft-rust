//! Player intents on the wire ↔ physics input (T2-02).
//!
//! The client and the server run the same `sim::physics::step` on the same
//! inputs, so both convert through these functions: the client quantizes its
//! own yaw exactly as the server will see it, and both sides end up with
//! bit-identical positions as long as they see the same blocks.

use protocol::{Angle16, Buttons, Correction, EntityUpdate, Intent};
use sim::physics::{PlayerBody, PlayerInput};

/// The intent the client sends for one tick. `fly_speed_steps` are the
/// mouse-wheel notches since the previous tick. `state_hash` is filled in by
/// the client after it simulated the tick ([`state_hash`]).
#[must_use]
pub fn intent(
    tick: u32,
    input: &PlayerInput,
    yaw: f64,
    pitch: f64,
    fly_speed_steps: i32,
) -> Intent {
    let mut b = 0u16;
    let mut set = |on: bool, bit: u16| {
        if on {
            b |= bit;
        }
    };
    set(input.forward > 0.0, Buttons::FORWARD);
    set(input.forward < 0.0, Buttons::BACK);
    set(input.strafe < 0.0, Buttons::LEFT);
    set(input.strafe > 0.0, Buttons::RIGHT);
    set(input.jump, Buttons::JUMP);
    set(input.descend, Buttons::SNEAK);
    set(input.sprint, Buttons::SPRINT);
    set(input.toggle_fly, Buttons::FLY_TOGGLE);
    Intent {
        tick,
        buttons: Buttons(b),
        yaw: Angle16::from_radians(yaw as f32),
        pitch: Angle16::from_radians(pitch as f32),
        fly_speed_steps: fly_speed_steps.clamp(-128, 127) as i8,
        state_hash: 0,
    }
}

/// FNV-1a over the exact simulated state: client and server compare these
/// after every tick; a mismatch makes the server send a [`Correction`].
#[must_use]
pub fn state_hash(body: &PlayerBody, fly_speed: f64) -> u32 {
    let mut h = 0x811c_9dc5u32;
    let mut eat = |bytes: &[u8]| {
        for &b in bytes {
            h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
        }
    };
    for v in body.pos.to_array().iter().chain(&body.vel.to_array()) {
        eat(&v.to_bits().to_le_bytes());
    }
    eat(&[flags(body)]);
    eat(&fly_speed.to_bits().to_le_bytes());
    // Ability state only when there is some: profiles without abilities hash
    // exactly as before (T1-18).
    if !body.abilities.is_empty() {
        eat(&body.abilities.to_bytes());
    }
    h
}

/// The state bits of a body ([`EntityUpdate`] flags).
#[must_use]
pub fn flags(body: &PlayerBody) -> u8 {
    let mut f = 0;
    if body.on_ground {
        f |= EntityUpdate::ON_GROUND;
    }
    if body.flying {
        f |= EntityUpdate::FLYING;
    }
    if body.in_fluid {
        f |= EntityUpdate::IN_FLUID;
    }
    f
}

/// A correction carrying `body` after `tick`.
#[must_use]
pub fn correction(tick: u32, body: &PlayerBody, fly_speed: f64) -> Correction {
    Correction {
        tick,
        pos: body.pos.to_array(),
        vel: body.vel.to_array(),
        flags: flags(body),
        fly_speed,
        abilities: {
            let mut out = [0u8; protocol::ABILITY_STATE_BYTES];
            out[..sim::abilities::STATE_BYTES].copy_from_slice(&body.abilities.to_bytes());
            out
        },
    }
}

/// The body and fly speed a correction describes.
#[must_use]
pub fn corrected(c: &Correction) -> (PlayerBody, f64) {
    (
        PlayerBody {
            pos: glam::DVec3::from_array(c.pos),
            vel: glam::DVec3::from_array(c.vel),
            on_ground: c.flags & EntityUpdate::ON_GROUND != 0,
            flying: c.flags & EntityUpdate::FLYING != 0,
            in_fluid: c.flags & EntityUpdate::IN_FLUID != 0,
            abilities: sim::abilities::AbilityState::from_bytes(
                &c.abilities[..sim::abilities::STATE_BYTES],
            )
            .unwrap_or_default(),
        },
        c.fly_speed,
    )
}

/// The yaw both sides simulate with: the wire value.
#[must_use]
pub fn wire_yaw(intent: &Intent) -> f64 {
    f64::from(intent.yaw.to_radians())
}

/// Physics input of an intent; `fly_speed` is the multiplier after applying
/// the intent's wheel steps ([`apply_fly_steps`]).
#[must_use]
pub fn physics_input(intent: &Intent, fly_speed: f64) -> PlayerInput {
    let b = intent.buttons;
    let axis = |p: bool, n: bool| f64::from(u8::from(p)) - f64::from(u8::from(n));
    PlayerInput {
        forward: axis(b.has(Buttons::FORWARD), b.has(Buttons::BACK)),
        strafe: axis(b.has(Buttons::RIGHT), b.has(Buttons::LEFT)),
        jump: b.has(Buttons::JUMP),
        sprint: b.has(Buttons::SPRINT),
        descend: b.has(Buttons::SNEAK),
        toggle_fly: b.has(Buttons::FLY_TOGGLE),
        fly_speed,
        yaw: wire_yaw(intent),
    }
}

/// The fly speed multiplier after `steps` wheel notches (×1.25 each),
/// clamped to the movement profile's range — the same rule as single-player.
#[must_use]
pub fn apply_fly_steps(fly_speed: f64, steps: i8, range: [f64; 2]) -> f64 {
    if steps == 0 {
        return fly_speed;
    }
    (fly_speed * 1.25f64.powi(i32::from(steps))).clamp(range[0], range[1])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buttons_round_trip_through_the_wire() {
        let input = PlayerInput {
            forward: 1.0,
            strafe: -1.0,
            jump: true,
            sprint: true,
            descend: false,
            toggle_fly: true,
            fly_speed: 0.0,
            yaw: 0.0,
        };
        let i = intent(7, &input, 1.0, -0.4, 2);
        let back = physics_input(&i, 1.5);
        assert_eq!(back.forward, 1.0);
        assert_eq!(back.strafe, -1.0);
        assert!(back.jump && back.sprint && back.toggle_fly && !back.descend);
        assert_eq!(back.fly_speed, 1.5);
        assert!((back.yaw - 1.0).abs() < 1e-4);
        assert_eq!(i.fly_speed_steps, 2);
        assert_eq!(apply_fly_steps(1.0, 2, [0.25, 8.0]), 1.5625);
        assert_eq!(apply_fly_steps(4.0, 10, [0.25, 8.0]), 8.0);
    }

    #[test]
    fn corrections_round_trip_and_hashes_see_every_bit() {
        let body = PlayerBody {
            pos: glam::DVec3::new(1.5, 70.25, -3.0),
            vel: glam::DVec3::new(0.1, -0.0784, 0.0),
            on_ground: true,
            in_fluid: false,
            flying: false,
            abilities: Default::default(),
        };
        let c = correction(9, &body, 1.25);
        let (back, fly) = corrected(&c);
        assert_eq!(back, body);
        assert_eq!(state_hash(&back, fly), state_hash(&body, 1.25));
        let mut nudged = body;
        nudged.pos.x = f64::from_bits(body.pos.x.to_bits() + 1);
        assert_ne!(state_hash(&nudged, 1.25), state_hash(&body, 1.25));
        let mut flying = body;
        flying.flying = true;
        assert_ne!(state_hash(&flying, 1.25), state_hash(&body, 1.25));
        // Ability state travels and is hashed.
        let mut dashing = body;
        dashing.abilities.cooldown[0] = 17;
        dashing.abilities.box_from = 2;
        let c = correction(9, &dashing, 1.25);
        assert_eq!(corrected(&c).0, dashing);
        assert_ne!(state_hash(&dashing, 1.25), state_hash(&body, 1.25));
        assert!(correction(9, &body, 1.25).abilities.iter().all(|&b| b == 0));
    }
}
