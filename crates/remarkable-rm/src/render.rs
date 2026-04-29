//! Rasterize a parsed [`Page`] to a PNG image.
//!
//! Composites stroke layers only — no PDF/ePub backgrounds. Pen colors map
//! to the fixed palette in spec §7.3; the highlighter is alpha-blended so
//! overlapping highlight strokes don't fully obscure ink.

use tiny_skia::{Color, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform};

use crate::document::{Layer, Page};
use crate::scene::line::{Line, Pen, PenColor};

/// Default canvas for a reMarkable 2 page (firmware 3.x).
pub const DEFAULT_WIDTH: u32 = 1404;
pub const DEFAULT_HEIGHT: u32 = 1872;

/// Highlighter alpha (0x40 ≈ 0.25) — matches `lines-are-rusty`. Higher
/// obscures underlying ink; lower stops reading as "highlight."
const HIGHLIGHTER_ALPHA_U8: u8 = 0x40;

/// `Point.width` is the tablet's u16 stroke-width unit. The v1 parser
/// (`scene::line::read_point_v1`) scales raw float pen widths by 4 before
/// storing, which matches the v2 wire format — so dividing by 4 yields
/// pixels at native canvas resolution, before the per-line
/// `thickness_scale` multiplier.
const POINT_WIDTH_TO_PX: f32 = 1.0 / 4.0;

/// Floor and ceiling on the rendered stroke width. Below 0.5 px tiny-skia
/// can't render anti-aliased strokes; the ceiling caps the highlighter
/// (raw ≈ 120 → 30 px) at a plausible pen size.
const MIN_STROKE_PX: f32 = 0.5;
const MAX_STROKE_PX: f32 = 30.0;

#[derive(Debug, Clone, Copy)]
pub struct RenderOptions {
    pub width: u32,
    pub height: u32,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self {
            width: DEFAULT_WIDTH,
            height: DEFAULT_HEIGHT,
        }
    }
}

impl RenderOptions {
    /// Pick canvas dimensions from a [`Page`], falling back to the firmware
    /// default. The page's `paper_size` block is what the tablet itself
    /// uses, so prefer it whenever the parser surfaced one.
    #[must_use]
    pub fn for_page(page: &Page) -> Self {
        match page.paper_size {
            Some((w, h)) => Self {
                width: w,
                height: h,
            },
            None => Self::default(),
        }
    }
}

/// Rasterize a parsed page into PNG-encoded bytes.
///
/// # Errors
/// Returns an error if the canvas dimensions are zero or exceed
/// `tiny-skia`'s allocation limit, or if the PNG encoder fails.
pub fn render_page(page: &Page, opts: &RenderOptions) -> Result<Vec<u8>, RenderError> {
    let mut pixmap = Pixmap::new(opts.width, opts.height).ok_or(RenderError::InvalidCanvas {
        width: opts.width,
        height: opts.height,
    })?;
    pixmap.fill(Color::WHITE);

    let transform = page_to_canvas_transform(page, opts);

    for layer in &page.layers {
        if !layer.visible {
            continue;
        }
        draw_layer(&mut pixmap, layer, transform);
    }

    pixmap
        .encode_png()
        .map_err(|e| RenderError::Encode(e.to_string()))
}

/// reMarkable stroke coordinates use a top-center origin: x = 0 is the page
/// midline, y = 0 is the top edge. Map that native page space onto the
/// requested output canvas with one tiny-skia transform so geometry and
/// effective stroke widths scale together.
#[allow(clippy::cast_precision_loss)]
fn page_to_canvas_transform(page: &Page, opts: &RenderOptions) -> Transform {
    let source = RenderOptions::for_page(page);
    let scale_x = opts.width as f32 / source.width as f32;
    let scale_y = opts.height as f32 / source.height as f32;
    Transform::from_row(scale_x, 0.0, 0.0, scale_y, opts.width as f32 / 2.0, 0.0)
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("invalid canvas dimensions: {width}x{height}")]
    InvalidCanvas { width: u32, height: u32 },
    #[error("PNG encoding failed: {0}")]
    Encode(String),
}

fn draw_layer(pixmap: &mut Pixmap, layer: &Layer, transform: Transform) {
    for line in &layer.lines {
        if is_eraser(line.tool) {
            continue;
        }
        let Some(path) = build_path(line) else {
            continue;
        };
        let mut paint = Paint {
            anti_alias: true,
            ..Paint::default()
        };
        paint.set_color(pen_color_to_rgba(line.color));

        let stroke = Stroke {
            width: stroke_width(line),
            line_cap: LineCap::Round,
            line_join: LineJoin::Round,
            ..Stroke::default()
        };

        pixmap.stroke_path(&path, &paint, &stroke, transform, None);
    }
}

fn build_path(line: &Line) -> Option<tiny_skia::Path> {
    let mut points = line.points.iter();
    let first = points.next()?;
    let mut pb = PathBuilder::new();
    pb.move_to(first.x, first.y);
    let mut emitted = false;
    for p in points {
        pb.line_to(p.x, p.y);
        emitted = true;
    }
    if !emitted {
        // Single-point strokes: draw a degenerate segment so a dot still
        // shows under round caps. tiny-skia's PathBuilder collapses a path
        // with one move_to and no line_to into None, so we'd silently lose
        // the dot otherwise.
        pb.line_to(first.x, first.y);
    }
    pb.finish()
}

