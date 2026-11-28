//! The entities channel: per-client server datagrams (`networking-protocol.md`,
//! "Entity deltas"), sent every server tick.
//!
//! Each datagram carries:
//! - **the local player's reconciliation data**: the newest intent tick the
//!   server applied, and — only when the client's predicted state hash did
//!   not match the server's — the authoritative state to roll back to
//!   ([`Correction`]);
//! - **entity deltas**: the server keeps, per client, the newest snapshot the
//!   client acknowledged (`InputDatagram::ack_snapshot`) and sends only what
//!   changed since then. A known entity's movement is a small [`Step8`]; a new
//!   or far-moved one gets a full [`RelPos`] relative to `origin`.

use bitcode::{Decode, Encode};

use crate::{Angle8, BlockXyz, MAX_DATAGRAM, Message, NetId, ProtocolError, RelPos, check};

/// Most entity updates in one datagram. When more changed, the server sends
/// the nearest ones; the rest follow in later snapshots.
pub const MAX_ENTITY_UPDATES: usize = 128;

/// Body orientation of an entity.
#[derive(Encode, Decode, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Look {
    /// Body yaw.
    pub yaw: Angle8,
    /// Pitch.
    pub pitch: Angle8,
    /// Head yaw.
    pub head_yaw: Angle8,
}

/// A movement since the baseline in 1/32 block, ±4 blocks per axis.
#[derive(Encode, Decode, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Step8 {
    /// Δx in 1/32 block.
    pub x: i8,
    /// Δy in 1/32 block.
    pub y: i8,
    /// Δz in 1/32 block.
    pub z: i8,
}

impl Step8 {
    /// The step between two absolute quantized positions, if it fits.
    #[must_use]
    pub fn between(from: [i32; 3], to: [i32; 3]) -> Option<Self> {
        let d = |i: usize| i8::try_from(i64::from(to[i]) - i64::from(from[i])).ok();
        Some(Self {
            x: d(0)?,
            y: d(1)?,
            z: d(2)?,
        })
    }

    /// `from` moved by this step.
    #[must_use]
    pub fn apply(self, from: [i32; 3]) -> [i32; 3] {
        [
            from[0].saturating_add(i32::from(self.x)),
            from[1].saturating_add(i32::from(self.y)),
            from[2].saturating_add(i32::from(self.z)),
        ]
    }
}

/// What changed about one entity since the baseline. `None` fields did not
/// change; a new entity has `kind`, `pos` and `look` set.
#[derive(Encode, Decode, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct EntityUpdate {
    /// Which entity.
    pub id: NetId,
    /// Entity type (0 = player; mod-defined types come in stage 4). Set when
    /// the entity first appears to this client.
    pub kind: Option<u16>,
    /// Absolute position relative to the datagram's origin (new entities and
    /// moves too large for `moved`).
    pub pos: Option<RelPos>,
    /// Movement since the baseline position.
    pub moved: Option<Step8>,
    /// Orientation.
    pub look: Option<Look>,
    /// State bits (see [`EntityUpdate::FLYING`] …).
    pub flags: Option<u8>,
}

impl EntityUpdate {
    /// Flag: standing on the ground.
    pub const ON_GROUND: u8 = 1 << 0;
    /// Flag: flying.
    pub const FLYING: u8 = 1 << 1;
    /// Flag: in a fluid.
    pub const IN_FLUID: u8 = 1 << 2;
}

/// The authoritative state of the local player after `tick`, sent when the
/// client's prediction for that tick differed (exact `f64` values, so the
/// client can replay its later intents bit for bit).
#[derive(Encode, Decode, Clone, Copy, Debug, Default, PartialEq)]
pub struct Correction {
    /// The intent tick this state follows.
    pub tick: u32,
    /// Feet position.
    pub pos: [f64; 3],
    /// Velocity, blocks per tick.
    pub vel: [f64; 3],
    /// [`EntityUpdate::ON_GROUND`], [`EntityUpdate::FLYING`], [`EntityUpdate::IN_FLUID`].
    pub flags: u8,
    /// Fly speed multiplier.
    pub fly_speed: f64,
    /// Movement-ability state (cooldowns, charges, box; `sim::abilities`,
    /// T1-18), zero-padded; all zero when the profile has no abilities.
    pub abilities: [u8; ABILITY_STATE_BYTES],
}

/// Bytes reserved for [`Correction::abilities`].
pub const ABILITY_STATE_BYTES: usize = 32;

/// One server datagram to one client.
#[derive(Encode, Decode, Clone, Debug, Default, PartialEq)]
pub struct EntityDatagram {
    /// Server tick of this snapshot.
    pub snapshot: u32,
    /// The acknowledged snapshot these changes are relative to; 0 = none.
    pub baseline: u32,
    /// The newest intent tick of this client the server has applied.
    pub ack_tick: Option<u32>,
    /// Set when the client's prediction diverged.
    pub correction: Option<Correction>,
    /// Origin of the absolute positions (near the receiving player).
    pub origin: BlockXyz,
    /// Changed or new entities.
    pub updates: Vec<EntityUpdate>,
    /// Entities that left this client's view or were removed.
    pub removed: Vec<NetId>,
}

impl Message for EntityDatagram {
    const MAX_LEN: usize = MAX_DATAGRAM;

    fn validate(&self) -> Result<(), ProtocolError> {
        check(self.snapshot > 0, "snapshot 0 is reserved")?;
        check(
            self.baseline < self.snapshot,
            "baseline must precede snapshot",
        )?;
        check(
            self.updates.len() + self.removed.len() <= MAX_ENTITY_UPDATES,
            "too many entity updates",
        )?;
        if let Some(c) = &self.correction {
            check(
                c.pos.iter().chain(&c.vel).all(|v| v.is_finite()) && c.fly_speed.is_finite(),
                "correction values",
            )?;
        }
        Ok(())
    }
}
