//! Tag-length-value reader — spec §3.

use std::io::Read;

use crate::error::ParseError;

/// Tag type discriminator (low nibble of the tag varuint).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TagType {
    Byte1 = 0x01,
    Byte4 = 0x04,
    Byte8 = 0x08,
    Length4 = 0x0C,
    Id = 0x0F,
}

#[derive(Debug, Clone, Copy)]
pub struct Tag {
    pub index: u32,
    pub tag_type: TagType,
}

pub fn read_tag<R: Read>(_r: &mut R) -> Result<Tag, ParseError> {
    todo!("spec §3 — tag encoding")
}

/// Read a length-prefixed sub-block at `index`, returning its raw bytes.
pub fn read_subblock<R: Read>(_r: &mut R, _index: u32) -> Result<Vec<u8>, ParseError> {
    todo!("spec §3 — sub-blocks")
}
