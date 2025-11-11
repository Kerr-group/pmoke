use crate::config::Config;
use crate::lockin::run;
use anyhow::Result;

pub fn li(cfg: &Config) -> Result<()> {
    run(cfg)?;
    Ok(())
}
