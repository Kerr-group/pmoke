use crate::config::Config;
use crate::lockin::run_li;
use crate::utils::waveform::read_all_fetched_waveforms;
use anyhow::{Result, bail};

pub fn li(cfg: &Config) -> Result<()> {
    let _lock = crate::commands::run_dir::AnalysisLock::acquire(&cfg.paths().run_dir, "li")?;
    crate::plot::warn_canonical_plot_layout(cfg);
    crate::commands::run_dir::write_run_state(cfg, "analyzing", "li", None)?;
    let result = li_inner(cfg);
    match &result {
        Ok(()) => {
            crate::commands::run_dir::write_run_state(cfg, "analyzing", "li_complete", None)?
        }
        Err(error) => {
            crate::commands::run_dir::write_run_state(cfg, "failed", "li", Some(error))?
        }
    }
    result
}

fn li_inner(cfg: &Config) -> Result<()> {
    let data = read_all_fetched_waveforms(cfg)?;
    if data.channels.is_empty() {
        bail!("fetched data is empty, cannot run lock-in analysis");
    }
    crate::commands::analyze::validate_waveform_data(&data)?;
    let staging_cfg = crate::commands::run_dir::prepare_analysis_staging(
        cfg,
        crate::commands::run_dir::AnalysisStage::Li,
    )?;
    let (_, _, _, _, reference, provenance) = run_li(&staging_cfg, &data.t, &data.channels)?;
    crate::lockin::provenance::write_analysis_metadata(
        &staging_cfg.paths(),
        &cfg.resolver(),
        &reference,
        &provenance,
        staging_cfg.roles.reference_ch,
    )?;
    crate::commands::run_dir::publish_analysis_staging(cfg, &staging_cfg)
}
