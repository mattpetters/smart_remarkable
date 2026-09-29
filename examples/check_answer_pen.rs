//! On-device UI round trip. Changes pen settings temporarily; draws no ink and
//! sends no model request. Stop the gesture listener before running this check.
use anyhow::Result;
use smart_remarkable::{ink_session::with_ballpoint_color, preferences::{self, InkColor}};

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let fail = std::env::args().any(|arg| arg == "--fail");
    let args: Vec<_> = std::env::args().collect();
    let color: InkColor = match args.iter().position(|a| a == "--color") {
        Some(i) => serde_json::from_value(serde_json::json!(args.get(i + 1).ok_or_else(|| anyhow::anyhow!("--color needs a value"))?))?,
        None => preferences::load()?.ink_color,
    };
    let result = with_ballpoint_color(color, || async move {
        println!("{:?} Ballpoint verified; no ink drawn.", color);
        if fail {
            anyhow::bail!("Simulated request failure");
        }
        Ok(())
    })
    .await;
    match result {
        Err(error) if fail && error.to_string() == "Simulated request failure" => {
            println!("Simulated failure returned after settings restoration.");
            Ok(())
        }
        result => result,
    }
}
