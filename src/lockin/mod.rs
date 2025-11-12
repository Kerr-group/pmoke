pub mod lockin_core;
pub mod lockin_params;
pub mod lockin_plot;
pub mod reference;
pub mod resolve;
pub mod save;
pub mod sensor;
pub mod stride;
pub mod time;

use crate::config::Config;
use crate::constants::{FETCHED_FNAME, HARMONICS, LI_HEADER, LI_RESULTS_NAME};
use crate::lockin::reference::fit::RefFitParams;
use crate::lockin::reference::run_reference;
use crate::lockin::save::{get_headers, write_li_results};
use crate::lockin::sensor::run_sensor;
use crate::lockin::time::time_builder;
use crate::utils::csv::read_csv;
use anyhow::{Context, Result, bail};
use rayon::prelude::*;

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

    let t = time_builder(cfg)?;

    run_li(cfg, &t, &data)?;

    Ok(())
}

pub fn run_li(cfg: &Config, t: &[f64], data: &[Vec<f64>]) -> Result<()> {
    let (sensor_ch, sensor_idx) = resolve::sensor_column_indices(cfg)?;
    let (_, ref_idx) = resolve::reference_column_index(cfg)?;
    let (signal_ch, signal_idx) = resolve::signal_column_indices(cfg)?;

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

    // Reference analysis
    let ref_fit_params = run_reference(t, &ref_data)?;
    drop(ref_data);

    // Sensor analysis
    let (t_stride, sensor_integral_stride) =
        run_sensor(cfg, t, &sensor_data, &sensor_ch, ref_fit_params.f_ref)?;
    drop(sensor_data);

    // Lock-in processing
    let result = li_process(cfg, t, &signal_data, ref_fit_params)?;
    drop(signal_data);

    // Save lock-in results
    let headers = get_headers(cfg, sensor_ch)?;
    let t0 = std::time::Instant::now();
    for (sig_ch, li_result) in signal_ch.iter().zip(result.iter()) {
        let li_result_fname = format!("{}_ch{}.csv", LI_RESULTS_NAME, sig_ch);
        write_li_results(
            &li_result_fname,
            &headers,
            &t_stride,
            &sensor_integral_stride,
            li_result,
        )?;
    }
    let elapsed_save = t0.elapsed();
    println!(
        "💾 Saved lock-in results for signals {:?} in {:.2?}",
        signal_ch, elapsed_save
    );

    let headers = LI_HEADER;
    let labels: Vec<String> = headers
        .iter()
        .map(|s| s.trim().replace("(V)", ""))
        .collect();

    lockin_plot::LIPlotter {}
        .plot(&t_stride, &result, &signal_ch, &labels)
        .context("failed to plot lock-in results")?;

    Ok(())
}

pub fn li_process(
    cfg: &Config,
    t: &[f64],
    signal_data: &[Vec<f64>],
    ref_fit_params: RefFitParams,
) -> Result<Vec<Vec<Vec<f64>>>> {
    let f_ref: f64 = ref_fit_params.f_ref;
    let omega_tref: f64 = ref_fit_params.omega_tref;
    let fil_length: usize = cfg.lockin.filter_length_samples;
    let stride: usize = cfg.lockin.stride_samples;
    let workers: usize = cfg.lockin.workers;

    let harmonics = HARMONICS;
    let ref_types = [lockin_core::RefType::Sin, lockin_core::RefType::Cos];

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .context("Failed to build rayon thread pool")?;

    println!("🔒 Starting lock-in processing with {} workers...", workers);
    let t0 = std::time::Instant::now();

    let mut all_signals_results: Vec<Vec<Vec<f64>>> = Vec::with_capacity(signal_data.len());

    for signal in signal_data.iter() {
        let li_processor =
            lockin_core::LockinProcessor::new(t, signal, f_ref, omega_tref, fil_length, stride)?;

        // [ (1,Sin), (1,Cos), (2,Sin), ... ]
        let mut args_list = Vec::new();
        for h in harmonics {
            for &r in &ref_types {
                args_list.push((h, r));
            }
        }

        let results_list: Vec<Vec<f64>> = pool.install(|| {
            args_list
                .par_iter()
                .map(|&(harmonic, ref_type)| li_processor.compute_lockin(harmonic, ref_type))
                .collect() // [LI1x, LI1y, LI2x, ...]
        });

        all_signals_results.push(results_list);
    }

    let elapsed_li = t0.elapsed();
    println!("🔒 Completed lock-in processing in {:.2?}", elapsed_li);

    Ok(all_signals_results)
}
