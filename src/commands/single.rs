use crate::communications::oscilloscope::OscilloscopeHandler;
use crate::config::Config;
use anyhow::Result;
use colored::*;

pub fn single(cfg: &Config) -> Result<()> {
    let mut handler = OscilloscopeHandler::initialize(cfg)?;
    handler.set_single()?;

    println!(
        "{}  Oscilloscope set to single mode successfully.",
        "✔".green().bold()
    );
    Ok(())
}
