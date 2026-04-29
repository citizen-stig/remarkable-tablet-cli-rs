//! Top-level block envelope — spec §4 (header) and §5 (block types).

use std::io::Read;

use crate::error::ParseError;

/// File header magic — exactly 43 bytes (spec §1).
pub const FILE_HEADER: &[u8; 43] = b"reMarkable .lines file, version=6          ";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum BlockType {
    MigrationInfo = 0x00,
    SceneTree = 0x01,
    TreeNode = 0x02,
    SceneGlyphItem = 0x03,
    SceneGroupItem = 0x04,
    SceneLineItem = 0x05,
    SceneTextItem = 0x06,
    RootText = 0x07,
    SceneTombstoneItem = 0x08,
    AuthorIds = 0x09,
    PageInfo = 0x0A,
    SceneInfo = 0x0D,
}

#[derive(Debug, Clone, Copy)]
pub struct BlockHeader {
    pub length: u32,
    pub min_version: u8,
    pub current_version: u8,
    pub block_type: u8,
}

pub fn read_file_header<R: Read>(_r: &mut R) -> Result<(), ParseError> {
    todo!("spec §1 — validate the 43-byte magic")
}

pub fn read_block_header<R: Read>(_r: &mut R) -> Result<BlockHeader, ParseError> {
    todo!("spec §4 — block header layout")
}
