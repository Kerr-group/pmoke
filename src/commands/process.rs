use crate::commands::analyze::run_analyze_locked;
use crate::commands::fetch::{
    begin_fetch_after_preflight_locked, preflight_fetch_locked, run_fetch_after_preflight_locked,
};
use crate::communications::oscilloscope::OscilloscopeHandler;
use crate::config::{Config, ValidationTarget};
use anyhow::{Context, Result};

pub fn process(cfg: &Config) -> Result<()> {
    if cfg.force {
        anyhow::bail!(
            "--force is not supported for process/auto; \
             use fetch --force followed by analyze"
        );
    }
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock =
        crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "process")?;

    crate::config::validate_for_target(cfg, ValidationTarget::Process)?;
    preflight_fetch_locked(cfg)?;
    validate_scope_connection(cfg)?;
    begin_fetch_after_preflight_locked(cfg)?;
    let data = run_fetch_after_preflight_locked(cfg)?;
    run_analyze_locked(cfg, &data)?;

    Ok(())
}

fn validate_scope_connection(cfg: &Config) -> Result<()> {
    let mut scope = OscilloscopeHandler::initialize(cfg)
        .context("failed to connect to oscilloscope during process preflight")?;
    scope
        .identify()
        .context("failed to identify oscilloscope during process preflight")?;
    Ok(())
}
