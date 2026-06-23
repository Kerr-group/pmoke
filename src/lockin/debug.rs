use crate::config::{Config, LockinLpfKind, Window};
use crate::lockin::lockin_core::{FilterDesign, HarmonicLockinResult};
use crate::lockin::lockin_params::LockinParams;
use crate::ui;
use anyhow::{Context, Result, bail};
use num_complex::Complex64;
use std::f64::consts::PI;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

const DEBUG_MARKER: &str = ".pmoke_lockin_debug";
const MIN_PSD_SAMPLES: usize = 8;
const MIN_BACKGROUND_SAMPLES: usize = 8;
const MIN_SIGNAL_P95_SAMPLES: usize = 20;
const PSD_MAX_SAMPLES: usize = 2048;
const PSD_BINS: usize = 512;
const RESPONSE_BINS: usize = 1024;

#[allow(clippy::too_many_arguments)]
pub fn write_harmonic_debug(
    cfg: &Config,
    signal_ch: u8,
    harmonic: usize,
    params: LockinParams,
    filter: Option<&FilterDesign>,
    t_raw: &[f64],
    t_output: &[f64],
    result: &HarmonicLockinResult,
) -> Result<()> {
    let dir = prepare_debug_dir(cfg, signal_ch, harmonic, params, filter)?;

    write_metadata(&dir, cfg, signal_ch, harmonic, params, filter)?;
    write_filter_response(&dir, params, filter)?;
    write_baseband_psd(&dir, cfg, t_raw, result.mixed_signal.as_deref())?;
    write_snr_summary(&dir, cfg, t_output, result)?;

    Ok(())
}

fn prepare_debug_dir(
    cfg: &Config,
    signal_ch: u8,
    harmonic: usize,
    params: LockinParams,
    filter: Option<&FilterDesign>,
) -> Result<PathBuf> {
    let label = cfg
        .lockin
        .lpf_debug_label
        .clone()
        .unwrap_or_else(|| auto_label(cfg, params, filter));
    let dir = PathBuf::from("lockin_debug").join(label).join(format!(
        "{}_ch{}_h{}",
        lpf_kind_name(cfg.lockin.lpf_kind),
        signal_ch,
        harmonic
    ));

    if dir.exists() {
        if !cfg.lockin.lpf_debug_overwrite {
            bail!(
                "lock-in debug target directory already exists: {}",
                dir.display()
            );
        }
        let marker = dir.join(DEBUG_MARKER);
        if !marker.exists() {
            bail!(
                "refusing to overwrite debug directory without marker {}: {}",
                DEBUG_MARKER,
                dir.display()
            );
        }
        fs::remove_dir_all(&dir)
            .with_context(|| format!("failed to clear debug directory {}", dir.display()))?;
    }

    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create debug directory {}", dir.display()))?;
    fs::write(dir.join(DEBUG_MARKER), "pmoke lock-in debug output\n")?;

    Ok(dir)
}

fn auto_label(cfg: &Config, params: LockinParams, filter: Option<&FilterDesign>) -> String {
    let cutoff = filter
        .map(|f| format!("{:.6e}", f.cutoff_hz))
        .unwrap_or_else(|| "none".to_string());
    format!(
        "{}_{}_cutoff_{}_half_{:.6}",
        lpf_kind_name(cfg.lockin.lpf_kind),
        params.cutoff_source.as_str(),
        sanitize_label_component(&cutoff),
        cfg.lockin.lpf_half_window_cycles
    )
}

fn sanitize_label_component(s: &str) -> String {
    s.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn lpf_kind_name(kind: LockinLpfKind) -> &'static str {
    match kind {
        LockinLpfKind::FirZeroPhase => "fir_zero_phase",
        LockinLpfKind::BoxcarLegacy => "boxcar_legacy",
        LockinLpfKind::FirBoxcarEnbw => "fir_boxcar_enbw",
        LockinLpfKind::SyncIirZeroPhase => "sync_iir_zero_phase",
    }
}

