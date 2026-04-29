//! Integration tests against real `.rm` v6 fixtures captured from a
//! reMarkable 2 tablet (see `tests/fixtures/README.md` for ground truth).

use std::collections::BTreeSet;

use remarkable_rm::{Pen, PenColor, parse_page};

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
        assert!(!line.points.is_empty(), "every stroke has at least one point");
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
