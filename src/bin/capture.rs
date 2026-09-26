//! Read-only display diagnostic: never creates virtual input devices.
use anyhow::{bail, Result};
use smart_remarkable::{device::DeviceModel, screenshot::Screenshot};

fn main() -> Result<()> {
    let path = std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!("Usage: capture OUTPUT.png"))?;
    if DeviceModel::detect() == DeviceModel::Unknown {
        bail!("Unsupported device; refusing to guess framebuffer layout");
    }
    let mut screen = Screenshot::new()?;
    screen.take_screenshot()?;
    screen.save_image(&path)?;
    println!("Saved display to {path}; selection: {:?}", screen.detect_selection_rect());
    Ok(())
}
