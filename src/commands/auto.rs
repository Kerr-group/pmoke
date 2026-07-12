use crate::commands::analyze::run_analyze_locked;
use crate::commands::fetch::{
    begin_fetch_after_preflight_locked, preflight_fetch_locked, run_fetch_after_preflight_locked,
};
use crate::communications::function_generator::FGHandler;
use crate::communications::oscilloscope::OscilloscopeHandler;
use crate::{
    commands::autoshot::autoshot,
    config::{Config, ValidationTarget},
};
use anyhow::{Context, Result};
use std::thread;
use std::time::Duration;

pub fn auto(cfg: &Config) -> Result<()> {
    if cfg.force {
        anyhow::bail!(
            "--force is not supported for process/auto; \
             use fetch --force followed by analyze"
        );
    }
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock = crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "auto")?;

    crate::config::validate_for_target(cfg, ValidationTarget::Auto)?;
    preflight_fetch_locked(cfg)?;
    validate_instrument_connections(cfg)?;
    begin_fetch_after_preflight_locked(cfg)?;

    if let Err(error) = autoshot(cfg) {
        crate::commands::run_dir::write_run_state(cfg, "failed", "auto", Some(&error))?;
        return Err(error);
    }

    thread::sleep(Duration::from_secs(3));

    let data = run_fetch_after_preflight_locked(cfg)?;
    run_analyze_locked(cfg, &data)?;

    Ok(())
}

fn validate_instrument_connections(cfg: &Config) -> Result<()> {
    let mut scope = OscilloscopeHandler::initialize(cfg)
        .context("failed to connect to oscilloscope during auto preflight")?;
    scope
        .identify()
        .context("failed to identify oscilloscope during auto preflight")?;

    let mut generator = FGHandler::initialize(cfg)
        .context("failed to connect to generator during auto preflight")?;
    generator
        .identify()
        .context("failed to identify generator during auto preflight")?;
    Ok(())
}
