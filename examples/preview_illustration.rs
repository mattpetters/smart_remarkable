//! Render the first illustration of a bridge answer without touching a device.
//! cargo run --example preview_illustration -- answer.json output.svg output.png
use anyhow::{Context, Result};
use smart_remarkable::{
    illustration,
    touch::Rect,
    util::{svg_to_bitmap, write_bitmap_to_file},
};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(args.len() == 4, "Expected answer.json output.svg output.png");
    let answer: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let drawings: Vec<illustration::Illustration> = serde_json::from_value(answer["illustrations"].clone())?;
    illustration::validate(&drawings)?;
    let svg = illustration::svg(
        drawings.first().context("No illustration in this answer")?,
        Rect { x: 64, y: 80, w: 694, h: 440 },
    )?;
    std::fs::write(&args[2], &svg)?;
    let bitmap = svg_to_bitmap(&svg, 768, 1024)?;
    write_bitmap_to_file(&bitmap, &args[3])?;
    Ok(())
}
