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

    let mut pen_counts: BTreeMap<Pen, u32> = BTreeMap::new();
    let mut color_counts: BTreeMap<PenColor, u32> = BTreeMap::new();
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
            *pen_counts.entry(line.tool).or_default() += 1;
            *color_counts.entry(line.color).or_default() += 1;
        }
    }
    println!("total lines: {total_lines}");
    println!("total points: {total_points}");
    println!("pens: {pen_counts:?}");
    println!("colors: {color_counts:?}");

    ExitCode::SUCCESS
}
