use crate::{
    config::Config,
    kerr::run_kerr_analysis,
    lockin::run_li,
    phase::run_phase_analysis,
    ui,
    utils::waveform::{WaveformData, read_all_fetched_waveforms},
};
use anyhow::{Result, bail};

pub fn analyze(cfg: &Config) -> Result<()> {
    let pb = ui::spinner("reading fetched waveform data");
    let t0 = std::time::Instant::now();
    let data = read_all_fetched_waveforms(cfg)?;
    let elapsed_read = t0.elapsed();

    ui::finish_read(
        pb,
        format!(
            "fetched data: {} rows, {} columns ({})",
            data.channels.len(),
            data.channels.first().map_or(0, Vec::len),
            ui::fmt_duration(elapsed_read)
        ),
    );

    if data.channels.is_empty() {
        bail!("Fetched data is empty, cannot extract columns.");
    }

    run_analyze(cfg, data)?;
    Ok(())
}

pub fn run_analyze(cfg: &Config, data: WaveformData) -> Result<()> {
    let (t_stride, sensor_integral_stride, li_results) = run_li(cfg, &data.t, &data.channels)?;

    // run phase analysis here
    let ch = cfg.phase_signal_ch();

    if ch.is_empty() {
        ui::skipped("phase analysis: no channels specified");
        return Ok(());
    }
    let li_rotated_results =
        run_phase_analysis(cfg, &t_stride, &sensor_integral_stride, &li_results)?;
    drop(li_results);

    // run Kerr analysis here
    run_kerr_analysis(cfg, &t_stride, &sensor_integral_stride, &li_rotated_results)?;

    Ok(())
}
