//! Top-level page representation, assembled from blocks per spec §10.

use crate::crdt::CrdtId;
use crate::error::ParseError;
use crate::scene::line::Line;
use crate::scene::text::RootText;

#[derive(Debug, Clone, Default)]
pub struct Page {
    pub layers: Vec<Layer>,
    pub text: Option<RootText>,
    /// Optional, present in firmware 3.14+ via `SceneInfoBlock` (spec §5.6).
    pub paper_size: Option<(u32, u32)>,
}

#[derive(Debug, Clone)]
pub struct Layer {
    pub node_id: CrdtId,
    pub name: String,
    pub visible: bool,
    pub lines: Vec<Line>,
}

/// Parse one full `.rm` v6 file into a [`Page`]. Top-level entry point for
/// downstream renderers.
pub fn parse_page(_bytes: &[u8]) -> Result<Page, ParseError> {
    todo!("spec §10 — assemble Page from sequential block reads")
}
