//! Common CRDT-sequence-item envelope shared by block types 0x03-0x06 and
//! 0x08 — spec §6.1.

use crate::crdt::CrdtId;

#[derive(Debug, Clone)]
pub struct ItemEnvelope<V> {
    pub parent_id: CrdtId,
    pub item_id: CrdtId,
    pub left_id: CrdtId,
    pub right_id: CrdtId,
    pub deleted_length: u32,
    pub value: Option<V>,
}

/// Item-type discriminator inside the value sub-block — spec §6.2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ItemType {
    GlyphRange = 0x01,
    Group = 0x02,
    Line = 0x03,
    Text = 0x05,
}
