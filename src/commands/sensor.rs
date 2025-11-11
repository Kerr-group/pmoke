use crate::config::Config;
use crate::lockin::sensor::run;
use anyhow::Result;

pub fn sensor(cfg: &Config) -> Result<()> {
    run(cfg)?;

    Ok(())
}
