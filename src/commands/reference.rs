use crate::config::Config;
use crate::lockin::reference::run;
use anyhow::Result;

pub fn reference(cfg: &Config) -> Result<()> {
    run(cfg)?;
    Ok(())
}
