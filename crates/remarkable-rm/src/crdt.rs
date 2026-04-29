//! CRDT identifiers and last-writer-wins values — spec §2 (`CrdtId`) and §9.

use std::io::Read;

use crate::error::ParseError;

/// CRDT identifier, doubling as a logical timestamp.
///
/// `author` is the per-file author index; `seq` is that author's sequence
/// number. The pair `(0, 0)` is the start/end sentinel; `(0, 1)` is the
/// scene-tree root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CrdtId {
    pub author: u8,
    pub seq: u64,
}

impl CrdtId {
    pub const SENTINEL: Self = Self { author: 0, seq: 0 };
    pub const ROOT: Self = Self { author: 0, seq: 1 };
}

/// Last-writer-wins wrapper. On conflict, the value with the highest
/// `timestamp` wins (spec §9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LwwValue<T> {
    pub timestamp: CrdtId,
    pub value: T,
}

pub fn read_crdt_id<R: Read>(_r: &mut R) -> Result<CrdtId, ParseError> {
    todo!("spec §2 — CrdtId encoding")
}
