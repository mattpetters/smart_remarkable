//! On-device integration check: inserts one blank note page after the current
//! page and restores the original pen/tool state. Draws no ink or model output.
//! Run only while the gesture listener is idle and the device is not in use.
use anyhow::Result;
use smart_remarkable::{
    ink_session::with_red_ballpoint,
    note_page::insert_after_current,
    screenshot::Screenshot,
    touch::{Touch, TriggerCorner},
};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let mut initial = Screenshot::new()?;
    initial.take_screenshot()?;
    with_red_ballpoint(|| async {
        insert_after_current(&mut Touch::new(false, TriggerCorner::FourFinger)).await?;
        println!("Native note page inserted and opened; original captured context is retained by the request coordinator.");
        Ok(())
    })
    .await?;
    println!("Original tool and profiles restored on the new note page.");
    Ok(())
}
