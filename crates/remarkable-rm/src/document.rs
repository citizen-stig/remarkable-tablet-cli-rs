//! Top-level page representation, assembled from blocks per spec §10.

use std::collections::{HashMap, HashSet};

use crate::blocks::{BlockType, iter_blocks};
use crate::crdt::CrdtId;
use crate::error::ParseError;
use crate::primitives::Reader;
use crate::scene::items::{ItemType, read_scene_item};
use crate::scene::line::Line;
use crate::scene::text::{RootText, read_root_text_block};
use crate::scene::tree::{SceneTreeBlock, read_scene_tree_block, read_tree_node_block};

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
pub fn parse_page(bytes: &[u8]) -> Result<Page, ParseError> {
    let mut intermediate = Intermediate::default();

    for block in iter_blocks(bytes)? {
        let block = block?;
        let cur_version = block.header.current_version;
        let mut body = block.body;
        match block.header.kind() {
            Some(BlockType::SceneTree) => {
                intermediate.trees.push(read_scene_tree_block(&mut body)?);
            }
            Some(BlockType::TreeNode) => {
                let n = read_tree_node_block(&mut body)?;
                intermediate.nodes.push(NodeProps {
                    node_id: n.node_id,
                    label: n.label.value,
                    visible: n.visible.value,
                });
            }
            Some(BlockType::SceneGroupItem) => {
                // Parsed for forward-compat / format validation; the renderer
                // links lines directly to layers via `parent_id`, so the
                // group records aren't needed for assembly.
                let _ = read_scene_item(&mut body, ItemType::Group, |sub| sub.read_id(2))?;
            }
            Some(BlockType::SceneLineItem) => {
                let envelope = read_scene_item(&mut body, ItemType::Line, |sub| {
                    crate::scene::line::read_line(sub, cur_version)
                })?;
                if envelope.deleted_length == 0
                    && let Some(line) = envelope.value
                {
                    intermediate.lines.push(LineRecord {
                        parent_id: envelope.parent_id,
                        line,
                    });
                }
            }
            Some(BlockType::RootText) => {
                intermediate.text = Some(read_root_text_block(&mut body)?);
            }
            Some(BlockType::SceneInfo) => {
                intermediate.paper_size = read_paper_size(&mut body)?;
            }
            // MigrationInfo, AuthorIds, PageInfo, SceneGlyphItem,
            // SceneTextItem, SceneTombstoneItem, and unknown types are
            // consumed but not surfaced (renderer doesn't need them yet).
            _ => {}
        }
    }

    Ok(intermediate.assemble())
}

#[derive(Debug, Default)]
struct Intermediate {
    trees: Vec<SceneTreeBlock>,
    nodes: Vec<NodeProps>,
    lines: Vec<LineRecord>,
    text: Option<RootText>,
    paper_size: Option<(u32, u32)>,
}

#[derive(Debug)]
struct NodeProps {
    node_id: CrdtId,
    label: String,
    visible: bool,
}

#[derive(Debug)]
struct LineRecord {
    parent_id: CrdtId, // identifies the layer node this stroke belongs to
    line: Line,
}

impl Intermediate {
    fn assemble(self) -> Page {
        let Self {
            trees,
            nodes,
            lines,
            text,
            paper_size,
        } = self;

        let node_props: HashMap<CrdtId, (String, bool)> = nodes
            .into_iter()
            .map(|n| (n.node_id, (n.label, n.visible)))
            .collect();

        let mut lines_by_parent: HashMap<CrdtId, Vec<Line>> = HashMap::new();
        for lr in lines {
            lines_by_parent.entry(lr.parent_id).or_default().push(lr.line);
        }

        // Process SceneTreeBlocks in insertion order, deduplicating by
        // `tree_id` (later `is_update` blocks already overrode parent fields
        // since we assigned in the original loop, but for layer detection we
        // only need first-occurrence order).
        let mut layers = Vec::new();
        let mut seen: HashSet<CrdtId> = HashSet::new();
        for tree in trees {
            if !seen.insert(tree.tree_id) {
                continue;
            }
            if tree.parent_id != CrdtId::ROOT {
                continue;
            }
            let id = tree.tree_id;
            let (name, visible) = node_props
                .get(&id)
                .cloned()
                .unwrap_or_else(|| (String::new(), true));
            let layer_lines = lines_by_parent.remove(&id).unwrap_or_default();
            layers.push(Layer {
                node_id: id,
                name,
                visible,
                lines: layer_lines,
            });
        }

        Page {
            layers,
            text,
            paper_size,
        }
    }
}

/// Extract the (optional) `paper_size` from a `SceneInfoBlock` body — spec
/// §5.6. Every field after `current_layer` is optional, detected by remaining
/// bytes in the block.
fn read_paper_size(body: &mut Reader<'_>) -> Result<Option<(u32, u32)>, ParseError> {
    let _current_layer = body.read_lww_id(1)?;
    if body.is_eof() {
        return Ok(None);
    }
    let _background_visible = body.read_lww_bool(2)?;
    if body.is_eof() {
        return Ok(None);
    }
    let _root_document_visible = body.read_lww_bool(3)?;
    if body.is_eof() {
        return Ok(None);
    }
    let mut sub = body.read_subblock(5)?;
    let width = sub.read_u32()?;
    let height = sub.read_u32()?;
    Ok(Some((width, height)))
}
