use crate::{
    config::Config,
    constants::FETCHED_FNAME,
    kerr::run_kerr_analysis,
    lockin::{run_li, time::time_builder},
    phase::run_phase_analysis,
    utils::csv::read_csv,
};
use anyhow::{Result, bail};

pub fn analyze(cfg: &Config) -> Result<()> {
    let t0 = std::time::Instant::now();
    let data = read_csv(FETCHED_FNAME)?;
    let elapsed_read = t0.elapsed();

    println!(
        "📥 Read fetched data ({} rows, {} columns) in {:.2?}",
        data.len(),
        if data.is_empty() { 0 } else { data[0].len() },
        elapsed_read
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
    let ch = &cfg.phase.use_signal_ch;

    if ch.is_empty() {
        println!("⚠️ No channels specified for phase analysis. Skipping phase analysis.");
        return Ok(());
    }
    let li_rotated_results =
        run_phase_analysis(cfg, &t_stride, &sensor_integral_stride, &li_results)?;
    drop(li_results);

    // run Kerr analysis here
    run_kerr_analysis(cfg, &t_stride, &sensor_integral_stride, &li_rotated_results)?;

    Ok(())
}
