use crate::config::Config;
use crate::phase::run;
use anyhow::Result;

pub fn phase(cfg: &Config) -> Result<()> {
    run(cfg)?;
    Ok(())
}
