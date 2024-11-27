use blehr::{Error, Scanner};
use futures::StreamExt;
use log::info;

#[tokio::main]
async fn main() -> Result<(), Box<Error>> {
  pretty_env_logger::init();
  let mut scanner = Scanner::new().await?.find_adapter().await?;

  info!("Scanning for HR sensors ...");

  loop {
    info!("Scanning ...");
    scanner.start().await?;

    if let Some(mut sensor) = scanner.next_sensor().await? {
      println!(
        "Found {}",
        sensor.name().await.unwrap_or_else(|| "unknown sensor".to_string())
      );

      scanner.stop().await?;

      if let Ok(mut hr_stream) = sensor.hr_stream().await {
        while let Some(hr) = hr_stream.next().await {
          info!("hr: {:?}", hr);
        }
      }
    }
  }
}
