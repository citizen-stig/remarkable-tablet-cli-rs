//! Integration tests against real `.rm` v6 fixtures captured from a
//! reMarkable 2 tablet (see `tests/fixtures/README.md` for ground truth).

use std::collections::BTreeSet;

use remarkable_rm::{BlockType, CrdtId, Pen, PenColor, parse_page};

const SMOKE: &[u8] = include_bytes!("fixtures/smoke.rm");
const PENS_SMALL: &[u8] = include_bytes!("fixtures/pens-small.rm");
const EDITS: &[u8] = include_bytes!("fixtures/edits.rm");
const LAYERS: &[u8] = include_bytes!("fixtures/layers.rm");

#[test]
fn smoke_minimum_viable_file() {
    let page = parse_page(SMOKE).unwrap();
    assert_eq!(page.paper_size, Some((1404, 1872)));
    assert_eq!(page.layers.len(), 1);
    let layer = &page.layers[0];
    assert_eq!(layer.name, "Layer 1");
    assert!(layer.visible);
    assert_eq!(layer.lines.len(), 3);
    for line in &layer.lines {
        assert!(
            !line.points.is_empty(),
            "every stroke has at least one point"
        );
    }
}

#[test]
fn pens_small_covers_all_v2_tools() {
    let page = parse_page(PENS_SMALL).unwrap();
    assert_eq!(page.layers.len(), 1);
    let lines = &page.layers[0].lines;
    assert_eq!(lines.len(), 9);

    // Each of the 9 strokes uses a distinct pen — exactly the 9 v2 tools.
    let tools: BTreeSet<Pen> = lines.iter().map(|l| l.tool).collect();
    let expected: BTreeSet<Pen> = [
        Pen::PaintbrushV2,
        Pen::MechanicalPencilV2,
        Pen::PencilV2,
        Pen::BallpointV2,
        Pen::MarkerV2,
        Pen::FinelinerV2,
        Pen::HighlighterV2,
        Pen::Calligraphy,
        Pen::Shader,
    ]
    .into_iter()
    .collect();
    assert_eq!(tools, expected);

    // Five distinct colors are exercised: Black, Blue, Gray, Highlight, Red.
    let colors: BTreeSet<PenColor> = lines.iter().map(|l| l.color).collect();
    let expected_colors: BTreeSet<PenColor> = [
        PenColor::Black,
        PenColor::Blue,
        PenColor::Gray,
        PenColor::Highlight,
        PenColor::Red,
    ]
    .into_iter()
    .collect();
    assert_eq!(colors, expected_colors);
}

#[test]
fn edits_filters_out_tombstoned_strokes() {
    let page = parse_page(EDITS).unwrap();
    assert_eq!(page.layers.len(), 1);
    // Ground truth: 8 strokes remain after some were erased. Tombstoned line
    // items are dropped during assembly, so erased strokes do not appear.
    assert_eq!(page.layers[0].lines.len(), 8);
}

#[test]
fn layers_yields_three_named_layers() {
    let page = parse_page(LAYERS).unwrap();
    assert_eq!(page.paper_size, Some((1404, 1872)));
    assert_eq!(page.layers.len(), 3);
    assert_eq!(page.layers[0].name, "Layer 1");
    assert_eq!(page.layers[1].name, "Layer 2");
    assert_eq!(page.layers[2].name, "Layer 3");
    assert_eq!(page.layers[0].lines.len(), 2);
    assert_eq!(page.layers[1].lines.len(), 3);
    assert_eq!(page.layers[2].lines.len(), 4);
    for layer in &page.layers {
        assert!(layer.visible);
    }
}

#[test]
fn fixtures_have_no_invalid_pen_values() {
    for bytes in [SMOKE, PENS_SMALL, EDITS, LAYERS] {
        let page = parse_page(bytes).unwrap();
        for layer in &page.layers {
            for line in &layer.lines {
                // Implicit: parsing succeeded → all pen/color enum values
                // are valid. This test exists to lock that contract in.
                let _ = line.tool;
                let _ = line.color;
            }
        }
    }
}

#[test]
fn nested_groups_flatten_in_crdt_order() {
    let layer_node_id = CrdtId { author: 1, seq: 10 };
    let group_node_id = CrdtId { author: 1, seq: 20 };
    let layer_item_id = CrdtId { author: 1, seq: 29 };
    let a_id = CrdtId { author: 1, seq: 30 };
    let group_item_id = CrdtId { author: 1, seq: 31 };
    let b_id = CrdtId { author: 1, seq: 32 };
    let c_id = CrdtId { author: 1, seq: 33 };

    let bytes = page_bytes([
        scene_tree_block(CrdtId { author: 1, seq: 1 }, layer_node_id, CrdtId::ROOT),
        scene_tree_block(CrdtId { author: 1, seq: 2 }, group_node_id, layer_node_id),
        group_item_block(
            CrdtId::ROOT,
            layer_item_id,
            CrdtId::SENTINEL,
            CrdtId::SENTINEL,
            layer_node_id,
        ),
        line_item_block(
            layer_node_id,
            b_id,
            group_item_id,
            CrdtId::SENTINEL,
            PenColor::Blue,
        ),
        group_item_block(layer_node_id, group_item_id, a_id, b_id, group_node_id),
        line_item_block(
            group_node_id,
            c_id,
            CrdtId::SENTINEL,
            CrdtId::SENTINEL,
            PenColor::Red,
        ),
        line_item_block(
            layer_node_id,
            a_id,
            CrdtId::SENTINEL,
            group_item_id,
            PenColor::Black,
        ),
    ]);

    let page = parse_page(&bytes).unwrap();
    assert_eq!(page.layers.len(), 1);
    assert_eq!(page.layers[0].node_id, layer_node_id);
    let colors: Vec<_> = page.layers[0].lines.iter().map(|line| line.color).collect();
    assert_eq!(colors, vec![PenColor::Black, PenColor::Red, PenColor::Blue]);
}

