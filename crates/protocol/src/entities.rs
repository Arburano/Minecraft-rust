//! The entities channel: per-client entity deltas as datagrams
//! (`networking-protocol.md`, "Entity deltas").
//!
//! The server keeps, per client, the newest snapshot the client acknowledged
//! (`InputDatagram::ack_snapshot`) and sends only what changed since then.
//! Positions are 1/32 block relative to `origin`, angles 8 bits.

use bitcode::{Decode, Encode};

use crate::{Angle8, BlockXyz, MAX_DATAGRAM, Message, NetId, ProtocolError, RelPos, check};

/// Most entity updates in one datagram. The server splits larger deltas over
/// several datagrams with the same `snapshot`.
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

/// What changed about one entity. `None` fields did not change since the
/// baseline; a new entity (or a full snapshot) has every field set.
#[derive(Encode, Decode, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct EntityUpdate {
    /// Which entity.
    pub id: NetId,
    /// Entity type, as a number in the server's entity table (stage 4; mods
    /// define types). Set when the entity first appears to this client.
    pub kind: Option<u16>,
    /// Position relative to the datagram's origin.
    pub pos: Option<RelPos>,
    /// Orientation.
    pub look: Option<Look>,
    /// State bits (on ground, sneaking, flying…), defined with the entity types.
    pub flags: Option<u8>,
}

/// One entity datagram.
#[derive(Encode, Decode, Clone, Debug, Default, PartialEq, Eq)]
pub struct EntityDatagram {
    /// Server tick of this snapshot.
    pub snapshot: u32,
    /// The acknowledged snapshot these changes are relative to; 0 = full state.
    pub baseline: u32,
    /// Origin of the relative positions (near the receiving player).
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
        )
    }
}
