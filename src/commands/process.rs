use crate::commands::analyze::run_analyze;
use crate::commands::fetch::run_fetch_for_process;
use crate::config::Config;
use anyhow::Result;

pub fn process(cfg: &Config) -> Result<()> {
    let data = run_fetch_for_process(cfg)?;

    run_analyze(cfg, data)?;

    Ok(())
}
