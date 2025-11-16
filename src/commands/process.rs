use crate::commands::analyze::run_analyze;
use crate::commands::fetch::run_fetch;
use crate::config::Config;
use crate::lockin::time::time_builder;
use anyhow::Result;

pub fn process(cfg: &Config) -> Result<()> {
    let data = run_fetch(cfg)?;

    let t = time_builder(cfg)?;

    run_analyze(cfg, t, data)?;

    Ok(())
}
