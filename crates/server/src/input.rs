//! Player intents on the wire ↔ physics input (T2-02).
//!
//! The client and the server run the same `sim::physics::step` on the same
//! inputs, so both convert through these functions: the client quantizes its
//! own yaw exactly as the server will see it, and both sides end up with
//! bit-identical positions as long as they see the same blocks.

use protocol::{Angle16, Buttons, Intent};
use sim::physics::PlayerInput;

/// The intent the client sends for one tick. `fly_speed_steps` are the
/// mouse-wheel notches since the previous tick.
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
    }
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
}
