use crate::config::Config;
use crate::lockin::run_li;
use crate::signal::run_signal_analysis;
use crate::utils::waveform::read_all_fetched_waveforms;
use anyhow::{Result, bail};

pub fn signal(cfg: &Config) -> Result<()> {
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock = crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "signal")?;
    crate::config::validate_for_target(cfg, crate::config::ValidationTarget::Signal)?;
    crate::commands::run_dir::prepare_analysis_run(cfg)?;
    crate::commands::run_dir::write_run_state(cfg, "analyzing", "signal", None)?;
    let result = signal_inner(cfg);
    match &result {
        Ok(()) => {
            crate::commands::run_dir::write_run_state(cfg, "analyzing", "signal_complete", None)?
        }
        Err(error) => {
            crate::commands::run_dir::write_run_state(cfg, "failed", "signal", Some(error))?
        }
    }
    result
}

fn signal_inner(cfg: &Config) -> Result<()> {
    if cfg.signals.is_empty() {
        bail!("no [[signals]] entries are configured");
    }
    let data = read_all_fetched_waveforms(cfg)?;
    if data.channels.is_empty() {
        bail!("fetched data is empty, cannot run signal averaging");
    }
    crate::commands::analyze::validate_waveform_data(&data)?;
    let staging_cfg = crate::commands::run_dir::prepare_analysis_staging(
        cfg,
        crate::commands::run_dir::AnalysisStage::Signal,
    )?;
    crate::commands::run_dir::write_analysis_config_snapshots(&staging_cfg)?;
    let (t_stride, sensor_rate_stride, sensor_integral_stride, _, reference, provenance) =
        run_li(&staging_cfg, &data.t, &data.channels)?;
    run_signal_analysis(
        &staging_cfg,
        &t_stride,
        &sensor_rate_stride,
        &sensor_integral_stride,
        &data.t,
        &data.channels,
        reference.f_ref,
    )?;
    crate::lockin::provenance::write_analysis_metadata(
        &staging_cfg,
        &staging_cfg.paths(),
        &cfg.resolver(),
        &reference,
        &provenance,
        staging_cfg.roles.reference_ch,
    )?;
    crate::lockin::provenance::refresh_analysis_manifest_outputs(&staging_cfg, "signal")?;
    crate::commands::run_dir::publish_analysis_staging(cfg, &staging_cfg)
}
