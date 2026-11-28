//! Movement abilities (ADR-0013, `docs/modding/movement-profiles.md`): extra
//! moves of a movement profile built from a few engine primitives, so most
//! abilities are "just a file".
//!
//! An ability is **active** on a tick when its trigger holds, all its
//! conditions hold, its cooldown is over and it has a charge left. Each active
//! tick applies its effects. Instant effects (impulse, set velocity) are
//! usually paired with a "pressed" trigger or a cooldown; continuous ones
//! (hold surface, box size, gravity scale) with a "held" trigger.
//!
//! All state lives in [`AbilityState`] inside the player body, so prediction,
//! reconciliation and saving cover it, and a profile without abilities runs
//! exactly the code it ran before abilities existed.

use serde::Deserialize;

/// Abilities per profile at most (the state is a fixed-size array).
pub const MAX_ABILITIES: usize = 8;

/// When an ability may start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// Jump went down this tick.
    JumpPressed,
    /// Jump is held.
    JumpHeld,
    /// Sprint went down this tick.
    SprintPressed,
    /// Sprint is held.
    SprintHeld,
    /// Descend (sneak) went down this tick.
    DescendPressed,
    /// Descend (sneak) is held.
    DescendHeld,
}

/// A state the player must be in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Condition {
    /// Standing on something.
    OnGround,
    /// Not on the ground, not in a fluid, not flying.
    InAir,
    /// Inside a swim block.
    InFluid,
    /// A wall within 0.05 blocks on a horizontal side.
    TouchingWall,
    /// Movement keys pressed.
    Moving,
    /// Moving down.
    Falling,
    /// Not flying.
    NotFlying,
}

/// One engine primitive.
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Effect {
    /// Adds velocity in the view frame (blocks per tick).
    Impulse {
        /// Along the view yaw.
        #[serde(default)]
        forward: f64,
        /// Up.
        #[serde(default)]
        up: f64,
        /// To the right of the view.
        #[serde(default)]
        right: f64,
    },
    /// Sets velocity components; absent ones are kept.
    SetVelocity {
        /// Vertical velocity.
        #[serde(default)]
        vertical: Option<f64>,
        /// Horizontal speed along the view yaw (sideways velocity cleared).
        #[serde(default)]
        forward: Option<f64>,
    },
    /// Holds against a surface: falling is limited to `slide`; with `climb`
    /// above 0, the player moves up at that speed instead.
    HoldSurface {
        /// Fastest fall while holding (blocks per tick, ≥ 0).
        #[serde(default)]
        slide: f64,
        /// Climb speed (blocks per tick); 0 = only slide.
        #[serde(default)]
        climb: f64,
    },
    /// A different collision box while active (crouch, crawl); the normal
    /// box returns only when there is room.
    BoxSize {
        /// Box height.
        height: f64,
        /// Eye height.
        eye_height: f64,
    },
    /// Multiplies gravity while active (glide, low-gravity jump).
    GravityScale {
        /// Factor.
        scale: f64,
    },
}

/// One ability of a movement profile.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ability {
    /// Id, unique in the profile (`dash`, `double_jump`).
    pub id: String,
    /// When it may start.
    pub trigger: Trigger,
    /// States that must all hold.
    #[serde(default)]
    pub conditions: Vec<Condition>,
    /// Ticks after an active tick before it may be active again.
    #[serde(default)]
    pub cooldown: u16,
    /// Activations until the player lands or enters a fluid; 0 = unlimited.
    #[serde(default)]
    pub charges: u8,
    /// Animation state the client plays while active (entity animation
    /// system, later).
    #[serde(default)]
    pub animation: String,
    /// What it does on an active tick.
    pub effects: Vec<Effect>,
}

/// Per-player ability state (inside [`crate::physics::PlayerBody`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct AbilityState {
    /// Ticks left before each ability may be active again.
    pub cooldown: [u16; MAX_ABILITIES],
    /// Charges used since the last landing.
    pub used: [u8; MAX_ABILITIES],
    /// Bit per ability: active on the last tick (animations, replication).
    pub active: u8,
    /// Buttons of the last tick (bit 0 jump, 1 sprint, 2 descend), for the
    /// "pressed" triggers.
    pub prev_buttons: u8,
    /// The ability whose box size is in effect, plus one; 0 = the normal box.
    pub box_from: u8,
}

/// Bytes of [`AbilityState::to_bytes`] (fits the 32 bytes the protocol
/// reserves in a correction).
pub const STATE_BYTES: usize = MAX_ABILITIES * 3 + 3;
const _: () = assert!(STATE_BYTES <= 32);

