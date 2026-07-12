use crate::config::Config;
use crate::config::ValidationTarget;
use crate::lockin::reference::run;
use anyhow::Result;

pub fn reference(cfg: &Config) -> Result<()> {
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock =
        crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "reference")?;
    crate::config::validate_for_target(cfg, ValidationTarget::Reference)?;
    crate::commands::run_dir::prepare(cfg)?;

    let staging_cfg = crate::commands::run_dir::prepare_analysis_staging(
        cfg,
        crate::commands::run_dir::AnalysisStage::Reference,
    )?;
    run(&staging_cfg)?;
    refresh_manifest_if_present(&staging_cfg, "reference")?;
    crate::commands::run_dir::publish_analysis_staging(cfg, &staging_cfg)
}

pub(crate) fn refresh_manifest_if_present(cfg: &Config, stage: &str) -> Result<()> {
    if cfg.paths().analysis_manifest().is_file() {
        crate::lockin::provenance::refresh_analysis_manifest_outputs(cfg, stage)?;
    }
    Ok(())
}
