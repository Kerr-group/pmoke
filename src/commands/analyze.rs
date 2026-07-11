use crate::{
    config::Config,
    kerr::run_kerr_analysis,
    lockin::run_li,
    phase::run_phase_analysis,
    ui,
    utils::waveform::{WaveformData, read_all_fetched_waveforms},
};
use anyhow::{Result, bail};

pub fn analyze(cfg: &Config) -> Result<()> {
    let pb = ui::spinner("reading fetched waveform data");
    let t0 = std::time::Instant::now();
    let data = read_all_fetched_waveforms(cfg)?;
    let elapsed_read = t0.elapsed();

    ui::finish_read(
        pb,
        format!(
            "fetched data: {} channels, {} samples ({})",
            data.channels.len(),
            data.channels.first().map_or(0, Vec::len),
            ui::fmt_duration(elapsed_read)
        ),
    );

    if data.channels.is_empty() {
        bail!("Fetched data is empty, cannot extract channels.");
    }

    run_analyze(cfg, &data)?;
    Ok(())
}

pub fn run_analyze(cfg: &Config, data: &WaveformData) -> Result<()> {
    validate_waveform_data(data)?;
    let (t_stride, sensor_rate_stride, sensor_integral_stride, li_results) =
        run_li(cfg, &data.t, &data.channels)?;

    // run phase analysis here
    let ch = cfg.phase_signal_ch();

    if ch.is_empty() {
        ui::skipped("phase analysis: no channels specified");
        return Ok(());
    }
    let li_rotated_results = run_phase_analysis(
        cfg,
        &t_stride,
        &sensor_rate_stride,
        &sensor_integral_stride,
        &li_results,
    )?;
    drop(li_results);

    // run Kerr analysis here
    run_kerr_analysis(
        cfg,
        &t_stride,
        &sensor_rate_stride,
        &sensor_integral_stride,
        &li_rotated_results,
    )?;

    Ok(())
}

