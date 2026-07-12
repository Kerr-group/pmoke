use crate::{config::Config, kerr::run};
use anyhow::Result;

pub fn kerr(cfg: &Config) -> Result<()> {
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock = crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "kerr")?;
    crate::config::validate_for_target(cfg, crate::config::ValidationTarget::Kerr)?;
    crate::commands::run_dir::prepare(cfg)?;
    crate::plot::warn_canonical_plot_layout(cfg);
    crate::commands::run_dir::write_run_state(cfg, "analyzing", "kerr", None)?;
    let result = kerr_inner(cfg);
    match &result {
        Ok(()) => crate::commands::run_dir::write_run_state(cfg, "complete", "kerr", None)?,
        Err(error) => {
            crate::commands::run_dir::write_run_state(cfg, "failed", "kerr", Some(error))?
        }
    }
    result
}

fn kerr_inner(cfg: &Config) -> Result<()> {
    let staging_cfg = crate::commands::run_dir::prepare_analysis_staging(
        cfg,
        crate::commands::run_dir::AnalysisStage::Kerr,
    )?;
    run(&staging_cfg)?;
    crate::lockin::provenance::refresh_analysis_manifest_outputs(&staging_cfg, "kerr")?;
    crate::commands::run_dir::publish_analysis_staging(cfg, &staging_cfg)
}
