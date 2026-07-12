use crate::config::Config;
use crate::config::ValidationTarget;
use crate::lockin::sensor::run;
use anyhow::Result;

pub fn sensor(cfg: &Config) -> Result<()> {
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock = crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "sensor")?;
    crate::config::validate_for_target(cfg, ValidationTarget::Sensor)?;
    crate::commands::run_dir::prepare(cfg)?;
    crate::commands::reference::require_analysis_manifest(cfg)?;

    let staging_cfg = crate::commands::run_dir::prepare_analysis_staging(
        cfg,
        crate::commands::run_dir::AnalysisStage::Sensor,
    )?;
    run(&staging_cfg)?;
    crate::commands::reference::refresh_manifest_if_present(&staging_cfg, "sensor")?;
    crate::commands::run_dir::publish_analysis_staging(cfg, &staging_cfg)
}
