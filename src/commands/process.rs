use crate::commands::analyze::run_analyze_locked;
use crate::commands::fetch::run_fetch_for_process_locked;
use crate::config::Config;
use anyhow::Result;

pub fn process(cfg: &Config) -> Result<()> {
    if cfg.force {
        anyhow::bail!(
            "--force is not yet supported for process/auto; \
             use fetch --force followed by analyze"
        );
    }
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock =
        crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "process")?;

    let data = run_fetch_for_process_locked(cfg)?;
    run_analyze_locked(cfg, &data)?;

    Ok(())
}