fn write_metadata(
    dir: &Path,
    cfg: &Config,
    signal_ch: u8,
    harmonic: usize,
    params: LockinParams,
    filter: Option<&FilterDesign>,
) -> Result<()> {
    let mut rows = vec![
        ("signal_ch".to_string(), signal_ch.to_string()),
        ("harmonic".to_string(), harmonic.to_string()),
        (
            "lpf_kind".to_string(),
            lpf_kind_name(cfg.lockin.lpf_kind).to_string(),
        ),
        ("f_ref".to_string(), params.f_ref.to_string()),
        ("dt".to_string(), params.dt.to_string()),
        ("sample_rate".to_string(), params.sample_rate.to_string()),
        ("stride_samples".to_string(), params.stride.to_string()),
        ("output_rate".to_string(), params.output_rate.to_string()),
        (
            "lpf_half_window_cycles".to_string(),
            cfg.lockin.lpf_half_window_cycles.to_string(),
        ),
        ("t_half".to_string(), params.t_half.to_string()),
        ("n_half".to_string(), params.n_half.to_string()),
        ("tap_count".to_string(), (2 * params.n_half + 1).to_string()),
        (
            "cutoff_source".to_string(),
            params.cutoff_source.as_str().to_string(),
        ),
        (
            "fallback_used".to_string(),
            params.fallback_used.to_string(),
        ),
        (
            "stopband_atten_db".to_string(),
            cfg.lockin.lpf_stopband_atten_db.to_string(),
        ),
        ("attenuation_is_guaranteed".to_string(), "false".to_string()),
    ];

    if let Some(filter) = filter {
        rows.push(("cutoff_hz".to_string(), filter.cutoff_hz.to_string()));
        rows.push((
            "design_cutoff_hz".to_string(),
            filter.design_cutoff_hz.to_string(),
        ));
        rows.push((
            "filter_cutoff_source".to_string(),
            filter.cutoff_source.to_string(),
        ));
        rows.push(("kaiser_beta".to_string(), filter.kaiser_beta.to_string()));
        rows.push((
            "sync_average_samples".to_string(),
            opt_usize(filter.sync_average_samples),
        ));
        rows.push(("iir_order".to_string(), opt_usize(filter.iir_order)));
        rows.push((
            "settling_samples".to_string(),
            filter.settling_samples.to_string(),
        ));
        rows.push((
            "estimated_enbw_hz".to_string(),
            filter.estimated_enbw_hz.to_string(),
        ));
        rows.push((
            "legacy_boxcar_enbw_hz".to_string(),
            opt_f64(filter.legacy_boxcar_enbw_hz),
        ));
        rows.push((
            "enbw_match_error_hz".to_string(),
            opt_f64(filter.enbw_match_error_hz),
        ));
        rows.push((
            "enbw_match_reachable".to_string(),
            opt_bool(filter.enbw_match_reachable),
        ));
        rows.push((
            "user_cutoff_unused".to_string(),
            filter.user_cutoff_unused.to_string(),
        ));
        rows.push((
            "cutoff_hz_over_f_ref".to_string(),
            (filter.cutoff_hz / params.f_ref).to_string(),
        ));
        rows.push((
            "cutoff_hz_over_output_rate".to_string(),
            (filter.cutoff_hz / params.output_rate).to_string(),
        ));
    } else {
        rows.push(("cutoff_hz".to_string(), "NaN".to_string()));
        rows.push(("kaiser_beta".to_string(), "NaN".to_string()));
        rows.push(("estimated_enbw_hz".to_string(), "NaN".to_string()));
    }

    write_key_value_csv(&dir.join("metadata.csv"), &rows)
}

fn opt_f64(value: Option<f64>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "NaN".to_string())
}

fn opt_bool(value: Option<bool>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "NaN".to_string())
}

fn opt_usize(value: Option<usize>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "NaN".to_string())
}

