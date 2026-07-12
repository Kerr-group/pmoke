use crate::commands::fetch::{
    begin_fetch_after_preflight_locked, preflight_fetch_locked, run_fetch_after_preflight_locked,
};
use crate::commands::single::single;
use crate::commands::trigger::trigger;
use crate::communications::function_generator::FGHandler;
use crate::communications::oscilloscope::OscilloscopeHandler;
use crate::config::{Config, ValidationTarget};
use anyhow::{Context, Result};
use std::thread;
use std::time::Duration;

pub fn automeasure(cfg: &Config) -> Result<()> {
    if cfg.force {
        anyhow::bail!(
            "--force is not supported for automeasure; \
             use fetch --force after an explicit trigger"
        );
    }
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock =
        crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "automeasure")?;
    crate::config::validate_for_target(cfg, ValidationTarget::Automeasure)?;
    preflight_fetch_locked(cfg)?;
    validate_instrument_connections(cfg)?;
    begin_fetch_after_preflight_locked(cfg)?;

    if let Err(error) = run_measurement_trigger(cfg) {
        crate::commands::run_dir::write_run_state(cfg, "failed", "automeasure", Some(&error))?;
        return Err(error);
    }

    run_fetch_after_preflight_locked(cfg)?;
    Ok(())
}

fn run_measurement_trigger(cfg: &Config) -> Result<()> {
    single(cfg)?;

    thread::sleep(Duration::from_secs(1));

    trigger(cfg)?;

    thread::sleep(Duration::from_secs(1));

    Ok(())
}

fn validate_instrument_connections(cfg: &Config) -> Result<()> {
    let mut scope = OscilloscopeHandler::initialize(cfg)
        .context("failed to connect to oscilloscope during automeasure preflight")?;
    scope
        .identify()
        .context("failed to identify oscilloscope during automeasure preflight")?;
    let mut generator = FGHandler::initialize(cfg)
        .context("failed to connect to generator during automeasure preflight")?;
    generator
        .identify()
        .context("failed to identify generator during automeasure preflight")?;
    Ok(())
}
