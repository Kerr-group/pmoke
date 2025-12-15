pub mod ref_analysis;
pub mod ref_plot;

use std::f64::consts::PI;

use crate::config::Config;
use crate::constants::FETCHED_FNAME;
use crate::lockin::reference::ref_analysis::{RefFitParams, ReferenceFFT, ReferenceFitter};
use crate::lockin::time::time_builder;
use crate::utils::channels::build_channel_list;
use crate::utils::csv::read_selected_columns;
use anyhow::{Context, Result, anyhow, bail};

pub fn run(cfg: &Config) -> Result<()> {
    let t = time_builder(cfg)?;
    let _ = run_fit_ref(cfg, &t)?;
    Ok(())
}

pub fn run_fit_ref(cfg: &Config, t: &[f64]) -> Result<RefFitParams> {
    let ref_ch = extract_single_reference_ch(cfg)?;

    let channels = build_channel_list(cfg)?;
    let col_idx = channels
        .iter()
        .position(|ch| *ch == ref_ch)
        .ok_or_else(|| {
            anyhow!(
                "reference channel {} not found in fetched channels {:?}",
                ref_ch,
                channels
            )
        })?;

    let t0 = std::time::Instant::now();
    let ref_data = read_selected_columns(FETCHED_FNAME, &[col_idx])
        .context("failed to read reference column from csv")?
        .pop()
        .ok_or_else(|| {
            anyhow!(
                "read_selected_columns returned no data for column index {}",
                col_idx
            )
        })?;
    let elapsed_read = t0.elapsed();
    println!(
        "📥 Read reference column {} in {:.2?}",
        col_idx + 1,
        elapsed_read
    );

    let results = run_fit_ref_core(cfg, t, &ref_data).context("failed to fit reference signal")?;

    Ok(results)
}

fn stride_samples(cfg: &Config, t: &[f64], ref_data: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let stride_samples = cfg.reference.stride_samples;
    let window_samples = cfg.reference.window_samples;

    let block_len = window_samples + 1;
    let estimated_capacity = (t.len() / stride_samples + 1) * block_len;

    let mut fit_t = Vec::with_capacity(estimated_capacity);
    let mut fit_ref_data = Vec::with_capacity(estimated_capacity);

    let mut i = 0;
    while i < t.len() {
        let end = (i + block_len).min(t.len());

        fit_t.extend_from_slice(&t[i..end]);
        fit_ref_data.extend_from_slice(&ref_data[i..end]);

        i += stride_samples;
    }

    (fit_t, fit_ref_data)
}

pub fn run_fit_ref_core(cfg: &Config, t: &[f64], ref_data: &[f64]) -> Result<RefFitParams> {
    let fft_t_start = cfg.reference.fft_window.start;
    let fft_t_end = cfg.reference.fft_window.end;

    let (idx_start, idx_end) = get_range_indices(t, fft_t_start, fft_t_end);

    let fft_t = &t[idx_start..idx_end];
    let fft_ref_data = &ref_data[idx_start..idx_end];
    let fft_results = fft_ref(fft_t, fft_ref_data).context("failed to fft reference signal")?;

    println!(
        "🔍 Reference FFT results: f_ref = {:.6} MHz, A_ref = {:.6}, omega_tref = {:.6} rad",
        fft_results.f_ref * 1e-6,
        fft_results.a_ref,
        fft_results.omega_tref
    );

    let (fit_t, fit_ref_data) = stride_samples(cfg, t, ref_data);
    let results =
        fit_ref(&fit_t, &fit_ref_data, fft_results).context("failed to fit reference signal")?;
    plot_fit_results(&fit_t, &fit_ref_data, &results).context("failed to plot reference signal")?;
    Ok(results)
}

fn fft_ref(t: &[f64], ref_data: &[f64]) -> Result<RefFitParams> {
    if t.len() != ref_data.len() {
        bail!(
            "time length ({}) and reference length ({}) differ",
            t.len(),
            ref_data.len()
        );
    }

    if t.is_empty() {
        println!("(Info) Time and reference data are empty. Skipping fft.");
        bail!("Cannot fft empty data.");
    }

    let results = ReferenceFFT {}
        .fft(t, ref_data)
        .context("failed to fft reference signal")?;

    Ok(results)
}

fn fit_ref(t: &[f64], ref_data: &[f64], params: RefFitParams) -> Result<RefFitParams> {
    if t.len() != ref_data.len() {
        bail!(
            "time length ({}) and reference length ({}) differ",
            t.len(),
            ref_data.len()
        );
    }

    if t.is_empty() {
        println!("(Info) Time and reference data are empty. Skipping fit.");
        bail!("Cannot fit empty data.");
    }

    let results = ReferenceFitter {}
        .fit(t, ref_data, params)
        .context("failed to fit reference signal")?;

    Ok(results)
}

fn get_range_indices(t: &[f64], start: f64, end: f64) -> (usize, usize) {
    let idx_start = t.iter().position(|&ti| ti >= start).unwrap_or(0);
    let idx_end = t.iter().position(|&ti| ti > end).unwrap_or(t.len());

    if idx_start >= idx_end {
        panic!(
            "Invalid range: start index {} is not less than end index {}",
            idx_start, idx_end
        );
    }

    (idx_start, idx_end)
}

fn plot_fit_results(t: &[f64], ref_data: &[f64], results: &RefFitParams) -> Result<()> {
    if t.is_empty() {
        println!("(Info) No data to plot.");
        return Ok(());
    }

    let f = results.f_ref;
    let a = results.a_ref;
    let omegat = results.omega_tref;

    if f == 0.0 {
        bail!("Reference frequency is zero, cannot plot results.");
    }

    let t_period = 1.0 / f;
    let t_start_plot = 0.0;
    let t_end_plot = 3.0 * t_period;

    let (idx_start, idx_end) = get_range_indices(t, t_start_plot, t_end_plot);

    let t_plot = &t[idx_start..idx_end];
    let ref_plot = &ref_data[idx_start..idx_end];
    // -------------------------

    let fit_plot: Vec<f64> = t_plot
        .iter()
        .map(|&ti| a * (2.0 * PI * f * ti - omegat).sin())
        .collect();

    ref_plot::ReferencePlotter {}
        .plot(t_plot, ref_plot, &fit_plot)
        .context("failed to plot reference signal")?;

    Ok(())
}

fn extract_single_reference_ch(cfg: &Config) -> Result<u8> {
    match cfg.roles.reference_ch.len() {
        0 => bail!("reference channel is not specified in the configuration"),
        1 => Ok(cfg.roles.reference_ch[0]),
        _ => bail!("multiple reference channels are not supported"),
    }
}
