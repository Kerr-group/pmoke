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
use crate::ui;
use crate::utils::csv::read_csv;
use anyhow::{bail, Context, Result};
use rayon::prelude::*;

pub struct LockinProcessOutput {
    pub result: Vec<Vec<Vec<f64>>>,
    pub base_index_range: (usize, usize),
    pub output_index_range: (usize, usize),
}

pub fn run(cfg: &Config) -> Result<()> {
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

    let sensor_data: Vec<&[f64]> = sensor_idx.iter().map(|&idx| data[idx].as_slice()).collect();
    let ref_data = data[ref_idx].as_slice();
    let signal_data: Vec<&[f64]> = signal_idx.iter().map(|&idx| data[idx].as_slice()).collect();

    // Reference analysis
    let ref_fit_params = run_fit_ref_core(cfg, t, ref_data)?;

    // Sensor analysis
    let (mut t_stride, mut sensor_integral_stride) =
        run_sensor(cfg, t, &sensor_data, &sensor_ch, ref_fit_params.f_ref)?;

    // Lock-in processing
    let lockin_output = li_process(cfg, t, &signal_ch, &signal_data, ref_fit_params)?;
    trim_lockin_context_to_result(
        &mut t_stride,
        &mut sensor_integral_stride,
        &lockin_output.result,
        lockin_output.base_index_range,
        lockin_output.output_index_range,
    )?;

    // Save lock-in results
    let headers = get_li_headers(cfg)?;
    let t0 = std::time::Instant::now();
    for (sig_ch, li_result) in signal_ch.iter().zip(lockin_output.result.iter()) {
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
    ui::saved(format!(
        "lock-in results for signals {:?} ({})",
        signal_ch,
        ui::fmt_duration(elapsed_save)
    ));

    let headers = LI_HEADER;
    let labels: Vec<String> = headers
        .iter()
        .map(|s| s.trim().replace("(V)", ""))
        .collect();

    lockin_plot::LIPlotter {}
        .plot(&t_stride, &lockin_output.result, &signal_ch, &labels)
        .context("failed to plot lock-in results")?;

    Ok((t_stride, sensor_integral_stride, lockin_output.result))
}

pub fn li_process(
    cfg: &Config,
    t: &[f64],
    signal_ch: &[u8],
    signal_data: &[&[f64]],
    ref_fit_params: RefFitParams,
) -> Result<LockinProcessOutput> {
    let f_ref: f64 = ref_fit_params.f_ref;
    let omega_tref: f64 = ref_fit_params.omega_tref;
    let workers: usize = cfg.lockin.workers;

    let harmonics = HARMONICS;

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build()
        .context("Failed to build rayon thread pool")?;

    let pb = ui::progress(
        format!("lock-in processing with {workers} workers"),
        (signal_data.len() * harmonics.len()) as u64,
    );
    let t0 = std::time::Instant::now();

    let mut all_signals_results: Vec<Vec<Vec<f64>>> = Vec::with_capacity(signal_data.len());
    let mut printed_lockin_summary = false;
    let mut base_index_range = None;
    let mut output_index_range = None;

    for (&sig_ch, signal) in signal_ch.iter().zip(signal_data.iter()) {
        pb.set_message(format!("lock-in ch{sig_ch}"));
        let li_processor =
            lockin_core::LockinProcessor::new(t, signal, f_ref, omega_tref, &cfg.lockin)?;
        let processor_base_range = li_processor.base_index_range();
        let processor_output_range = li_processor.output_index_range();
        if let Some(expected) = base_index_range {
            if processor_base_range != expected {
                bail!(
                    "lock-in base index range mismatch for signal ch{sig_ch}: {:?}, expected {:?}",
                    processor_base_range,
                    expected
                );
            }
        } else {
            base_index_range = Some(processor_base_range);
        }
        if let Some(expected) = output_index_range {
            if processor_output_range != expected {
                bail!(
                    "lock-in output index range mismatch for signal ch{sig_ch}: {:?}, expected {:?}",
                    processor_output_range,
                    expected
                );
            }
        } else {
            output_index_range = Some(processor_output_range);
        }
        if !printed_lockin_summary {
            ui::suspend_progress(&pb, || {
                ui::section("Lock-in settings");
                for line in li_processor.summary_lines() {
                    ui::bullet(line);
                }
            });
            printed_lockin_summary = true;
        }
        let include_debug = cfg.lockin.lpf_debug_output;

        let harmonic_results: Vec<lockin_core::HarmonicLockinResult> = if include_debug {
            let t_output = li_processor.output_times();
            let params = li_processor.params();
            let filter = li_processor.filter_design();
            let mut results = Vec::with_capacity(harmonics.len());
            for &harmonic in &harmonics {
                pb.set_message(format!("lock-in ch{sig_ch} h{harmonic}"));
                let result = li_processor.compute_harmonic_detailed(harmonic, include_debug);
                if include_debug {
                    debug::write_harmonic_debug(
                        cfg, sig_ch, harmonic, params, filter, t, &t_output, &result,
                    )
                    .with_context(|| {
                        format!("failed to write lock-in debug output for ch{sig_ch} h{harmonic}")
                    })?;
                    results.push(result.without_debug_data());
                } else {
                    results.push(result);
                }
                pb.inc(1);
            }
            results
        } else {
            let progress = pb.clone();
            pool.install(|| {
                harmonics
                    .par_iter()
                    .map(|&harmonic| {
                        progress.set_message(format!("lock-in ch{sig_ch} h{harmonic}"));
                        let result = li_processor.compute_harmonic_detailed(harmonic, false);
                        progress.inc(1);
                        result
                    })
                    .collect()
            })
        };

        let mut results_list = Vec::with_capacity(harmonic_results.len() * 2);
        for result in harmonic_results {
            results_list.push(result.li_x);
            results_list.push(result.li_y);
        }

        all_signals_results.push(results_list);
    }

    let elapsed_li = t0.elapsed();
    ui::finish_success(
        pb,
        format!(
            "lock-in processing completed ({})",
            ui::fmt_duration(elapsed_li)
        ),
    );

    Ok(LockinProcessOutput {
        result: all_signals_results,
        base_index_range: base_index_range.unwrap_or((0, 0)),
        output_index_range: output_index_range.unwrap_or((0, 0)),
    })
}

fn trim_lockin_context_to_result(
    t_stride: &mut Vec<f64>,
    sensor_integral_stride: &mut [Vec<f64>],
    result: &[Vec<Vec<f64>>],
    base_index_range: (usize, usize),
    output_index_range: (usize, usize),
) -> Result<()> {
    let Some(first_signal) = result.first() else {
        return Ok(());
    };
    let Some(first_column) = first_signal.first() else {
        return Ok(());
    };
    let target_len = first_column.len();
    let (base_start, base_end) = base_index_range;
    let (output_start, output_end) = output_index_range;
    if output_start > output_end {
        bail!(
            "lock-in output index range {:?} is empty or reversed",
            output_index_range
        );
    }
    if output_start < base_start || output_end > base_end {
        bail!(
            "lock-in output index range {:?} is outside base stride range {:?}",
            output_index_range,
            base_index_range
        );
    }
    let expected_base_len = base_end
        .checked_sub(base_start)
        .map(|span| span + 1)
        .unwrap_or(0);
    if t_stride.len() != expected_base_len {
        bail!(
            "time stride length ({}) does not match base stride range {:?} length ({expected_base_len})",
            t_stride.len(),
            base_index_range
        );
    }
    let expected_len = output_end
        .checked_sub(output_start)
        .map(|span| span + 1)
        .unwrap_or(0);
    if target_len != expected_len {
        bail!(
            "lock-in result length ({target_len}) does not match output index range {:?} length ({expected_len})",
            output_index_range
        );
    }
    for (signal_idx, signal) in result.iter().enumerate() {
        for (col_idx, column) in signal.iter().enumerate() {
            if column.len() != target_len {
                bail!(
                    "lock-in result length mismatch: signal {signal_idx} column {col_idx} has {}, expected {target_len}",
                    column.len()
                );
            }
        }
    }
    if target_len > t_stride.len() {
        bail!(
            "lock-in result length ({target_len}) exceeds time stride length ({})",
            t_stride.len()
        );
    }
    for col in sensor_integral_stride.iter() {
        if col.len() != t_stride.len() {
            bail!(
                "sensor stride length ({}) does not match time stride length ({})",
                col.len(),
                t_stride.len()
            );
        }
    }
    if target_len == t_stride.len() {
        return Ok(());
    }

    let trim_front = output_start - base_start;
    t_stride.drain(..trim_front);
    t_stride.truncate(target_len);
    for col in sensor_integral_stride {
        if target_len > col.len() {
            bail!(
                "lock-in result length ({target_len}) exceeds sensor stride length ({})",
                col.len()
            );
        }
        col.drain(..trim_front);
        col.truncate(target_len);
    }
    Ok(())
}
