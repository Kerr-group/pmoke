use crate::commands::analyze::run_analyze_locked;
use crate::commands::fetch::run_fetch_for_process_locked;
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
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock = crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "auto")?;

    autoshot(cfg)?;

    thread::sleep(Duration::from_secs(3));

    let data = run_fetch_for_process_locked(cfg)?;
    run_analyze_locked(cfg, &data)?;

    Ok(())
}
