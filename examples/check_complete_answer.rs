//! Hardware integration check. Creates fresh note pages after the current page
//! and writes the supplied answer there. Run only while the listener is idle.
use anyhow::{ensure, Result};
use smart_remarkable::{
    answer_delivery::draw_complete_answer,
    cancellation::SmartRemarkableCancellation,
    ink_session::with_answer_ballpoint,
    note_page::insert_after_current,
    pen::Pen,
    screenshot::Screenshot,
    touch::{Rect, Touch, TriggerCorner},
};
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let path = std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!("Supply an answer JSON file"))?;
    let data: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let lines: Vec<String> = serde_json::from_value(data["lines"].clone())?;
    ensure!((17..=25).contains(&lines.len()), "Integration fixture must span 17-25 lines");
    let mut ss = Screenshot::new()?;
    ss.take_screenshot()?;
    with_answer_ballpoint(|| async {
        let fresh = insert_after_current(&mut Touch::new(false, TriggerCorner::FourFinger)).await?;
        let clear = smart_remarkable::page_layout::append_rect(&fresh, Rect { x: 64, y: 55, w: 0, h: 0 })?;
        ensure!(clear.y < 100 && clear.h > 800, "Test page is not blank; no ink drawn");
        // Deliberately start near the bottom so the rest must use another page.
        draw_complete_answer(
            &lines,
            Rect { x: 64, y: 780, w: 694, h: 190 },
            Arc::new(Mutex::new(Pen::new(false))),
            Arc::new(Mutex::new(None)),
            Arc::new(SmartRemarkableCancellation::new()),
        )
        .await?;
        Ok(())
    })
    .await?;
    println!("Complete response drawn across note pages; original tool restored.");
    Ok(())
}
