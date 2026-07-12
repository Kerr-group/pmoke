use crate::config::Config;
use crate::phase::run;
use anyhow::Result;

pub fn phase(cfg: &Config) -> Result<()> {
    let _lock = crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "phase")?;
    crate::plot::warn_canonical_plot_layout(cfg);
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
    let staging_cfg = crate::commands::run_dir::prepare_analysis_staging(
        cfg,
        crate::commands::run_dir::AnalysisStage::Phase,
    )?;
    run(&staging_cfg)?;
    crate::lockin::provenance::refresh_analysis_manifest_outputs(&staging_cfg, "phase")?;
    crate::commands::run_dir::publish_analysis_staging(cfg, &staging_cfg)
}
