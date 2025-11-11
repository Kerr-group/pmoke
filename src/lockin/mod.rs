pub mod reference;
pub mod resolve;
pub mod sensor;
pub mod time;

// use std::f64::consts::PI;

use crate::config::Config;
use crate::constants::FETCHED_FNAME;
use crate::lockin::reference::fit::{RefFitParams, ReferenceHandler};
use crate::lockin::reference::run_reference;
use crate::lockin::sensor::run_sensor;
use crate::lockin::time::time_builder;
use crate::utils::csv::read_csv;
use anyhow::{Context, Result, anyhow, bail};

pub fn run(cfg: &Config) -> Result<()> {
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

    let (sensor_ch, sensor_idx) = resolve::sensor_column_indices(cfg)?;
    let (_, ref_idx) = resolve::reference_column_index(cfg)?;
    let (_, signal_idx) = resolve::signal_column_indices(cfg)?;

    let max_sensor_idx = sensor_idx.iter().max().cloned().unwrap_or(0);
    let max_signal_idx = signal_idx.iter().max().cloned().unwrap_or(0);
    let max_needed_idx = std::cmp::max(max_sensor_idx, std::cmp::max(ref_idx, max_signal_idx));

    if max_needed_idx >= data.len() {
        bail!(
            "Configuration error: required channel index {} is out of bounds. CSV only has {} columns.",
            max_needed_idx,
            data.len()
        );
    }

    let sensor_data: Vec<Vec<f64>> = sensor_idx.iter().map(|&idx| data[idx].clone()).collect();
    let ref_data: Vec<f64> = data[ref_idx].clone();
    let signal_data: Vec<Vec<f64>> = signal_idx.iter().map(|&idx| data[idx].clone()).collect();
    let t = time_builder(cfg)?;

    // Reference analysis
    let ref_fit_params = run_reference(&t, &ref_data)?;

    // Sensor analysis
    let (t_stride, sensor_integral_stride) = run_sensor(cfg, &t, &sensor_data, &sensor_ch)?;

    Ok(())
}

pub fn run_li(cfg: &Config) -> Result<()> {
    Ok(())
}
