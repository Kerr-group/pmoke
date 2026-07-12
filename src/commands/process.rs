use crate::commands::analyze::run_analyze;
use crate::commands::fetch::run_fetch_for_process;
use crate::config::Config;
use anyhow::Result;

pub fn process(cfg: &Config) -> Result<()> {
    if cfg.force {
        anyhow::bail!(
            "--force is not yet supported for process/auto; \
             use fetch --force followed by analyze"
        );
    }
    let data = run_fetch_for_process(cfg)?;

    run_analyze(cfg, &data)?;

    Ok(())
}