fn validate_waveform_data(data: &WaveformData) -> Result<()> {
    let sample_count = data.t.len();
    if sample_count < 2 {
        bail!("analysis requires at least two waveform samples (got {sample_count})");
    }
    if data.channels.is_empty() {
        bail!("analysis requires at least one waveform channel");
    }
    for (channel_index, channel) in data.channels.iter().enumerate() {
        if channel.len() != sample_count {
            bail!(
                "waveform channel {} has {} samples, expected {sample_count}",
                channel_index + 1,
                channel.len()
            );
        }
        if let Some((sample_index, value)) = channel
            .iter()
            .copied()
            .enumerate()
            .find(|(_, value)| !value.is_finite())
        {
            bail!(
                "waveform channel {} contains a non-finite value at sample {sample_index}: {value}",
                channel_index + 1
            );
        }
    }

    let time = data.t.as_ref();
    let dt = time.value_at(1) - time.value_at(0);
    if !dt.is_finite() || dt <= 0.0 {
        bail!("waveform time step must be positive and finite (got {dt})");
    }
    for index in 0..sample_count {
        let value = time.value_at(index);
        if !value.is_finite() {
            bail!("waveform time is non-finite at sample {index}: {value}");
        }
        if index > 1 {
            let step = value - time.value_at(index - 1);
            let roundoff = value.abs().max(time.value_at(index - 1).abs()) * f64::EPSILON * 16.0;
            let tolerance = (dt.abs() * 1.0e-6).max(roundoff);
            if !step.is_finite() || (step - dt).abs() > tolerance {
                bail!(
                    "waveform time step changes at sample {index}: {step}, expected {dt} ± {tolerance}"
                );
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{run_analyze, validate_waveform_data};
    use crate::config::{KerrType, LockinLpfKind, Window};
    use crate::utils::csv::read_csv;
    use crate::utils::waveform::WaveformData;
    use std::f64::consts::PI;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TemporaryDirectory(PathBuf);

    impl TemporaryDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "pmoke-synthetic-analysis-{}-{nonce}",
                std::process::id()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn waveform_preflight_rejects_non_finite_and_missing_samples() {
        let non_finite = WaveformData {
            t: vec![0.0, 1.0, 2.0].into(),
            channels: vec![vec![0.0, f64::NAN, 1.0]],
        };
        assert!(
            validate_waveform_data(&non_finite)
                .unwrap_err()
                .to_string()
                .contains("non-finite")
        );

        let short_channel = WaveformData {
            t: vec![0.0, 1.0, 2.0].into(),
            channels: vec![vec![0.0, 1.0]],
        };
        assert!(
            validate_waveform_data(&short_channel)
                .unwrap_err()
                .to_string()
                .contains("expected 3")
        );
    }

    #[test]
    fn waveform_preflight_rejects_a_gap_in_explicit_time() {
        let data = WaveformData {
            t: vec![0.0, 1.0, 3.0].into(),
            channels: vec![vec![0.0, 1.0, 2.0]],
        };
        assert!(
            validate_waveform_data(&data)
                .unwrap_err()
                .to_string()
                .contains("time step changes")
        );
    }

    #[test]
    fn synthetic_harmonics_pipeline_recovers_folded_kerr_angle() {
        let directory = TemporaryDirectory::new();
        let mut cfg = crate::test_support::test_config(vec![1], vec![3]);
        cfg.version = 4;
        cfg.source_path = directory.0.join("config.toml");
        cfg.roles.reference_ch = 2;
        cfg.reference.fft_window = Window {
            start: 0.0,
            end: 0.097_3,
        };
        cfg.reference.stride_samples = 1_000;
        cfg.reference.window_samples = 100;
        cfg.pulse.bg_window_before = Window {
            start: 0.0,
            end: 0.01,
        };
        cfg.pulse.bg_window_after = Window {
            start: 0.18,
            end: 0.199,
        };
        cfg.lockin.lpf_kind = LockinLpfKind::BoxcarLegacy;
        cfg.lockin.stride_samples = 20;
        cfg.lockin.lpf_half_window_cycles = 1.0;
        cfg.lockin.lpf_cutoff_hz = None;
        cfg.phase.m_omega_t0_offset = vec![0.0; 6];
        cfg.kerr.kerr_type = KerrType::Harmonics;

        let sample_count = 20_000;
        let dt = 1.0e-5;
        let frequency = 1_000.0;
        let theta = 0.01_f64;
        let bessel = [
            0.581_864_936_842_083_3,
            0.315_745_306_087_972_3,
            0.104_537_902_479_595_42,
            0.025_139_158_519_404_087,
            0.004_762_786_735_204_94,
            0.000_745_551_998_014_054_3,
        ];
        let time = (0..sample_count)
            .map(|index| index as f64 * dt)
            .collect::<Vec<_>>();
        let sensor = time
            .iter()
            .map(|value| {
                if (0.03..0.15).contains(value) {
                    1.0
                } else {
                    0.0
                }
            })
            .collect::<Vec<_>>();
        let reference = time
            .iter()
            .map(|value| {
                let amplitude_drift = 1.0 + 0.01 * (2.0 * PI * 3.0 * value).sin();
                0.02 + 0.01 * value + amplitude_drift * (2.0 * PI * frequency * value).sin()
            })
            .collect::<Vec<_>>();
        let signal = time
            .iter()
            .map(|value| {
                let harmonics = bessel
                    .iter()
                    .enumerate()
                    .map(|(index, coefficient)| {
                        let harmonic = index + 1;
                        let amplitude = if harmonic % 2 == 0 {
                            (2.0 * theta).cos() * coefficient
                        } else {
                            (2.0 * theta).sin() * coefficient
                        };
                        let phase = if harmonic % 2 == 0 { PI / 2.0 } else { PI };
                        2.0 * amplitude
                            * (harmonic as f64 * 2.0 * PI * frequency * value + phase).sin()
                    })
                    .sum::<f64>();
                let deterministic_noise = 1.0e-5 * (2.0 * PI * 12_345.0 * value + 0.4).sin();
                0.01 + 0.002 * value + harmonics + deterministic_noise
            })
            .collect::<Vec<_>>();
        let data = WaveformData {
            t: time.into(),
            channels: vec![sensor, reference, signal],
        };

        run_analyze(&cfg, &data).unwrap();

        let columns = read_csv(directory.0.join("kerr_results.csv")).unwrap();
        let kerr = columns.last().unwrap();
        assert!(!kerr.is_empty());
        let expected = 0.5 * (2.0 * theta).tan().atan();
        let maximum_error = kerr
            .iter()
            .map(|value| (value - expected).abs())
            .fold(0.0_f64, f64::max);
        assert!(
            maximum_error < 2.0e-4,
            "expected {expected}, maximum error was {maximum_error}"
        );
        assert!(directory.0.join("analysis_metadata.toml").is_file());
    }
}
