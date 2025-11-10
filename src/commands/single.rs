use crate::communications::oscilloscope::OscilloscopeHandler;
use crate::config::Config;
use anyhow::{Context, Result};

pub fn single(cfg: &Config) -> Result<()> {
    let mut handler = OscilloscopeHandler::initialize(cfg)
        .context("failed to initialize oscilloscope handler")?;
    handler
        .set_single()
        .context("failed to set oscilloscope to single mode")?;

    println!("⏱️ Oscilloscope set to single mode successfully.",);
    Ok(())
}
