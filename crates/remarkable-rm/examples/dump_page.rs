//! Quick probe — dumps `parse_page` output for a `.rm` file.
//!
//! Usage: `cargo run -q -p remarkable-rm --example dump_page -- <path>.rm`

use std::collections::BTreeMap;
use std::env;
use std::process::ExitCode;

use remarkable_rm::{Pen, PenColor, parse_page};

fn main() -> ExitCode {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: dump_page <path.rm>");
        return ExitCode::from(2);
    };
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read {path}: {e}");
            return ExitCode::from(1);
        }
    };
    let page = match parse_page(&bytes) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("parse {path}: {e}");
            return ExitCode::from(1);
        }
    };

    println!("file: {path}");
    println!("paper_size: {:?}", page.paper_size);
    println!("text: {}", if page.text.is_some() { "yes" } else { "no" });
    println!("layers: {}", page.layers.len());

    let mut pen_counts: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut color_counts: BTreeMap<&'static str, u32> = BTreeMap::new();
    let mut total_lines = 0usize;
    let mut total_points = 0usize;

    for (i, layer) in page.layers.iter().enumerate() {
        println!(
            "  layer {i}: node_id={:?} name={:?} visible={} lines={}",
            layer.node_id,
            layer.name,
            layer.visible,
            layer.lines.len()
        );
        total_lines += layer.lines.len();
        for line in &layer.lines {
            total_points += line.points.len();
            *pen_counts.entry(pen_name(line.tool)).or_default() += 1;
            *color_counts.entry(color_name(line.color)).or_default() += 1;
        }
    }
    println!("total lines: {total_lines}");
    println!("total points: {total_points}");
    println!("pens: {pen_counts:?}");
    println!("colors: {color_counts:?}");

    ExitCode::SUCCESS
}

fn pen_name(p: Pen) -> &'static str {
    match p {
        Pen::PaintbrushV1 => "PaintbrushV1",
        Pen::PencilV1 => "PencilV1",
        Pen::BallpointV1 => "BallpointV1",
        Pen::MarkerV1 => "MarkerV1",
        Pen::FinelinerV1 => "FinelinerV1",
        Pen::HighlighterV1 => "HighlighterV1",
        Pen::Eraser => "Eraser",
        Pen::MechanicalPencilV1 => "MechanicalPencilV1",
        Pen::EraserAreaSelect => "EraserAreaSelect",
        Pen::PaintbrushV2 => "PaintbrushV2",
        Pen::MechanicalPencilV2 => "MechanicalPencilV2",
        Pen::PencilV2 => "PencilV2",
        Pen::BallpointV2 => "BallpointV2",
        Pen::MarkerV2 => "MarkerV2",
        Pen::FinelinerV2 => "FinelinerV2",
        Pen::HighlighterV2 => "HighlighterV2",
        Pen::Calligraphy => "Calligraphy",
        Pen::Shader => "Shader",
    }
}

fn color_name(c: PenColor) -> &'static str {
    match c {
        PenColor::Black => "Black",
        PenColor::Gray => "Gray",
        PenColor::White => "White",
        PenColor::Yellow => "Yellow",
        PenColor::Green => "Green",
        PenColor::Pink => "Pink",
        PenColor::Blue => "Blue",
        PenColor::Red => "Red",
        PenColor::GrayOverlap => "GrayOverlap",
        PenColor::Highlight => "Highlight",
        PenColor::GreenV2 => "GreenV2",
        PenColor::Cyan => "Cyan",
        PenColor::Magenta => "Magenta",
        PenColor::YellowV2 => "YellowV2",
    }
}
