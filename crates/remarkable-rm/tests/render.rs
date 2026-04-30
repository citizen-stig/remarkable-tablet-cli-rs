//! Integration tests for the renderer against the v6 fixtures.
//!
//! These keep the bar low on purpose — we avoid pixel-exact snapshots
//! (brittle while the rasterizer evolves) and only assert coarse
//! invariants: PNG round-trips through `tiny-skia`, dimensions match,
//! and stroke pixels actually appear on the canvas.

use remarkable_rm::{DEFAULT_HEIGHT, DEFAULT_WIDTH, RenderOptions, parse_page, render_page};
use tiny_skia::Pixmap;

const SMOKE: &[u8] = include_bytes!("fixtures/smoke.rm");
const PENS_SMALL: &[u8] = include_bytes!("fixtures/pens-small.rm");
const EDITS: &[u8] = include_bytes!("fixtures/edits.rm");
const LAYERS: &[u8] = include_bytes!("fixtures/layers.rm");

fn render_default(bytes: &[u8]) -> Pixmap {
    let page = parse_page(bytes).expect("fixture parses");
    let opts = RenderOptions::for_page(&page);
    let png = render_page(&page, opts).expect("render succeeds");
    Pixmap::decode_png(&png).expect("PNG round-trips")
}

fn count_non_white_pixels(pixmap: &Pixmap) -> usize {
    pixmap
        .pixels()
        .iter()
        .filter(|p| !(p.red() == 0xFF && p.green() == 0xFF && p.blue() == 0xFF))
        .count()
}

#[test]
fn smoke_renders_with_strokes() {
    let pixmap = render_default(SMOKE);
    assert_eq!(pixmap.width(), DEFAULT_WIDTH);
    assert_eq!(pixmap.height(), DEFAULT_HEIGHT);
    assert!(
        count_non_white_pixels(&pixmap) > 100,
        "smoke fixture has 3 strokes, expected ink on canvas"
    );
}

#[test]
fn pens_small_renders_distinct_colors() {
    let pixmap = render_default(PENS_SMALL);
    assert_eq!(pixmap.width(), DEFAULT_WIDTH);
    assert_eq!(pixmap.height(), DEFAULT_HEIGHT);
    // 9 strokes across all v2 tools, mostly black or color — should be a
    // healthy amount of ink.
    assert!(
        count_non_white_pixels(&pixmap) > 1_000,
        "pens-small should paint many pixels across 9 strokes"
    );
}

#[test]
fn edits_renders_remaining_strokes() {
    // The fixture has 8 surviving strokes after tombstone filtering —
    // the parser hands those to us; the renderer just needs to draw them.
    let pixmap = render_default(EDITS);
    assert!(
        count_non_white_pixels(&pixmap) > 100,
        "edits fixture should still show ink for non-tombstoned strokes"
    );
}

#[test]
fn layers_renders_all_three() {
    let pixmap = render_default(LAYERS);
    assert!(
        count_non_white_pixels(&pixmap) > 100,
        "layers fixture should composite all visible layers"
    );
}

#[test]
fn dimensions_track_paper_size() {
    let page = parse_page(SMOKE).unwrap();
    let opts = RenderOptions::for_page(&page);
    let png = render_page(&page, opts).unwrap();
    let pixmap = Pixmap::decode_png(&png).unwrap();
    let (expected_w, expected_h) = page.paper_size.expect("smoke has paper size");
    assert_eq!(pixmap.width(), expected_w);
    assert_eq!(pixmap.height(), expected_h);
}

#[test]
fn explicit_options_override_paper_size() {
    let page = parse_page(SMOKE).unwrap();
    let opts = RenderOptions {
        width: 702,
        height: 936,
    };
    let png = render_page(&page, opts).unwrap();
    let pixmap = Pixmap::decode_png(&png).unwrap();
    assert_eq!(pixmap.width(), 702);
    assert_eq!(pixmap.height(), 936);
}
