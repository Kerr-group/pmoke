use crate::commands::process::process;
use crate::{commands::autoshot::autoshot, config::Config};
use anyhow::Result;
use std::thread;
use std::time::Duration;

pub fn auto(cfg: &Config) -> Result<()> {
    if cfg.force {
        anyhow::bail!(
            "--force is not yet supported for process/auto; \
             use fetch --force followed by analyze"
        );
    }
    autoshot(cfg)?;

    thread::sleep(Duration::from_secs(3));

    process(cfg)?;

    Ok(())
}
