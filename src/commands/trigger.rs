use crate::communications::function_generator::FGHandler;
use crate::config::Config;
use anyhow::{Context, Result};
use colored::*;

pub fn trigger(cfg: &Config) -> Result<()> {
    let mut handler =
        FGHandler::initialize(cfg).context("failed to initialize function generator handler")?;
    handler.trigger().context("failed to trigger")?;

    println!("{}  Trigger command sent successfully.", "✔".green().bold());
    Ok(())
}
