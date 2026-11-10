//! Independent upstream fixtures; provenance and limitations live in
//! `conformance/parser/rmscene/README.md` at the workspace root.

use remarkable_rm::scene::text::{ParagraphStyle, TextContent};
use remarkable_rm::{CrdtId, Page, Pen, PenColor, parse_page};

fn fixture(name: &str) -> Page {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/parser/rmscene")
        .join(name)
        .join(format!("{name}.rm"));
    let bytes =
        std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    parse_page(&bytes).unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

macro_rules! parses {
    ($($test:ident => $name:literal),+ $(,)?) => {
        $(
            #[test]
            fn $test() {
                // Acceptance coverage, not a claim of complete semantic support.
                let _ = fixture($name);
            }
        )+
    };
}

parses! {
    formatted_paragraphs => "Bold_Heading_Bullet_Normal",
    paper_pro_colors => "Color_and_tool_v3.14.4",
    stroke_v2 => "Lines_v2",
    updated_stroke_v2 => "Lines_v2_updated",
    highlight_and_shader => "More_color_highlight_shader_v3.15.4.2",
    plain_text => "Normal_AB",
    text_with_two_layers => "Normal_A_stroke_2_layers",
    firmware_3_2_2 => "Normal_A_stroke_2_layers_v3.2.2",
    firmware_3_3_2 => "Normal_A_stroke_2_layers_v3.3.2",
    pdf_highlight_page_one => "Wikipedia_highlighted_p1",
    pdf_highlight_page_two => "Wikipedia_highlighted_p2",
    optional_scene_info => "With_SceneInfo_Block",
    concurrent_text_inserts => "test-crdt-ordering",
}

#[test]
fn normal_ab_matches_upstream_semantics() {
    // rmscene tests/test_scene_stream.py::test_normal_ab establishes these values.
    let page = fixture("Normal_AB");
    assert_eq!(page.layers.len(), 1, "got layers: {:?}", page.layers);
    let layer = &page.layers[0];
    assert_eq!(layer.node_id, CrdtId { author: 0, seq: 11 });
    assert_eq!(layer.name, "Layer 1");
    assert!(layer.visible, "got hidden layer: {layer:?}");
    assert!(layer.lines.is_empty(), "got strokes: {:?}", layer.lines);

    let text = page.text.expect("Normal_AB: expected root text, got None");
    assert_eq!(text.items.len(), 1, "got text items: {:?}", text.items);
    let item = &text.items[0];
    assert_eq!(item.item_id, CrdtId { author: 1, seq: 16 });
    assert_eq!(item.deleted_length, 0);
    assert!(
        matches!(&item.content, Some(TextContent::String(value)) if value == "AB"),
        "expected AB, got {:?}",
        item.content
    );
    assert_eq!(text.paragraphs.len(), 1);
    assert_eq!(text.paragraphs[0].style, ParagraphStyle::Plain);
    assert_eq!(text.pos_x.to_bits(), (-468.0_f64).to_bits());
    assert_eq!(text.pos_y.to_bits(), 234.0_f64.to_bits());
    assert_eq!(text.width.to_bits(), 936.0_f32.to_bits());
}

#[test]
fn paper_pro_dimensions_match_upstream() {
    // rmscene tests/test_scene_tree.py::test_scene_info_paper_size.
    let page = fixture("Color_and_tool_v3.14.4");
    assert_eq!(page.paper_size, Some((1620, 2160)));
}

#[test]
fn paper_pro_color_tool_relationships_match_upstream() {
    // rmscene tests/test_color_tool.py, strengthened to reject vacuous success.
    let page = fixture("Color_and_tool_v3.14.4");
    let mut seen = std::collections::BTreeSet::new();
    for line in page.layers.iter().flat_map(|layer| &layer.lines) {
        let expected_tool = match line.color {
            PenColor::Highlight => Pen::Shader,
            PenColor::GreenV2 | PenColor::Cyan | PenColor::Magenta => Pen::BallpointV2,
            _ => continue,
        };
        assert_eq!(line.tool, expected_tool, "got line: {line:?}");
        seen.insert(line.color);
    }
    assert!(
        seen.contains(&PenColor::Highlight),
        "expected to exercise the shader assertion, got colors: {seen:?}"
    );
    // Upstream permits these three ballpoint colors; it does not require all
    // three to survive scene assembly (e.g. deletion of a stroke).
    assert!(
        [PenColor::GreenV2, PenColor::Cyan, PenColor::Magenta]
            .iter()
            .any(|color| seen.contains(color)),
        "expected to exercise the colored ballpoint assertion, got colors: {seen:?}"
    );
}

fn text_paragraphs(name: &str) -> Vec<(ParagraphStyle, String)> {
    fixture(name)
        .text
        .unwrap()
        .layout()
        .unwrap()
        .paragraphs
        .into_iter()
        .map(|p| {
            (
                p.style,
                p.spans.iter().map(|s| s.text.as_str()).collect::<String>(),
            )
        })
        .filter(|(_, s)| !s.is_empty())
        .collect()
}

#[test]
fn concurrent_authors_reconstruct_text_in_upstream_order() {
    assert_eq!(
        text_paragraphs("test-crdt-ordering"),
        vec![(ParagraphStyle::Heading, "A12_Z".into())]
    );
}

#[test]
fn text_paragraph_styles_match_upstream() {
    assert_eq!(
        text_paragraphs("Bold_Heading_Bullet_Normal"),
        vec![
            (ParagraphStyle::Bold, "A".into()),
            (ParagraphStyle::Heading, "new line".into()),
            (
                ParagraphStyle::Bullet,
                "B is a letter of the alphabet".into()
            ),
            (ParagraphStyle::Plain, "C".into()),
        ]
    );
}

#[test]
fn inline_formatting_survives_deleted_character_ranges() {
    let text = fixture("Normal_A_stroke_2_layers_v3.3.2").text.unwrap();
    let paragraphs: Vec<String> = text
        .layout()
        .unwrap()
        .paragraphs
        .into_iter()
        .map(|p| {
            p.spans
                .iter()
                .map(|s| {
                    let mut content = s.text.clone();
                    if content.is_empty() {
                        return content;
                    }
                    if s.bold {
                        content = format!("<b>{content}</b>");
                    }
                    if s.italic {
                        content = format!("<i>{content}</i>");
                    }
                    content
                })
                .collect::<String>()
        })
        .filter(|p| !p.is_empty())
        .collect();
    assert_eq!(
        paragraphs,
        [
            "A",
            "v3.2.2",
            "Normal <b>bold</b> <i>italic</i>",
            "<b>Bold</b> <i>italic</i> normal",
            "Bold line",
            "Normal line",
            "Heading line"
        ]
    );
}

#[test]
fn official_sample_decodes_numbering_colours_highlights_and_anchors() {
    let bytes = include_bytes!(
        "../../../conformance/rendering/tablet-exports/remarkable-rm/sample-page/sample-page.rm"
    );
    let page = parse_page(bytes).unwrap();
    let text = page.text.as_ref().unwrap();
    let layout = text.layout().unwrap();
    // Independently confirmed with rmscene 0.8.0: 141 raw line blocks, 134 live.
    assert_eq!(
        page.layers
            .iter()
            .map(|layer| layer.lines.len())
            .sum::<usize>(),
        134
    );
    assert_eq!(
        layout
            .paragraphs
            .iter()
            .filter(|p| p.style == ParagraphStyle::Numbered)
            .count(),
        2
    );
    assert!(
        page.layers
            .iter()
            .flat_map(|l| &l.lines)
            .any(|line| line.color_rgba.is_some()),
        "expected explicit stroke colours"
    );
    assert!(
        page.layers.iter().any(|layer| !layer.glyphs.is_empty()),
        "expected text highlight rectangles"
    );
    // This spelling is present in the source bytes, independently of layout;
    // the official PNG's different spelling cannot be a renderer expectation.
    assert!(
        bytes
            .windows(b" and hilight".len())
            .any(|w| w == b" and hilight")
    );
    let paragraphs: Vec<String> = layout
        .paragraphs
        .iter()
        .map(|p| p.spans.iter().map(|s| s.text.as_str()).collect())
        .collect();
    assert!(
        paragraphs.iter().any(|p| p == "Text small and hilight"),
        "got {paragraphs:?}"
    );
    assert!(
        page.layers
            .iter()
            .flat_map(|l| &l.lines)
            .flat_map(|l| &l.points)
            .any(|p| p.y > 1872.0),
        "expected anchored content below nominal screen"
    );
}

#[test]
fn typed_text_is_rendered_with_bundled_fonts() {
    let page = fixture("Normal_AB");
    let opts = remarkable_rm::RenderOptions::for_page(&page);
    let png = remarkable_rm::render_page(&page, opts).unwrap();
    let image = tiny_skia::Pixmap::decode_png(&png).unwrap();
    let ink: Vec<_> = image
        .pixels()
        .iter()
        .enumerate()
        .filter(|(_, p)| p.red() < 200)
        .map(|(i, _)| (i % 1404, i / 1404))
        .collect();
    assert!(
        ink.len() > 50,
        "expected AB glyphs, got {} ink pixels",
        ink.len()
    );
    assert!(
        ink.iter()
            .all(|&(x, y)| (230..300).contains(&x) && (180..240).contains(&y)),
        "unexpected text bounds: {ink:?}"
    );
    assert_eq!(
        png,
        remarkable_rm::render_page(&page, opts).unwrap(),
        "text rasterization must be deterministic"
    );
}
