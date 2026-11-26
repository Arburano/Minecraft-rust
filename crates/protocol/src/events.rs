//! The events channel: reliable, ordered messages (chat, block edits, column
//! unloads). Block edits travel here rather than in input datagrams because
//! each one must arrive exactly once; T2-04 builds prediction and rollback on
//! the `seq` numbers.

use bitcode::{Decode, Encode};

use crate::{BlockXyz, ColumnXz, Message, ProtocolError, check, check_text};

/// Longest chat message, in bytes.
pub const MAX_CHAT_LEN: usize = 256;
/// Most block changes in one message.
pub const MAX_BLOCK_CHANGES: usize = 4096;

/// One changed block, with the state as a number of [`crate::RegistrySync`].
#[derive(Encode, Decode, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BlockChange {
    /// Where.
    pub pos: BlockXyz,
    /// The new state.
    pub state: u16,
}

/// Client → server events.
#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq)]
pub enum ClientEvent {
    /// A chat line.
    Chat {
        /// Text.
        text: String,
    },
    /// The player broke a block (predicted on the client).
    Dig {
        /// Client sequence number, echoed in [`ServerEvent::BlockAck`].
        seq: u32,
        /// The block.
        pos: BlockXyz,
    },
    /// The player placed a block (predicted on the client).
    Place {
        /// Client sequence number.
        seq: u32,
        /// The cell that receives the block.
        pos: BlockXyz,
        /// The placed state.
        state: u16,
    },
}

/// Server → client events.
#[derive(Encode, Decode, Clone, Debug, PartialEq, Eq)]
pub enum ServerEvent {
    /// A chat line.
    Chat {
        /// Sender name (empty for server messages).
        from: String,
        /// Text.
        text: String,
    },
    /// Authoritative block changes in loaded columns.
    BlockChanges(Vec<BlockChange>),
    /// The result of a client edit: rejected edits are rolled back.
    BlockAck {
        /// The client's sequence number.
        seq: u32,
        /// Whether the server applied it.
        accepted: bool,
    },
    /// The client may drop this column (out of view).
    ForgetColumn(ColumnXz),
}

impl Message for ClientEvent {
    const MAX_LEN: usize = 1024;

    fn validate(&self) -> Result<(), ProtocolError> {
        match self {
            Self::Chat { text } => check_text(text, MAX_CHAT_LEN, "chat text"),
            Self::Dig { .. } | Self::Place { .. } => Ok(()),
        }
    }
}

impl Message for ServerEvent {
    const MAX_LEN: usize = 64 * 1024;

    fn validate(&self) -> Result<(), ProtocolError> {
        match self {
            Self::Chat { from, text } => {
                check_text(from, crate::MAX_NAME_LEN, "chat sender")?;
                check_text(text, MAX_CHAT_LEN, "chat text")
            }
            Self::BlockChanges(c) => check(c.len() <= MAX_BLOCK_CHANGES, "too many block changes"),
            Self::BlockAck { .. } | Self::ForgetColumn(_) => Ok(()),
        }
    }
}