fn write_filter_response(
    dir: &Path,
    params: LockinParams,
    filter: Option<&FilterDesign>,
) -> Result<()> {
    let path = dir.join("filter_response.csv");
    let file =
        fs::File::create(&path).with_context(|| format!("failed to create {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    writeln!(writer, "frequency_hz,response_abs,response_db")?;

    if let Some(filter) = filter {
        let max_freq = 0.5 * params.output_rate;
        for idx in 0..=RESPONSE_BINS {
            let freq = max_freq * idx as f64 / RESPONSE_BINS as f64;
            let response = filter.response_abs(params.sample_rate, freq);
            let response_db = if response > 0.0 {
                20.0 * response.log10()
            } else {
                f64::NEG_INFINITY
            };
            writeln!(writer, "{freq},{response},{response_db}")?;
        }
    }

    Ok(())
}

fn write_baseband_psd(
    dir: &Path,
    cfg: &Config,
    t_raw: &[f64],
    mixed_signal: Option<&[Complex64]>,
) -> Result<()> {
    let path = dir.join("baseband_psd.csv");
    let file =
        fs::File::create(&path).with_context(|| format!("failed to create {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    writeln!(writer, "frequency_hz,psd_re,psd_im,psd_abs")?;

    let Some(mixed_signal) = mixed_signal else {
        return Ok(());
    };

    let window = cfg
        .lockin
        .snr_background_window
        .unwrap_or(cfg.pulse.bg_window_before);
    let samples = complex_window(t_raw, mixed_signal, window);
    if samples
        .iter()
        .any(|value| !value.re.is_finite() || !value.im.is_finite())
    {
        ui::warn("baseband PSD skipped: non-finite samples found in background window");
        return Ok(());
    }
    if samples.len() < MIN_PSD_SAMPLES {
        ui::warn(format!(
            "baseband PSD skipped: only {} samples in background window",
            samples.len()
        ));
        return Ok(());
    }

    let (samples, downsample_step) = downsample_to_limit(&samples, PSD_MAX_SAMPLES);
    let n = samples.len();
    let dt = t_raw
        .windows(2)
        .next()
        .map(|w| w[1] - w[0])
        .ok_or_else(|| anyhow::anyhow!("time axis must contain at least two samples"))?;
    let dt_eff = dt * downsample_step as f64;
    let sample_rate = 1.0 / dt_eff;
    let max_bin = PSD_BINS.min(n / 2);

    for bin in 0..=max_bin {
        let freq = bin as f64 * sample_rate / n as f64;
        let mut re_acc = Complex64::new(0.0, 0.0);
        let mut im_acc = Complex64::new(0.0, 0.0);
        let mut abs_acc = Complex64::new(0.0, 0.0);

        for (idx, sample) in samples.iter().enumerate() {
            let phase = -2.0 * PI * bin as f64 * idx as f64 / n as f64;
            let osc = Complex64::from_polar(1.0, phase);
            re_acc += sample.re * osc;
            im_acc += sample.im * osc;
            abs_acc += sample.norm() * osc;
        }

        let scale = 1.0 / n as f64;
        let psd_re = re_acc.norm_sqr() * scale;
        let psd_im = im_acc.norm_sqr() * scale;
        let psd_abs = abs_acc.norm_sqr() * scale;
        writeln!(writer, "{freq},{psd_re},{psd_im},{psd_abs}")?;
    }

    Ok(())
}

fn complex_window(t: &[f64], values: &[Complex64], window: Window) -> Vec<Complex64> {
    t.iter()
        .zip(values.iter())
        .filter_map(|(&ti, &value)| (ti >= window.start && ti <= window.end).then_some(value))
        .collect()
}

fn downsample_to_limit<T: Copy>(values: &[T], limit: usize) -> (Vec<T>, usize) {
    if values.len() <= limit {
        (values.to_vec(), 1)
    } else {
        let step = ((values.len() as f64) / (limit as f64)).ceil() as usize;
        (values.iter().step_by(step).copied().collect(), step)
    }
}

fn write_snr_summary(
    dir: &Path,
    cfg: &Config,
    t_output: &[f64],
    result: &HarmonicLockinResult,
) -> Result<()> {
    let bg_window = cfg
        .lockin
        .snr_background_window
        .unwrap_or(cfg.pulse.bg_window_before);
    let signal_window = cfg
        .lockin
        .snr_signal_window
        .unwrap_or(cfg.reference.fft_window);

    let bg = finite_lockin_window(t_output, &result.li_x, &result.li_y, bg_window);
    let sig = finite_lockin_window(t_output, &result.li_x, &result.li_y, signal_window);

    if bg.len() < MIN_BACKGROUND_SAMPLES {
        ui::warn(format!(
            "S/N background window has only {} finite samples; writing NaN metrics",
            bg.len()
        ));
    }
    if sig.len() < MIN_SIGNAL_P95_SAMPLES {
        ui::warn(format!(
            "S/N signal window has only {} finite samples; signal_p95 metrics will be NaN",
            sig.len()
        ));
    }

    let bg_amp: Vec<f64> = bg.iter().map(|sample| sample.amp).collect();
    let sig_amp: Vec<f64> = sig.iter().map(|sample| sample.amp).collect();
    let bg_lix: Vec<f64> = bg.iter().map(|sample| sample.lix).collect();
    let bg_liy: Vec<f64> = bg.iter().map(|sample| sample.liy).collect();

    let background_amp_mean = metric_if(bg.len() >= MIN_BACKGROUND_SAMPLES, || mean(&bg_amp));
    let background_amp_std = metric_if(bg.len() >= MIN_BACKGROUND_SAMPLES, || {
        population_std(&bg_amp)
    });
    let background_lix_std = metric_if(bg.len() >= MIN_BACKGROUND_SAMPLES, || {
        population_std(&bg_lix)
    });
    let background_liy_std = metric_if(bg.len() >= MIN_BACKGROUND_SAMPLES, || {
        population_std(&bg_liy)
    });
    let signal_peak_amp = metric_if(!sig_amp.is_empty(), || max_finite(&sig_amp));
    let signal_p95_amp = metric_if(sig.len() >= MIN_SIGNAL_P95_SAMPLES, || {
        nearest_rank_percentile(sig_amp.clone(), 95.0)
    });
    let signal_peak_snr = snr(signal_peak_amp, background_amp_std);
    let signal_p95_snr = snr(signal_p95_amp, background_amp_std);

    let rows = vec![
        (
            "background_amp_mean".to_string(),
            background_amp_mean.to_string(),
        ),
        (
            "background_amp_std".to_string(),
            background_amp_std.to_string(),
        ),
        (
            "background_lix_std".to_string(),
            background_lix_std.to_string(),
        ),
        (
            "background_liy_std".to_string(),
            background_liy_std.to_string(),
        ),
        ("signal_peak_amp".to_string(), signal_peak_amp.to_string()),
        ("signal_p95_amp".to_string(), signal_p95_amp.to_string()),
        ("signal_peak_snr".to_string(), signal_peak_snr.to_string()),
        ("signal_p95_snr".to_string(), signal_p95_snr.to_string()),
        ("background_samples".to_string(), bg.len().to_string()),
        ("signal_samples".to_string(), sig.len().to_string()),
    ];

    write_key_value_csv(&dir.join("snr_summary.csv"), &rows)
}

struct LockinSample {
    lix: f64,
    liy: f64,
    amp: f64,
}

fn finite_lockin_window(
    t: &[f64],
    li_x: &[f64],
    li_y: &[f64],
    window: Window,
) -> Vec<LockinSample> {
    t.iter()
        .zip(li_x.iter())
        .zip(li_y.iter())
        .filter_map(|((&ti, &lix), &liy)| {
            if ti >= window.start && ti <= window.end && lix.is_finite() && liy.is_finite() {
                let amp = lix.hypot(liy);
                amp.is_finite().then_some(LockinSample { lix, liy, amp })
            } else {
                None
            }
        })
        .collect()
}

fn metric_if(condition: bool, f: impl FnOnce() -> f64) -> f64 {
    if condition { f() } else { f64::NAN }
}

fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        f64::NAN
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn population_std(values: &[f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mean = mean(values);
    let variance = values
        .iter()
        .map(|value| {
            let diff = value - mean;
            diff * diff
        })
        .sum::<f64>()
        / values.len() as f64;
    variance.sqrt()
}

fn max_finite(values: &[f64]) -> f64 {
    values.iter().copied().fold(f64::NEG_INFINITY, f64::max)
}

fn nearest_rank_percentile(mut values: Vec<f64>, percentile: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let rank = ((percentile / 100.0) * values.len() as f64).ceil() as usize;
    values[rank.saturating_sub(1).min(values.len() - 1)]
}

fn snr(signal: f64, noise: f64) -> f64 {
    if !signal.is_finite() || !noise.is_finite() || noise <= 0.0 {
        f64::NAN
    } else {
        signal / noise
    }
}

fn write_key_value_csv(path: &Path, rows: &[(String, String)]) -> Result<()> {
    let file =
        fs::File::create(path).with_context(|| format!("failed to create {}", path.display()))?;
    let mut writer = BufWriter::new(file);
    writeln!(writer, "key,value")?;
    for (key, value) in rows {
        writeln!(writer, "{key},{value}")?;
    }
    Ok(())
}
