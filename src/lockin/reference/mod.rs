pub mod ref_analysis;
pub mod ref_plot;

use std::f64::consts::PI;

use crate::config::Config;
use crate::lockin::reference::ref_analysis::{RefFitParams, ReferenceFFT, ReferenceFitter};
use crate::utils::time_axis::TimeAxisRef;
use crate::utils::waveform::read_waveform_channels;
use crate::{plot, ui};
use anyhow::{Context, Result, bail};

pub fn run(cfg: &Config) -> Result<()> {
    let _ = run_fit_ref(cfg)?;
    Ok(())
}

pub fn run_fit_ref(cfg: &Config) -> Result<RefFitParams> {
    let ref_ch = extract_single_reference_ch(cfg)?;

    let pb = ui::spinner(format!("reading reference channel {ref_ch}"));
    let t0 = std::time::Instant::now();
    let waveform =
        read_waveform_channels(cfg, &[ref_ch]).context("failed to read reference channel")?;
    let ref_data =
        waveform.channels.into_iter().next().ok_or_else(|| {
            anyhow::anyhow!("read_waveform_channels returned no data for ch{ref_ch}")
        })?;
    let elapsed_read = t0.elapsed();
    ui::finish_read(
        pb,
        format!(
            "reference channel {ref_ch} ({})",
            ui::fmt_duration(elapsed_read)
        ),
    );

    let results =
        run_fit_ref_core(cfg, &waveform.t, &ref_data).context("failed to fit reference signal")?;

    Ok(results)
}

fn stride_samples(cfg: &Config, t: TimeAxisRef<'_>, ref_data: &[f64]) -> (Vec<f64>, Vec<f64>) {
    let stride_samples = cfg.reference.stride_samples;
    let window_samples = cfg.reference.window_samples;

    let block_len = window_samples + 1;
    let estimated_capacity = (t.len() / stride_samples + 1) * block_len;

    let mut fit_t = Vec::with_capacity(estimated_capacity);
    let mut fit_ref_data = Vec::with_capacity(estimated_capacity);

    let mut i = 0;
    while i < t.len() {
        let end = (i + block_len).min(t.len());

        fit_t.extend((i..end).map(|index| t.value_at(index)));
        fit_ref_data.extend_from_slice(&ref_data[i..end]);

        i += stride_samples;
    }

    (fit_t, fit_ref_data)
}

pub fn run_fit_ref_core<'a>(
    cfg: &Config,
    t: impl Into<TimeAxisRef<'a>>,
    ref_data: &[f64],
) -> Result<RefFitParams> {
    let t = t.into();
    if t.len() != ref_data.len() {
        bail!(
            "time length ({}) and reference length ({}) differ",
            t.len(),
            ref_data.len()
        );
    }
    let fft_t_start = cfg.reference.fft_window.start;
    let fft_t_end = cfg.reference.fft_window.end;

    let (idx_start, idx_end) = get_range_indices(t, fft_t_start, fft_t_end)?;

    let fft_ref_data = &ref_data[idx_start..idx_end];
    if fft_ref_data.len() < 2 {
        bail!("reference FFT window must contain at least two samples");
    }
    let fft_dt = t.value_at(idx_start + 1) - t.value_at(idx_start);
    let fft_results = fft_ref(fft_dt, fft_ref_data).context("failed to fft reference signal")?;

    ui::info(format!(
        "reference FFT: f_ref = {:.6} MHz, A_ref = {:.6}, omega_tref = {:.6} rad",
        fft_results.f_ref * 1e-6,
        fft_results.a_ref,
        fft_results.omega_tref
    ));

    let (fit_t, fit_ref_data) = stride_samples(cfg, t, ref_data);
    let results =
        fit_ref(&fit_t, &fit_ref_data, fft_results).context("failed to fit reference signal")?;
    ui::summary_table(
        "Reference fit",
        &["Metric", "Value"],
        vec![
            vec![
                "frequency".to_string(),
                format!("{:.8} MHz", results.f_ref * 1e-6),
            ],
            vec!["amplitude".to_string(), format!("{:.8} V", results.a_ref)],
            vec![
                "phase".to_string(),
                format!("{:.8} rad", results.omega_tref),
            ],
        ],
    );
    plot_fit_results(cfg, &fit_t, &fit_ref_data, &results)
        .context("failed to plot reference signal")?;
    Ok(results)
}

fn fft_ref(dt: f64, ref_data: &[f64]) -> Result<RefFitParams> {
    if ref_data.len() < 2 {
        ui::skipped("reference FFT: fewer than two data points");
        bail!("reference FFT requires at least two data points");
    }
    if !dt.is_finite() || dt <= 0.0 {
        bail!("reference FFT dt must be positive and finite (got {dt})");
    }

    let results = ReferenceFFT {}
        .fft(dt, ref_data)
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
        ui::skipped("reference fit: time and reference data are empty");
        bail!("Cannot fit empty data.");
    }

    let results = ReferenceFitter {}
        .fit(t, ref_data, params)
        .context("failed to fit reference signal")?;

    Ok(results)
}

fn get_range_indices(t: TimeAxisRef<'_>, start: f64, end: f64) -> Result<(usize, usize)> {
    let idx_start = t.partition_point(|x| x < start);
    let idx_end = t.partition_point(|x| x <= end);

    if idx_start >= idx_end {
        bail!(
            "Invalid range: start ({}) must be less than end ({})",
            idx_start,
            idx_end
        );
    }
    Ok((idx_start, idx_end))
}

fn plot_fit_results(
    cfg: &Config,
    t: &[f64],
    ref_data: &[f64],
    results: &RefFitParams,
) -> Result<()> {
    if t.is_empty() {
        ui::skipped("reference plot: no data");
        return Ok(());
    }

    plot::run_plot(
        &cfg.plot,
        &cfg.paths().reference_fit_plot(),
        "plotting reference fit",
        "reference plot completed",
        |output| {
            let f = results.f_ref;
            let a = results.a_ref;
            let omegat = results.omega_tref;

            if f == 0.0 {
                bail!("Reference frequency is zero, cannot plot results.");
            }

            let t_period = 1.0 / f;
            let t_start_data = t.first().copied().unwrap_or(0.0);
            let t_start_plot = t_start_data;
            let t_end_plot = t_start_data + 3.0 * t_period;

            let (idx_start, idx_end) = get_range_indices(t.into(), t_start_plot, t_end_plot)?;

            let t_plot = &t[idx_start..idx_end];
            let ref_plot = &ref_data[idx_start..idx_end];

            let fit_plot: Vec<f64> = t_plot
                .iter()
                .map(|&ti| a * (2.0 * PI * f * ti - omegat).sin())
                .collect();

            ref_plot::ReferencePlotter {}
                .plot(&cfg.plot, output, t_plot, ref_plot, &fit_plot)
                .context("failed to plot reference signal")
        },
    )?;

    Ok(())
}

fn extract_single_reference_ch(cfg: &Config) -> Result<u8> {
    let ref_ch = cfg.roles.reference_ch;
    if ref_ch == 0 {
        bail!("reference channel is not specified in the configuration");
    }
    Ok(ref_ch)
}
