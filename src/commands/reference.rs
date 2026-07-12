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
    require_analysis_manifest(cfg)?;

    let staging_cfg = crate::commands::run_dir::prepare_analysis_staging(
        cfg,
        crate::commands::run_dir::AnalysisStage::Reference,
    )?;
    run(&staging_cfg)?;
    refresh_manifest_if_present(&staging_cfg, "reference")?;
    crate::commands::run_dir::publish_analysis_staging(cfg, &staging_cfg)
}

pub fn require_analysis_manifest(cfg: &Config) -> Result<()> {
    let manifest = cfg.paths().analysis_manifest();
    if !manifest.is_file() {
        anyhow::bail!(
            "standalone reference/sensor requires an existing \
             analysis manifest; run pmoke li first"
        );
    }
    Ok(())
}

pub(crate) fn refresh_manifest_if_present(cfg: &Config, stage: &str) -> Result<()> {
    require_analysis_manifest(cfg)?;
    crate::lockin::provenance::refresh_analysis_manifest_outputs(cfg, stage)?;
    Ok(())
}
