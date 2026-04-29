pub mod debug;
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
use crate::lockin::reference::ref_analysis::RefFitParams;
use crate::lockin::reference::run_fit_ref_core;
use crate::lockin::save::{get_li_headers, write_li_results};
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

    let _ = run_li(cfg, &t, &data)?;

    Ok(())
}

pub fn run_li(
    cfg: &Config,
    t: &[f64],
    data: &[Vec<f64>],
) -> Result<(Vec<f64>, Vec<Vec<f64>>, Vec<Vec<Vec<f64>>>)> {
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
    let ref_fit_params = run_fit_ref_core(cfg, t, &ref_data)?;
    drop(ref_data);

    // Sensor analysis
    let (t_stride, sensor_integral_stride) =
        run_sensor(cfg, t, &sensor_data, &sensor_ch, ref_fit_params.f_ref)?;
    drop(sensor_data);

    // Lock-in processing
    let result = li_process(cfg, t, &signal_ch, &signal_data, ref_fit_params)?;
    drop(signal_data);

    // Save lock-in results
    let headers = get_li_headers(cfg)?;
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

    Ok((t_stride, sensor_integral_stride, result))
}

pub fn li_process(
    cfg: &Config,
    t: &[f64],
    signal_ch: &[u8],
    signal_data: &[Vec<f64>],
    ref_fit_params: RefFitParams,
) -> Result<Vec<Vec<Vec<f64>>>> {
    let f_ref: f64 = ref_fit_params.f_ref;
    let omega_tref: f64 = ref_fit_params.omega_tref;
    let workers: usize = cfg.lockin.workers;

    let harmonics = HARMONICS;

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .context("Failed to build rayon thread pool")?;

    println!("🔒 Starting lock-in processing with {} workers...", workers);
    let t0 = std::time::Instant::now();

    let mut all_signals_results: Vec<Vec<Vec<f64>>> = Vec::with_capacity(signal_data.len());

    for (&sig_ch, signal) in signal_ch.iter().zip(signal_data.iter()) {
        let li_processor =
            lockin_core::LockinProcessor::new(t, signal, f_ref, omega_tref, &cfg.lockin)?;
        let include_debug = cfg.lockin.lpf_debug_output;

        let harmonic_results: Vec<lockin_core::HarmonicLockinResult> = pool.install(|| {
            harmonics
                .par_iter()
                .map(|&harmonic| li_processor.compute_harmonic_detailed(harmonic, include_debug))
                .collect()
        });

        if include_debug {
            let t_output = li_processor.output_times();
            let params = li_processor.params();
            let filter = li_processor.filter_design();
            for (&harmonic, result) in harmonics.iter().zip(harmonic_results.iter()) {
                debug::write_harmonic_debug(
                    cfg, sig_ch, harmonic, params, filter, t, &t_output, result,
                )
                .with_context(|| {
                    format!("failed to write lock-in debug output for ch{sig_ch} h{harmonic}")
                })?;
            }
        }

        let mut results_list = Vec::with_capacity(harmonic_results.len() * 2);
        for result in harmonic_results {
            results_list.push(result.li_x);
            results_list.push(result.li_y);
        }

        all_signals_results.push(results_list);
    }

    let elapsed_li = t0.elapsed();
    println!("🔒 Completed lock-in processing in {:.2?}", elapsed_li);

    Ok(all_signals_results)
}
