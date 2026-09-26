//! On-device UI round trip. Changes pen settings temporarily; draws no ink and
//! sends no model request. Stop the gesture listener before running this check.
use anyhow::Result;
use smart_remarkable::ink_session::with_red_ballpoint;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let fail = std::env::args().any(|arg| arg == "--fail");
    let result = with_red_ballpoint(|| async move {
        println!("Red Ballpoint verified; no ink drawn.");
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
