use crate::config::Config;
use crate::config::ValidationTarget;
use crate::lockin::sensor::run;
use anyhow::Result;

pub fn sensor(cfg: &Config) -> Result<()> {
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock = crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "sensor")?;
    crate::config::validate_for_target(cfg, ValidationTarget::Sensor)?;
    crate::commands::run_dir::prepare_analysis_run(cfg)?;
    let result = sensor_inner(cfg);
    match &result {
        Ok(()) => {
            crate::commands::run_dir::write_run_state(cfg, "published", "sensor_complete", None)?
        }
        Err(error) => {
            crate::commands::run_dir::write_run_state(cfg, "failed", "sensor", Some(error))?
        }
    }
    result
}

fn sensor_inner(cfg: &Config) -> Result<()> {
    let staging_cfg = crate::commands::run_dir::prepare_analysis_staging(
        cfg,
        crate::commands::run_dir::AnalysisStage::Sensor,
    )?;
    crate::commands::run_dir::ensure_analysis_config_snapshots(&staging_cfg)?;
    crate::commands::run_dir::write_diagnostic_config_snapshots(&staging_cfg, "sensor")?;
    run(&staging_cfg)?;
    crate::commands::reference::refresh_manifest_if_present(&staging_cfg, "sensor")?;
    crate::commands::run_dir::publish_analysis_staging(cfg, &staging_cfg)
}
