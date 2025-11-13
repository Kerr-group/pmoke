use crate::{config::Config, kerr::run};
use anyhow::Result;

pub fn kerr(cfg: &Config) -> Result<()> {
    run(cfg)?;
    Ok(())
}
