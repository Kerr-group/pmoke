use crate::{
    config::Config,
    constants::FETCHED_FNAME,
    kerr::run_kerr_analysis,
    lockin::{run_li, time::time_builder},
    phase::run_phase_analysis,
    ui,
    utils::csv::read_csv,
};
use anyhow::{Result, bail};

pub fn analyze(cfg: &Config) -> Result<()> {
    let pb = ui::spinner(format!("reading {FETCHED_FNAME}"));
    let t0 = std::time::Instant::now();
    let data = read_csv(FETCHED_FNAME)?;
    let elapsed_read = t0.elapsed();

    ui::finish_read(
        pb,
        format!(
            "fetched data: {} rows, {} columns ({})",
            data.len(),
            if data.is_empty() { 0 } else { data[0].len() },
            ui::fmt_duration(elapsed_read)
        ),
    );

    if data.is_empty() {
        bail!("Fetched data is empty, cannot extract columns.");
    }

    let t = time_builder(cfg)?;

    run_analyze(cfg, t, data)?;
    Ok(())
}

pub fn run_analyze(cfg: &Config, t: Vec<f64>, data: Vec<Vec<f64>>) -> Result<()> {
    let (t_stride, sensor_integral_stride, li_results) = run_li(cfg, &t, &data)?;
    drop(t);

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
