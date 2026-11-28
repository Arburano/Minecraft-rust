//! Column encoding for the wire and the node's column cache.

use std::sync::Arc;

use protocol::{ChunkStream, ColumnPayload, Message, ProtocolError};
use world::storage::ColumnRecord;
use world::{Column, Registry};

/// A column as a storage record (light is computed on clients, ADR-0011).
#[must_use]
pub fn record(col: &Column) -> ColumnRecord {
    ColumnRecord {
        pos: col.pos(),
        bounds: col.bounds(),
        sections: col.sections().to_vec(),
        light: Vec::new(),
        entities: Vec::new(),
    }
}

/// The encoded `ChunkStream` message of a column.
pub fn encode(col: &Column) -> Result<Arc<[u8]>, ProtocolError> {
    let payload = ColumnPayload::encode(&record(col))?;
    Ok(ChunkStream::Column(payload).encode()?.into())
}

/// A live column back from its encoded message (server block numbering).
#[must_use]
pub fn decode(bytes: &[u8], registry: &Registry) -> Option<Column> {
    let ChunkStream::Column(payload) = ChunkStream::decode(bytes).ok()?;
    let record = payload.decode().ok()?;
    Column::from_sections(record.pos, record.bounds, record.sections, registry)
}
