use crate::config::Config;
use crate::phase::run;
use anyhow::Result;

pub fn phase(cfg: &Config) -> Result<()> {
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock = crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "phase")?;
    crate::config::validate_for_target(cfg, crate::config::ValidationTarget::Phase)?;
    crate::commands::run_dir::prepare_analysis_run(cfg)?;
    crate::commands::run_dir::write_run_state(cfg, "analyzing", "phase", None)?;
    let result = phase_inner(cfg);
    match &result {
        Ok(()) => {
            crate::commands::run_dir::write_run_state(cfg, "analyzing", "phase_complete", None)?
        }
        Err(error) => {
            crate::commands::run_dir::write_run_state(cfg, "failed", "phase", Some(error))?
        }
    }
    result
}

fn phase_inner(cfg: &Config) -> Result<()> {
    crate::lockin::provenance::validate_upstream_stage_config(cfg, "li")?;
    let staging_cfg = crate::commands::run_dir::prepare_analysis_staging(
        cfg,
        crate::commands::run_dir::AnalysisStage::Phase,
    )?;
    crate::commands::run_dir::write_analysis_config_snapshots(&staging_cfg)?;
    run(&staging_cfg)?;
    crate::lockin::provenance::refresh_analysis_manifest_outputs(&staging_cfg, "phase")?;
    crate::commands::run_dir::publish_analysis_staging(cfg, &staging_cfg)
}
