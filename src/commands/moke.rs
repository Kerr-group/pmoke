use crate::{config::Config, moke::run};
use anyhow::Result;

pub fn moke(cfg: &Config) -> Result<()> {
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock = crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "moke")?;
    crate::config::validate_for_target(cfg, crate::config::ValidationTarget::Moke)?;
    crate::commands::run_dir::prepare_analysis_run(cfg)?;
    crate::commands::run_dir::write_run_state(cfg, "analyzing", "moke", None)?;
    let result = moke_inner(cfg);
    match &result {
        Ok(()) => crate::commands::run_dir::write_run_state(cfg, "complete", "moke", None)?,
        Err(error) => {
            crate::commands::run_dir::write_run_state(cfg, "failed", "moke", Some(error))?
        }
    }
    result
}

fn moke_inner(cfg: &Config) -> Result<()> {
    crate::lockin::provenance::validate_upstream_stage_config(cfg, "phase")?;
    let staging_cfg = crate::commands::run_dir::prepare_analysis_staging(
        cfg,
        crate::commands::run_dir::AnalysisStage::Moke,
    )?;
    crate::commands::run_dir::write_analysis_config_snapshots(&staging_cfg)?;
    run(&staging_cfg)?;
    crate::lockin::provenance::refresh_analysis_manifest_outputs(&staging_cfg, "moke")?;
    crate::commands::run_dir::publish_analysis_staging(cfg, &staging_cfg)
}
