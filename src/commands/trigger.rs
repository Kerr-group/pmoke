use crate::communications::function_generator::FGHandler;
use crate::config::Config;
use crate::ui;
use anyhow::{Context, Result};

pub fn trigger(cfg: &Config) -> Result<()> {
    let mut handler =
        FGHandler::initialize(cfg).context("failed to initialize function generator handler")?;
    handler.trigger().context("failed to trigger")?;

    ui::success("trigger command sent");
    Ok(())
}