impl AbilityState {
    /// A fixed little-endian encoding (corrections, state hash, saving).
    #[must_use]
    pub fn to_bytes(&self) -> [u8; STATE_BYTES] {
        let mut out = [0u8; STATE_BYTES];
        for i in 0..MAX_ABILITIES {
            out[i * 2..i * 2 + 2].copy_from_slice(&self.cooldown[i].to_le_bytes());
            out[MAX_ABILITIES * 2 + i] = self.used[i];
        }
        out[MAX_ABILITIES * 3] = self.active;
        out[MAX_ABILITIES * 3 + 1] = self.prev_buttons;
        out[MAX_ABILITIES * 3 + 2] = self.box_from;
        out
    }

    /// Inverse of [`AbilityState::to_bytes`]; `None` for a wrong length.
    #[must_use]
    pub fn from_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() != STATE_BYTES {
            return None;
        }
        let mut s = Self::default();
        for i in 0..MAX_ABILITIES {
            s.cooldown[i] = u16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]]);
            s.used[i] = bytes[MAX_ABILITIES * 2 + i];
        }
        s.active = bytes[MAX_ABILITIES * 3];
        s.prev_buttons = bytes[MAX_ABILITIES * 3 + 1];
        s.box_from = bytes[MAX_ABILITIES * 3 + 2];
        Some(s)
    }

    /// Whether nothing is stored (a profile without abilities never changes it).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Button bits of [`AbilityState::prev_buttons`].
pub(crate) const JUMP: u8 = 1;
pub(crate) const SPRINT: u8 = 2;
pub(crate) const DESCEND: u8 = 4;

impl Trigger {
    pub(crate) fn holds(self, now: u8, prev: u8) -> bool {
        let pressed = |b: u8| now & b != 0 && prev & b == 0;
        match self {
            Self::JumpPressed => pressed(JUMP),
            Self::JumpHeld => now & JUMP != 0,
            Self::SprintPressed => pressed(SPRINT),
            Self::SprintHeld => now & SPRINT != 0,
            Self::DescendPressed => pressed(DESCEND),
            Self::DescendHeld => now & DESCEND != 0,
        }
    }
}

/// Checks a profile's abilities (count, ids, values).
pub(crate) fn validate(abilities: &[Ability]) -> Result<(), String> {
    if abilities.len() > MAX_ABILITIES {
        return Err(format!("at most {MAX_ABILITIES} abilities"));
    }
    for (i, a) in abilities.iter().enumerate() {
        if a.id.is_empty() || abilities[..i].iter().any(|b| b.id == a.id) {
            return Err(format!("ability id {:?} empty or duplicate", a.id));
        }
        if a.effects.is_empty() {
            return Err(format!("ability {}: no effects", a.id));
        }
        for e in &a.effects {
            let ok = match *e {
                Effect::Impulse { forward, up, right } => {
                    [forward, up, right].iter().all(|v| v.is_finite())
                }
                Effect::SetVelocity { vertical, forward } => {
                    vertical.is_none_or(f64::is_finite) && forward.is_none_or(f64::is_finite)
                }
                Effect::HoldSurface { slide, climb } => {
                    slide.is_finite() && slide >= 0.0 && climb.is_finite() && climb >= 0.0
                }
                Effect::BoxSize { height, eye_height } => {
                    height > 0.1 && eye_height > 0.0 && eye_height <= height
                }
                Effect::GravityScale { scale } => scale.is_finite() && scale >= 0.0,
            };
            if !ok {
                return Err(format!("ability {}: invalid {e:?}", a.id));
            }
        }
        if a.effects
            .iter()
            .filter(|e| matches!(e, Effect::BoxSize { .. }))
            .count()
            > 1
        {
            return Err(format!("ability {}: more than one box size", a.id));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_bytes_round_trip() {
        let mut s = AbilityState::default();
        assert!(s.is_empty());
        s.cooldown[3] = 513;
        s.used[7] = 2;
        s.active = 0b1001;
        s.prev_buttons = JUMP | DESCEND;
        s.box_from = 4;
        let b = s.to_bytes();
        assert_eq!(AbilityState::from_bytes(&b), Some(s));
        assert_eq!(AbilityState::from_bytes(&b[1..]), None);
    }

    #[test]
    fn triggers_see_edges() {
        assert!(Trigger::JumpPressed.holds(JUMP, 0));
        assert!(!Trigger::JumpPressed.holds(JUMP, JUMP));
        assert!(Trigger::JumpHeld.holds(JUMP, JUMP));
        assert!(Trigger::SprintPressed.holds(SPRINT | JUMP, JUMP));
        assert!(!Trigger::DescendHeld.holds(JUMP, DESCEND));
    }
}
