use crate::commands::single::single;
use crate::commands::trigger::trigger;
use crate::config::Config;
use anyhow::Result;
use std::thread;
use std::time::Duration;

pub fn autoshot(cfg: &Config) -> Result<()> {
    single(cfg)?;

    thread::sleep(Duration::from_secs(1));

    trigger(cfg)?;

    Ok(())
}