fn is_eraser(pen: Pen) -> bool {
    matches!(pen, Pen::Eraser | Pen::EraserAreaSelect)
}

fn stroke_width(line: &Line) -> f32 {
    let median = median_point_width(line);
    // `thickness_scale` is a small multiplier (~0.5..2.0); narrowing to f32
    // for the rasterizer is intentional and within representable range.
    #[allow(clippy::cast_possible_truncation)]
    let scale = line.thickness_scale as f32;
    let raw = (f32::from(median) * POINT_WIDTH_TO_PX) * scale;
    raw.clamp(MIN_STROKE_PX, MAX_STROKE_PX)
}

/// Median over the per-vertex `Point.width`. Picking the median (over a
/// mean) keeps the stroke width stable under outliers — the tablet often
/// records a single very-large `width` at the start or end of a stroke.
fn median_point_width(line: &Line) -> u16 {
    debug_assert!(!line.points.is_empty(), "build_path filters empty strokes");
    let mut widths: Vec<u16> = line.points.iter().map(|p| p.width).collect();
    widths.sort_unstable();
    widths[widths.len() / 2]
}

/// Map the tablet's pen-color enum to an RGBA color.
///
/// `Highlight` is rendered semi-transparent so overlapping highlighter
/// strokes layer like a real highlighter; every other color is opaque.
fn pen_color_to_rgba(color: PenColor) -> Color {
    match color {
        PenColor::Black => Color::from_rgba8(0, 0, 0, 0xFF),
        PenColor::Gray => Color::from_rgba8(0x80, 0x80, 0x80, 0xFF),
        PenColor::White => Color::from_rgba8(0xFF, 0xFF, 0xFF, 0xFF),
        PenColor::Yellow => Color::from_rgba8(0xFF, 0xC4, 0x00, 0xFF),
        PenColor::Green => Color::from_rgba8(0x00, 0x99, 0x33, 0xFF),
        PenColor::Pink => Color::from_rgba8(0xFF, 0x40, 0x80, 0xFF),
        PenColor::Blue => Color::from_rgba8(0x14, 0x4F, 0xC4, 0xFF),
        PenColor::Red => Color::from_rgba8(0xCC, 0x33, 0x00, 0xFF),
        PenColor::GrayOverlap => Color::from_rgba8(0x60, 0x60, 0x60, 0xFF),
        PenColor::Highlight => Color::from_rgba8(0xFF, 0xF2, 0x00, HIGHLIGHTER_ALPHA_U8),
        PenColor::GreenV2 => Color::from_rgba8(0x35, 0xB5, 0x57, 0xFF),
        PenColor::Cyan => Color::from_rgba8(0x00, 0xB7, 0xC4, 0xFF),
        PenColor::Magenta => Color::from_rgba8(0xC4, 0x35, 0xB5, 0xFF),
        PenColor::YellowV2 => Color::from_rgba8(0xE6, 0xC1, 0x00, 0xFF),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crdt::CrdtId;
    use crate::scene::line::Point;

    fn line_with_points(points: Vec<Point>, color: PenColor, tool: Pen) -> Line {
        Line {
            tool,
            color,
            thickness_scale: 1.0,
            starting_length: 0.0,
            points,
            timestamp: CrdtId { author: 1, seq: 1 },
            move_id: None,
        }
    }

    fn point(x: f32, y: f32) -> Point {
        Point {
            x,
            y,
            speed: 0,
            width: 1024,
            direction: 0,
            pressure: 200,
        }
    }

    fn page_with_lines(lines: Vec<Line>, paper_size: Option<(u32, u32)>) -> Page {
        Page {
            layers: vec![Layer {
                node_id: CrdtId { author: 1, seq: 2 },
                name: "L1".into(),
                visible: true,
                lines,
            }],
            text: None,
            paper_size,
        }
    }

    fn count_non_white_pixels_in_columns(pixmap: &Pixmap, start_x: u32, end_x: u32) -> usize {
        let width = usize::try_from(pixmap.width()).unwrap();
        let start_x = usize::try_from(start_x).unwrap();
        let end_x = usize::try_from(end_x).unwrap();

        pixmap
            .pixels()
            .iter()
            .enumerate()
            .filter(|(idx, pixel)| {
                let x = idx % width;
                (start_x..end_x).contains(&x)
                    && (pixel.red() != 0xFF || pixel.green() != 0xFF || pixel.blue() != 0xFF)
            })
            .count()
    }

    #[test]
    fn empty_page_renders_white_canvas() {
        let page = Page::default();
        let png = render_page(&page, &RenderOptions::default()).unwrap();
        let decoded = Pixmap::decode_png(&png).unwrap();
        assert_eq!(decoded.width(), DEFAULT_WIDTH);
        assert_eq!(decoded.height(), DEFAULT_HEIGHT);
        // Every pixel is white.
        let any_non_white = decoded.pixels().iter().any(|p| {
            !(p.red() == 0xFF && p.green() == 0xFF && p.blue() == 0xFF && p.alpha() == 0xFF)
        });
        assert!(!any_non_white, "empty page should be uniformly white");
    }

    #[test]
    fn black_stroke_paints_non_white_pixels() {
        // Stroke coords are top-center; pick values that remain on canvas
        // after the transform applied by `render_page`.
        let line = line_with_points(
            vec![point(-300.0, 100.0), point(300.0, 800.0)],
            PenColor::Black,
            Pen::FinelinerV2,
        );
        let page = page_with_lines(vec![line], None);
        let png = render_page(&page, &RenderOptions::default()).unwrap();
        let decoded = Pixmap::decode_png(&png).unwrap();
        let dark_pixel_count = decoded
            .pixels()
            .iter()
            .filter(|p| p.red() < 0x40 && p.green() < 0x40 && p.blue() < 0x40)
            .count();
        assert!(dark_pixel_count > 100, "expected stroke ink on canvas");
    }

    #[test]
    fn invisible_layer_is_skipped() {
        let line = line_with_points(
            vec![point(-300.0, 200.0), point(300.0, 800.0)],
            PenColor::Black,
            Pen::FinelinerV2,
        );
        let page = Page {
            layers: vec![Layer {
                node_id: CrdtId { author: 1, seq: 2 },
                name: "hidden".into(),
                visible: false,
                lines: vec![line],
            }],
            text: None,
            paper_size: None,
        };
        let png = render_page(&page, &RenderOptions::default()).unwrap();
        let decoded = Pixmap::decode_png(&png).unwrap();
        let any_ink = decoded
            .pixels()
            .iter()
            .any(|p| p.red() != 0xFF || p.green() != 0xFF || p.blue() != 0xFF);
        assert!(!any_ink, "invisible layer should not paint");
    }

    #[test]
    fn eraser_strokes_are_skipped() {
        let line = line_with_points(
            vec![point(-400.0, 50.0), point(400.0, 900.0)],
            PenColor::Black,
            Pen::Eraser,
        );
        let page = page_with_lines(vec![line], None);
        let png = render_page(&page, &RenderOptions::default()).unwrap();
        let decoded = Pixmap::decode_png(&png).unwrap();
        let any_ink = decoded
            .pixels()
            .iter()
            .any(|p| p.red() != 0xFF || p.green() != 0xFF || p.blue() != 0xFF);
        assert!(!any_ink, "Eraser strokes must not paint");
    }

    #[test]
    fn for_page_uses_paper_size_when_present() {
        let page = Page {
            paper_size: Some((800, 600)),
            ..Page::default()
        };
        let opts = RenderOptions::for_page(&page);
        assert_eq!((opts.width, opts.height), (800, 600));
    }

    #[test]
    fn for_page_falls_back_to_default() {
        let page = Page::default();
        let opts = RenderOptions::for_page(&page);
        assert_eq!((opts.width, opts.height), (DEFAULT_WIDTH, DEFAULT_HEIGHT));
    }

    #[test]
    fn explicit_output_size_scales_strokes_into_requested_canvas() {
        let left = line_with_points(
            vec![point(-650.0, 120.0), point(-650.0, 900.0)],
            PenColor::Black,
            Pen::FinelinerV2,
        );
        let right = line_with_points(
            vec![point(650.0, 120.0), point(650.0, 900.0)],
            PenColor::Black,
            Pen::FinelinerV2,
        );
        let page = page_with_lines(vec![left, right], None);
        let png = render_page(
            &page,
            &RenderOptions {
                width: 702,
                height: 936,
            },
        )
        .unwrap();
        let decoded = Pixmap::decode_png(&png).unwrap();

        assert!(
            count_non_white_pixels_in_columns(&decoded, 0, 60) > 50,
            "expected ink near the left edge after scaling"
        );
        assert!(
            count_non_white_pixels_in_columns(&decoded, 642, 702) > 50,
            "expected ink near the right edge after scaling"
        );
    }

    #[test]
    fn scaling_uses_page_paper_size_instead_of_default_width() {
        let left = line_with_points(
            vec![point(-350.0, 60.0), point(-350.0, 540.0)],
            PenColor::Black,
            Pen::FinelinerV2,
        );
        let right = line_with_points(
            vec![point(350.0, 60.0), point(350.0, 540.0)],
            PenColor::Black,
            Pen::FinelinerV2,
        );
        let page = page_with_lines(vec![left, right], Some((800, 600)));
        let png = render_page(
            &page,
            &RenderOptions {
                width: 400,
                height: 300,
            },
        )
        .unwrap();
        let decoded = Pixmap::decode_png(&png).unwrap();

        assert!(
            count_non_white_pixels_in_columns(&decoded, 0, 40) > 20,
            "expected ink near the left edge when scaling from paper_size width"
        );
        assert!(
            count_non_white_pixels_in_columns(&decoded, 360, 400) > 20,
            "expected ink near the right edge when scaling from paper_size width"
        );
    }
}
