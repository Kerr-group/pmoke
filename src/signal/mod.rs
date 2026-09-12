pub mod save;
pub mod signal_plot;

use crate::config::Config;
use crate::signal::save::{get_signal_headers, write_signal_results};
use crate::signal::signal_plot::SignalPlotter;
use crate::ui;
use crate::utils::channels::build_channel_list;
use crate::utils::time_axis::TimeAxisRef;
use anyhow::{Context, Result, bail};
use std::time::Instant;

pub struct SignalMeanOutput {
    pub channel: u8,
    pub mean: Vec<f64>,
}

/// Average raw readout channels over the lock-in support window.
///
/// Each output is the trapezoidal boxcar mean over `±half_window_s` around the
/// same stride centers the lock-in uses, so the returned means align
/// sample-for-sample with `t_stride`. `raw_data` columns follow
/// [`build_channel_list`] order, matching [`crate::lockin::run_li`].
#[allow(clippy::too_many_arguments)]
pub fn run_signal_analysis<'a>(
    cfg: &Config,
    t_stride: &[f64],
    sensor_rate_ch: &[Vec<f64>],
    sensor_integral_ch: &[Vec<f64>],
    raw_t: impl Into<TimeAxisRef<'a>>,
    raw_data: &[Vec<f64>],
    f_ref: f64,
) -> Result<Vec<SignalMeanOutput>> {
    if cfg.signals.is_empty() {
        ui::skipped("Signal analysis: no [[signals]] entries specified");
        return Ok(Vec::new());
    }
    let raw_t = raw_t.into();
    let dt = raw_t
        .dt()
        .ok_or_else(|| anyhow::anyhow!("signal time axis must contain at least two samples"))?;
    let t0 = raw_t
        .first()
        .ok_or_else(|| anyhow::anyhow!("signal time axis is empty"))?;
    let half_window_s = cfg.lockin.lpf_half_window_cycles / f_ref;
    let stride = cfg.lockin.stride_samples;

    let channels = build_channel_list(cfg)?;
    let mut outputs = Vec::with_capacity(cfg.signals.len());
    let t0_instant = Instant::now();
    let pb = ui::progress("running signal averaging", cfg.signals.len() as u64);
    for signal in &cfg.signals {
        pb.set_message(format!("signal averaging ch{}", signal.channel));
        let column = channels
            .iter()
            .position(|c| *c == signal.channel)
            .with_context(|| {
                format!(
                    "signal channel {} not found in fetched channels {:?}",
                    signal.channel, channels
                )
            })?;
        let data = raw_data.get(column).with_context(|| {
            format!(
                "fetched data has no column for signal channel {}",
                signal.channel
            )
        })?;
        if data.len() != raw_t.len() {
            bail!(
                "signal ch{} length ({}) differs from time axis length ({})",
                signal.channel,
                data.len(),
                raw_t.len()
            );
        }
        let output = pmoke_analysis_core::boxcar_mean(
            data,
            pmoke_analysis_core::BoxcarMeanSettings {
                start_time_s: t0,
                sample_interval_s: dt,
                half_window_s,
                stride_samples: stride,
            },
        )
        .with_context(|| format!("signal averaging failed for ch{}", signal.channel))?;
        if output.mean.len() != t_stride.len() {
            bail!(
                "signal ch{} output length ({}) differs from lock-in grid length ({})",
                signal.channel,
                output.mean.len(),
                t_stride.len()
            );
        }
        outputs.push(SignalMeanOutput {
            channel: signal.channel,
            mean: output.mean,
        });
        pb.inc(1);
    }

    let headers = get_signal_headers(cfg)?;
    let means = outputs
        .iter()
        .map(|output| output.mean.clone())
        .collect::<Vec<_>>();
    write_signal_results(
        cfg.paths().signal_csv(),
        &headers,
        t_stride,
        sensor_rate_ch,
        sensor_integral_ch,
        &means,
        cfg.lockin.save_npy,
    )?;

    ui::finish_saved(
        pb,
        format!(
            "signal means for channels {:?} ({})",
            outputs
                .iter()
                .map(|output| output.channel)
                .collect::<Vec<_>>(),
            ui::fmt_duration(t0_instant.elapsed())
        ),
    );

    let labels = cfg
        .signals
        .iter()
        .map(|signal| signal.label.clone())
        .collect::<Vec<_>>();
    let units = cfg
        .signals
        .iter()
        .map(|signal| signal.unit.clone())
        .collect::<Vec<_>>();
    crate::plot::run_plot(
        &cfg.plot,
        &cfg.paths().signal_combined_plot(),
        "plotting signal means",
        "signal plot completed",
        |output| {
            SignalPlotter {}
                .plot(&cfg.plot, output, t_stride, means.clone(), &labels, &units)
                .context("failed to plot signal means")
        },
    )?;
    for (entry, mean) in cfg.signals.iter().zip(means.iter()) {
        crate::plot::run_plot(
            &cfg.plot,
            &cfg.paths().signal_channel_plot(entry.channel),
            format!("plotting signal ch{}", entry.channel),
            format!("signal ch{} plot completed", entry.channel),
            |output| {
                SignalPlotter {}
                    .plot(
                        &cfg.plot,
                        output,
                        t_stride,
                        vec![mean.clone()],
                        &[entry.label.clone()],
                        &[entry.unit.clone()],
                    )
                    .context("failed to plot signal means")
            },
        )?;
    }
    Ok(outputs)
}

#[cfg(test)]
mod tests {
    use super::run_signal_analysis;
    use crate::config::Signal;
    use crate::test_support::test_config;
    use crate::utils::time_axis::WaveformTime;

    #[test]
    fn constant_signal_averages_to_itself_and_writes_csv() {
        let directory = std::env::temp_dir().join(format!(
            "pmoke-signal-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let mut cfg = test_config(vec![1], vec![]);
        cfg.version = 6;
        cfg.roles.reference_ch = 2;
        cfg.set_artifact_root(directory.clone());
        cfg.signals = vec![Signal {
            channel: 3,
            label: "DC".to_string(),
            unit: "V".to_string(),
        }];

        let dt = 1.0e-5;
        let raw_t = WaveformTime::Explicit((0..20_000).map(|i| i as f64 * dt).collect());
        let raw_data = vec![vec![0.5; 20_000], vec![0.25; 20_000], vec![2.5; 20_000]];
        let reference = pmoke_analysis_core::boxcar_mean(
            &raw_data[0],
            pmoke_analysis_core::BoxcarMeanSettings {
                start_time_s: 0.0,
                sample_interval_s: dt,
                half_window_s: 1.0e-3,
                stride_samples: 1,
            },
        )
        .unwrap();
        let stride_len = reference.time_s.len();
        let zeros = vec![vec![0.0; stride_len]];

        let outputs = run_signal_analysis(
            &cfg,
            &reference.time_s,
            &zeros,
            &zeros,
            raw_t.as_ref(),
            &raw_data,
            1000.0,
        )
        .unwrap();
        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].channel, 3);
        assert!(outputs[0].mean.iter().all(|v| (v - 2.5).abs() < 1.0e-9));

        let csv = std::fs::read_to_string(cfg.paths().signal_csv()).unwrap();
        assert!(csv.contains("Ch3 DC mean (V)"));
        std::fs::remove_dir_all(&directory).unwrap();
    }
}
