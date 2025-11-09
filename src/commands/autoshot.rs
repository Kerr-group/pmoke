use crate::communications::function_generator::FGHandler;
use crate::communications::oscilloscope::OscilloscopeHandler;
use crate::config::Config;
use anyhow::{Context, Result};
use colored::*;
use std::thread;
use std::time::Duration;

pub fn autoshot(cfg: &Config) -> Result<()> {
    let mut handler = OscilloscopeHandler::initialize(cfg)
        .context("failed to initialize oscilloscope handler")?;
    handler
        .set_single()
        .context("failed to set oscilloscope to single mode")?;

    println!(
        "{}  Oscilloscope set to single mode successfully.",
        "✔".green().bold()
    );

    thread::sleep(Duration::from_secs(1));

    let mut handler =
        FGHandler::initialize(cfg).context("failed to initialize function generator handler")?;
    handler.trigger().context("failed to trigger")?;

    println!("{}  Trigger command sent successfully.", "✔".green().bold());

    Ok(())
}
