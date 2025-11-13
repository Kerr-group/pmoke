use crate::commands::analyse::analyse;
use crate::{commands::autoshot::autoshot, config::Config};
use anyhow::Result;
use std::thread;
use std::time::Duration;

pub fn auto(cfg: &Config) -> Result<()> {
    autoshot(cfg)?;

    thread::sleep(Duration::from_secs(3));

    analyse(cfg)?;

    Ok(())
}