const FILE_HEADER: &[u8; 43] = b"reMarkable .lines file, version=6          ";

fn page_bytes(blocks: impl IntoIterator<Item = Vec<u8>>) -> Vec<u8> {
    let mut bytes = FILE_HEADER.to_vec();
    for block in blocks {
        bytes.extend_from_slice(&block);
    }
    bytes
}

fn scene_tree_block(tree_id: CrdtId, node_id: CrdtId, parent_id: CrdtId) -> Vec<u8> {
    let mut parent = Vec::new();
    push_id_field(&mut parent, 1, parent_id);

    let mut body = Vec::new();
    push_id_field(&mut body, 1, tree_id);
    push_id_field(&mut body, 2, node_id);
    push_byte_field(&mut body, 3, 0);
    push_subblock(&mut body, 4, &parent);
    block(BlockType::SceneTree, 1, body)
}

fn group_item_block(
    parent_id: CrdtId,
    item_id: CrdtId,
    left_id: CrdtId,
    right_id: CrdtId,
    node_id: CrdtId,
) -> Vec<u8> {
    let mut value = vec![0x02];
    push_id_field(&mut value, 2, node_id);
    scene_item_block(
        BlockType::SceneGroupItem,
        1,
        parent_id,
        item_id,
        left_id,
        right_id,
        &value,
    )
}

fn line_item_block(
    parent_id: CrdtId,
    item_id: CrdtId,
    left_id: CrdtId,
    right_id: CrdtId,
    color: PenColor,
) -> Vec<u8> {
    let mut points = Vec::new();
    points.extend_from_slice(&1.0_f32.to_le_bytes());
    points.extend_from_slice(&2.0_f32.to_le_bytes());
    points.extend_from_slice(&1u16.to_le_bytes());
    points.extend_from_slice(&1u16.to_le_bytes());
    points.push(0);
    points.push(255);

    let mut value = vec![0x03];
    push_u32_field(&mut value, 1, Pen::BallpointV2 as u32);
    push_u32_field(&mut value, 2, color as u32);
    push_f64_field(&mut value, 3, 1.0);
    push_f32_field(&mut value, 4, 0.0);
    push_subblock(&mut value, 5, &points);
    push_id_field(&mut value, 6, item_id);

    scene_item_block(
        BlockType::SceneLineItem,
        2,
        parent_id,
        item_id,
        left_id,
        right_id,
        &value,
    )
}

fn scene_item_block(
    block_type: BlockType,
    current_version: u8,
    parent_id: CrdtId,
    item_id: CrdtId,
    left_id: CrdtId,
    right_id: CrdtId,
    value: &[u8],
) -> Vec<u8> {
    let mut body = Vec::new();
    push_id_field(&mut body, 1, parent_id);
    push_id_field(&mut body, 2, item_id);
    push_id_field(&mut body, 3, left_id);
    push_id_field(&mut body, 4, right_id);
    push_u32_field(&mut body, 5, 0);
    push_subblock(&mut body, 6, value);
    block(block_type, current_version, body)
}

fn block(block_type: BlockType, current_version: u8, mut body: Vec<u8>) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(8 + body.len());
    bytes.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(&[0x00, 0x01, current_version, block_type as u8]);
    bytes.append(&mut body);
    bytes
}

fn push_id_field(bytes: &mut Vec<u8>, index: u32, value: CrdtId) {
    push_tag(bytes, index, 0x0F);
    bytes.push(value.author);
    bytes.extend_from_slice(&encode_varuint(value.seq));
}

fn push_byte_field(bytes: &mut Vec<u8>, index: u32, value: u8) {
    push_tag(bytes, index, 0x01);
    bytes.push(value);
}

fn push_u32_field(bytes: &mut Vec<u8>, index: u32, value: u32) {
    push_tag(bytes, index, 0x04);
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_f32_field(bytes: &mut Vec<u8>, index: u32, value: f32) {
    push_tag(bytes, index, 0x04);
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_f64_field(bytes: &mut Vec<u8>, index: u32, value: f64) {
    push_tag(bytes, index, 0x08);
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_subblock(bytes: &mut Vec<u8>, index: u32, value: &[u8]) {
    push_tag(bytes, index, 0x0C);
    bytes.extend_from_slice(&u32::try_from(value.len()).unwrap().to_le_bytes());
    bytes.extend_from_slice(value);
}

fn push_tag(bytes: &mut Vec<u8>, index: u32, tag_type: u8) {
    bytes.extend_from_slice(&encode_varuint(
        (u64::from(index) << 4) | u64::from(tag_type),
    ));
}

fn encode_varuint(mut value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let mut byte = u8::try_from(value & 0x7F).unwrap();
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        bytes.push(byte);
        if value == 0 {
            return bytes;
        }
    }
}
