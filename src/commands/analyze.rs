use crate::{
    config::Config,
    moke::run_moke_analysis,
    phase::run_phase_analysis,
    ui,
    utils::waveform::{WaveformData, read_all_fetched_waveforms},
};
use anyhow::{Context, Result, bail};

pub fn analyze(cfg: &Config) -> Result<()> {
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock =
        crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "analyze")?;
    crate::config::validate_for_target(cfg, crate::config::ValidationTarget::Analyze)?;
    crate::commands::run_dir::prepare_analysis_run(cfg)?;
    crate::commands::run_dir::write_run_state(cfg, "analyzing", "analysis", None)?;
    let result = (|| {
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
        run_analyze_inner(cfg, &data)
    })();
    record_analysis_result(cfg, &result)?;
    result
}

pub fn run_analyze(cfg: &Config, data: &WaveformData) -> Result<()> {
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock =
        crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "analysis")?;
    run_analyze_locked(cfg, data)
}

pub fn run_analyze_locked(cfg: &Config, data: &WaveformData) -> Result<()> {
    crate::commands::run_dir::write_run_state(cfg, "analyzing", "analysis", None)?;
    let result = run_analyze_inner(cfg, data);
    record_analysis_result(cfg, &result)?;
    result
}

fn record_analysis_result(cfg: &Config, result: &Result<()>) -> Result<()> {
    match &result {
        Ok(()) => crate::commands::run_dir::write_run_state(cfg, "complete", "analysis", None)?,
        Err(error) => {
            crate::commands::run_dir::write_run_state(cfg, "failed", "analysis", Some(error))?
        }
    }
    Ok(())
}

/// One executed step of the native analyze pipeline. The observer exists so
/// tests and evidence capture can prove the *real* execution order (sensor,
/// reference preparation, signal, lock-in, phase, MOKE) instead of a
/// reordered log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnalyzeStep {
    Sensor,
    ReferencePreparation,
    Signal,
    Lockin,
    Phase,
    Moke,
}

fn run_analyze_inner(cfg: &Config, data: &WaveformData) -> Result<()> {
    run_analyze_inner_observed(cfg, data, &mut |_| {})
}

pub(crate) fn run_analyze_inner_observed(
    cfg: &Config,
    data: &WaveformData,
    observer: &mut dyn FnMut(AnalyzeStep),
) -> Result<()> {
    let mut cfg_staging = cfg.clone();
    cfg_staging.staging_active = true;

    let staging_analysis = cfg_staging.paths().analysis_dir();
    if staging_analysis.exists() {
        std::fs::remove_dir_all(&staging_analysis)
            .context("failed to clean up previous incomplete staging directory")?;
    }
    std::fs::create_dir(&staging_analysis)
        .context("failed to create analysis staging directory")?;
    crate::commands::run_dir::write_analysis_config_snapshots(&cfg_staging)?;

    validate_waveform_data(data)?;

    // Execution order: sensor -> reference/common preparation -> signal ->
    // lock-in -> phase -> MOKE. The reference fit and the frozen analysis
    // window/output grid are prepared once after the sensor stage and reused
    // by the signal readout and the lock-in demodulation; the full-rate
    // sensor series is strided onto that prepared grid without repeating the
    // sensor computation.
    let sensor = crate::lockin::run_sensor_stage(&cfg_staging, &data.t, &data.channels)?;
    observer(AnalyzeStep::Sensor);
    let preparation =
        crate::lockin::prepare_lockin_grid(&cfg_staging, &data.t, &data.channels, &sensor)?;
    observer(AnalyzeStep::ReferencePreparation);

    if !cfg_staging.signals.is_empty() {
        crate::signal::run_signal_analysis(
            &cfg_staging,
            &preparation.t_stride,
            &preparation.sensor_rate_stride,
            &preparation.sensor_integral_stride,
            &data.t,
            &data.channels,
            preparation.reference.f_ref,
        )?;
        observer(AnalyzeStep::Signal);
    } else {
        ui::skipped("signal analysis: no [[signals]] entries specified");
    }

    let lockin_output =
        crate::lockin::execute_lockin(&cfg_staging, &data.t, &data.channels, &preparation)?;
    observer(AnalyzeStep::Lockin);
    let li_results = lockin_output.result;

    // run phase analysis here
    let ch = cfg_staging.phase_signal_ch();

    if !ch.is_empty() {
        let li_rotated_results = run_phase_analysis(
            &cfg_staging,
            &preparation.t_stride,
            &preparation.sensor_rate_stride,
            &preparation.sensor_integral_stride,
            &li_results,
        )?;
        observer(AnalyzeStep::Phase);
        drop(li_results);

        // run Moke analysis here
        run_moke_analysis(
            &cfg_staging,
            &preparation.t_stride,
            &preparation.sensor_rate_stride,
            &preparation.sensor_integral_stride,
            &li_rotated_results,
        )?;
        observer(AnalyzeStep::Moke);
    } else {
        ui::skipped("phase analysis: no channels specified");
    }

    crate::lockin::provenance::write_analysis_metadata(
        &cfg_staging,
        &cfg_staging.paths(),
        &cfg.resolver(),
        &preparation.reference,
        &lockin_output.provenance,
        cfg_staging.roles.reference_ch,
    )?;

    let canonical_analysis = cfg.paths().analysis_dir();
    crate::commands::run_dir::publish_staged_directory(
        &staging_analysis,
        &canonical_analysis,
        true, // Always allow overwrite for analysis results
    )?;

    Ok(())
}

/// Relative tolerance for explicit CSV time-step differences.
const TIME_AXIS_RELATIVE_TOLERANCE: f64 = 1.0e-6;
/// Additional scale-aware allowance for timestamp serialization roundoff.
const TIME_AXIS_ROUNDOFF_FACTOR: f64 = 16.0;

pub(crate) fn validate_waveform_data(data: &WaveformData) -> Result<()> {
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
            let roundoff = value.abs().max(time.value_at(index - 1).abs())
                * f64::EPSILON
                * TIME_AXIS_ROUNDOFF_FACTOR;
            let tolerance = (dt.abs() * TIME_AXIS_RELATIVE_TOLERANCE).max(roundoff);
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
    use super::{analyze, run_analyze, validate_waveform_data};
    use crate::config::{LockinLpfKind, MokeType, Window};
    use crate::utils::csv::read_csv;
    use crate::utils::waveform::WaveformData;
    use std::f64::consts::PI;
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static TEMP_DIRECTORY_COUNTER: AtomicUsize = AtomicUsize::new(0);

    struct TemporaryDirectory(PathBuf);

    impl TemporaryDirectory {
        fn new() -> Self {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let counter = TEMP_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "pmoke-synthetic-analysis-{}-{nonce}-{counter}",
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

    /// Fixture origin (s): nonzero to pin that the prepared downstream grid
    /// keeps the recorded time base instead of re-zeroing it.
    const FIXTURE_ORIGIN_S: f64 = 0.001_37;
    const FIXTURE_DT_S: f64 = 1.0e-5;
    const FIXTURE_SAMPLES: usize = 20_000;
    const FIXTURE_FREQUENCY_HZ: f64 = 1_000.0;
    const FIXTURE_THETA: f64 = 0.01;
    const FIXTURE_STRIDE_SAMPLES: usize = 20;
    /// Fractional lock-in support: `half_window_cycles / f_ref / dt` is not an
    /// integer for the fitted reference, so the trapezoidal support exercises
    /// its endpoint-interpolation path.
    const FIXTURE_HALF_WINDOW_CYCLES: f64 = 1.3;

    fn fixture_time() -> Vec<f64> {
        (0..FIXTURE_SAMPLES)
            .map(|index| FIXTURE_ORIGIN_S + index as f64 * FIXTURE_DT_S)
            .collect()
    }

    /// Synthetic recorded waveform: ch1 sensor, ch2 reference, ch3 lock-in
    /// signal, ch4.. raw `[[signals]]` readout channels.
    fn synthetic_recorded_waveform(max_channel: u8) -> WaveformData {
        let time = fixture_time();
        let relative = |value: f64| value - FIXTURE_ORIGIN_S;
        let sensor = time
            .iter()
            .map(|value| {
                if (0.03..0.15).contains(&relative(*value)) {
                    1.0
                } else {
                    0.0
                }
            })
            .collect::<Vec<_>>();
        let reference = time
            .iter()
            .map(|value| {
                let relative = relative(*value);
                let amplitude_drift = 1.0 + 0.01 * (2.0 * PI * 3.0 * relative).sin();
                0.02 + 0.01 * relative
                    + amplitude_drift * (2.0 * PI * FIXTURE_FREQUENCY_HZ * relative).sin()
            })
            .collect::<Vec<_>>();
        let bessel = [
            0.581_864_936_842_083_3,
            0.315_745_306_087_972_3,
            0.104_537_902_479_595_42,
            0.025_139_158_519_404_087,
            0.004_762_786_735_204_94,
            0.000_745_551_998_014_054_3,
        ];
        let lockin = time
            .iter()
            .map(|value| {
                let relative = relative(*value);
                let harmonics = bessel
                    .iter()
                    .enumerate()
                    .map(|(index, coefficient)| {
                        let harmonic = index + 1;
                        let amplitude = if harmonic % 2 == 0 {
                            (2.0 * FIXTURE_THETA).cos() * coefficient
                        } else {
                            (2.0 * FIXTURE_THETA).sin() * coefficient
                        };
                        let phase = if harmonic % 2 == 0 { PI / 2.0 } else { PI };
                        2.0 * amplitude
                            * (harmonic as f64 * 2.0 * PI * FIXTURE_FREQUENCY_HZ * relative + phase)
                                .sin()
                    })
                    .sum::<f64>();
                let deterministic_noise = 1.0e-5 * (2.0 * PI * 12_345.0 * relative + 0.4).sin();
                0.01 + 0.002 * relative + harmonics + deterministic_noise
            })
            .collect::<Vec<_>>();
        let mut channels = vec![sensor, reference, lockin];
        for channel in 4..=max_channel {
            let index = f64::from(channel);
            channels.push(
                time.iter()
                    .map(|value| {
                        let relative = relative(*value);
                        0.05 * index
                            + 0.01 * index * relative
                            + 0.001 * index * (2.0 * PI * 7.3 * relative).sin()
                    })
                    .collect::<Vec<_>>(),
            );
        }
        WaveformData {
            t: time.into(),
            channels,
        }
    }

    /// Regression fixture config: sensor ch1, reference ch2, lock-in ch3 and
    /// raw `[[signals]]` entries from `signal_channels`.
    fn analyze_regression_config(
        directory: &TemporaryDirectory,
        signal_channels: &[u8],
    ) -> crate::config::Config {
        let mut cfg = crate::test_support::test_config(vec![1], vec![3]);
        cfg.source_path = directory.0.join("config.toml");
        cfg.set_artifact_root(directory.0.clone());
        cfg.roles.reference_ch = 2;
        cfg.signals = signal_channels
            .iter()
            .copied()
            .map(|channel| crate::config::Signal {
                channel,
                label: format!("DC{channel}"),
                unit: "V".to_string(),
            })
            .collect();
        let max_channel = signal_channels.iter().copied().max().unwrap_or(3).max(3);
        cfg.channels = (1..=max_channel)
            .map(|index| crate::config::Channel {
                index,
                factor: Some(index as f64),
                scale_to_abs_max: None,
                label: Some(format!("ch{index}")),
                unit_out: Some("T".to_string()),
            })
            .collect();
        cfg.reference.fft_window = Window {
            start: FIXTURE_ORIGIN_S,
            end: FIXTURE_ORIGIN_S + 0.097_3,
        };
        cfg.reference.stride_samples = 1_000;
        cfg.reference.window_samples = 100;
        cfg.pulse.bg_window_before = Window {
            start: FIXTURE_ORIGIN_S,
            end: FIXTURE_ORIGIN_S + 0.01,
        };
        cfg.pulse.bg_window_after = Window {
            start: FIXTURE_ORIGIN_S + 0.18,
            end: FIXTURE_ORIGIN_S + 0.199,
        };
        cfg.lockin.lpf_kind = LockinLpfKind::BoxcarLegacy;
        cfg.lockin.stride_samples = FIXTURE_STRIDE_SAMPLES;
        cfg.lockin.lpf_half_window_cycles = FIXTURE_HALF_WINDOW_CYCLES;
        cfg.phase.m_omega_t0_offset = vec![0.0; 6];
        cfg.moke.moke_type = MokeType::Harmonics;
        cfg.moke.use_sensor_ch = 1;
        cfg
    }

    /// Relative tolerance for the golden samples recorded before the
    /// stage-order change. The reference fit runs through the embedded Python
    /// stack, whose last-bit results can differ per platform, so the samples
    /// are a before/after regression rather than a byte-identity check;
    /// structural identity and the recipe recomputation below stay tight.
    const GOLDEN_RELATIVE_TOLERANCE: f64 = 1.0e-4;
    const GOLDEN_ROWS: usize = 985;
    const GOLDEN_SENSOR_ROWS: usize = 1_000;

    fn assert_close(label: &str, actual: f64, expected: f64) {
        let tolerance = GOLDEN_RELATIVE_TOLERANCE * expected.abs().max(1.0e-6);
        assert!(
            (actual - expected).abs() <= tolerance,
            "{label}: expected {expected}, got {actual} (tolerance {tolerance})"
        );
    }

    #[test]
    fn analyze_executes_sensor_reference_signal_lockin_phase_moke_once_in_order() {
        let directory = TemporaryDirectory::new();
        let cfg = analyze_regression_config(&directory, &[4]);
        let data = synthetic_recorded_waveform(4);

        // Staging paths are deterministic for the run root, so the observer
        // can check the real filesystem state at each execution boundary: the
        // signal stage must have produced its CSV before lock-in starts.
        let mut staging_cfg = cfg.clone();
        staging_cfg.staging_active = true;
        let staging_paths = staging_cfg.paths();

        let mut steps = Vec::new();
        let mut lockin_csv_existed = Vec::new();
        let mut signal_csv_existed = Vec::new();
        super::run_analyze_inner_observed(&cfg, &data, &mut |step| {
            steps.push(step);
            lockin_csv_existed.push(staging_paths.lockin_xy_csv(3).exists());
            signal_csv_existed.push(staging_paths.signal_csv().exists());
        })
        .unwrap();

        assert_eq!(
            steps,
            vec![
                super::AnalyzeStep::Sensor,
                super::AnalyzeStep::ReferencePreparation,
                super::AnalyzeStep::Signal,
                super::AnalyzeStep::Lockin,
                super::AnalyzeStep::Phase,
                super::AnalyzeStep::Moke,
            ]
        );
        // The shared prerequisites run exactly once each.
        for step in [
            super::AnalyzeStep::Sensor,
            super::AnalyzeStep::ReferencePreparation,
            super::AnalyzeStep::Signal,
            super::AnalyzeStep::Lockin,
            super::AnalyzeStep::Phase,
            super::AnalyzeStep::Moke,
        ] {
            assert_eq!(
                steps.iter().filter(|candidate| **candidate == step).count(),
                1,
                "{step:?} must execute exactly once"
            );
        }
        // No lock-in artifact exists while sensor/reference/signal run, and
        // the signal output already exists by the time lock-in completes:
        // this is execution order, not reordered logging.
        let signal_index = steps
            .iter()
            .position(|step| *step == super::AnalyzeStep::Signal)
            .unwrap();
        let lockin_index = steps
            .iter()
            .position(|step| *step == super::AnalyzeStep::Lockin)
            .unwrap();
        assert!(
            lockin_csv_existed[..=signal_index]
                .iter()
                .all(|exists| !exists)
        );
        assert!(signal_csv_existed[lockin_index]);
    }

    #[test]
    fn analyze_pipeline_outputs_match_the_recorded_regression() {
        let directory = TemporaryDirectory::new();
        let cfg = analyze_regression_config(&directory, &[4]);
        let data = synthetic_recorded_waveform(4);
        run_analyze(&cfg, &data).unwrap();

        let paths = cfg.paths();
        let signal = read_csv(paths.signal_csv()).unwrap();
        let lockin = read_csv(paths.lockin_xy_csv(3)).unwrap();
        let moke = read_csv(paths.moke_csv()).unwrap();
        let sensor = read_csv(paths.sensor_csv()).unwrap();

        // Exact row/column/time/grid identity: the signal readout reuses the
        // lock-in grid and the same sensor columns, and MOKE consumes that
        // grid unchanged.
        assert_eq!(signal[0].len(), GOLDEN_ROWS);
        assert_eq!(lockin[0].len(), GOLDEN_ROWS);
        assert_eq!(moke[0].len(), GOLDEN_ROWS);
        assert_eq!(signal.len(), 4);
        assert_eq!(lockin.len(), crate::constants::LI_HEADER.len() + 3);
        assert_eq!(moke.len(), 5);
        assert_eq!(signal[0], lockin[0]);
        assert_eq!(signal[1], lockin[1]);
        assert_eq!(signal[2], lockin[2]);
        assert_eq!(moke[0], lockin[0]);

        // Nonzero origin and the prepared stride grid are preserved.
        let step = signal[0][1] - signal[0][0];
        assert!(signal[0][0] > FIXTURE_ORIGIN_S);
        assert!((step - FIXTURE_STRIDE_SAMPLES as f64 * FIXTURE_DT_S).abs() < 1e-12);
        assert_eq!(sensor[0].len(), GOLDEN_SENSOR_ROWS);
        assert!(
            (sensor[0][1] - sensor[0][0] - FIXTURE_STRIDE_SAMPLES as f64 * FIXTURE_DT_S).abs()
                < 1e-12,
            "sensor time step {} vs expected {}",
            sensor[0][1] - sensor[0][0],
            FIXTURE_STRIDE_SAMPLES as f64 * FIXTURE_DT_S
        );

        // Independent recomputation of the documented recipes from the
        // recorded provenance: the published signal means and the per-harmonic
        // lock-in XY must equal the shared boxcar geometry applied to the raw
        // waveform on the same grid, support and stride.
        let manifest: toml::Value =
            toml::from_str(&std::fs::read_to_string(paths.analysis_manifest()).unwrap()).unwrap();
        let f_ref = manifest["reference"]["frequency_hz"].as_float().unwrap();
        let omega_tref = manifest["reference"]["phase_rad"].as_float().unwrap();
        let time = data.t.to_vec();
        let dt = time[1] - time[0];

        let expected_mean = pmoke_analysis_core::boxcar_mean(
            &data.channels[3],
            pmoke_analysis_core::BoxcarMeanSettings {
                start_time_s: time[0],
                sample_interval_s: dt,
                half_window_s: FIXTURE_HALF_WINDOW_CYCLES / f_ref,
                stride_samples: FIXTURE_STRIDE_SAMPLES,
            },
        )
        .unwrap();
        assert_eq!(expected_mean.time_s.len(), signal[0].len());
        for (index, (actual, recomputed)) in signal[0]
            .iter()
            .zip(expected_mean.time_s.iter())
            .enumerate()
        {
            assert!(
                (actual - recomputed).abs() <= 1e-12 * (1.0 + recomputed.abs()),
                "signal grid row {index}: {actual} vs recomputed {recomputed}"
            );
        }
        for (index, (actual, recomputed)) in
            signal[3].iter().zip(expected_mean.mean.iter()).enumerate()
        {
            assert!(
                (actual - recomputed).abs() <= 1e-12 * (1.0 + recomputed.abs()),
                "signal mean row {index}: {actual} vs recomputed {recomputed}"
            );
        }

        let finite = pmoke_analysis_core::FiniteSignal::new(&data.channels[2]).unwrap();
        for harmonic in 1..=6usize {
            let pair = pmoke_analysis_core::analyze_boxcar_legacy_pair_finite(
                finite,
                pmoke_analysis_core::BoxcarLegacySettings {
                    start_time_s: time[0],
                    sample_interval_s: dt,
                    reference_frequency_hz: f_ref,
                    reference_phase_rad: omega_tref,
                    half_window_cycles: FIXTURE_HALF_WINDOW_CYCLES,
                    stride_samples: FIXTURE_STRIDE_SAMPLES,
                    harmonic,
                },
            )
            .unwrap();
            for (axis, recomputed) in [("LIx", &pair.x), ("LIy", &pair.y)] {
                let column = &lockin[3 + (harmonic - 1) * 2 + usize::from(axis == "LIy")];
                assert_eq!(column.len(), recomputed.len());
                for (index, (actual, recomputed)) in
                    column.iter().zip(recomputed.iter()).enumerate()
                {
                    assert!(
                        (actual - recomputed).abs() <= 1e-12 * (1.0 + recomputed.abs()),
                        "{axis}_h{harmonic} row {index}: {actual} vs recomputed {recomputed}"
                    );
                }
            }
        }

        // Golden samples recorded from the pre-change run (same fixture):
        // the reordered pipeline must keep publishing these values.
        assert_close("signal mean first row", signal[3][0], 0.2003571132229743);
        assert_close(
            "signal mean middle row",
            signal[3][GOLDEN_ROWS / 2],
            0.20003389244237385,
        );
        assert_close(
            "signal mean last row",
            signal[3][GOLDEN_ROWS - 1],
            0.2092113987318673,
        );
        assert_close(
            "lock-in LIx_h1 first row",
            lockin[3][0],
            0.031458144339011736,
        );
        assert_close(
            "lock-in LIx_h5 last row",
            lockin[11][GOLDEN_ROWS - 1],
            -0.015957756941260188,
        );
        assert_close(
            "lock-in LIy_h6 last row",
            lockin[14][GOLDEN_ROWS - 1],
            -0.011699493111255762,
        );
        assert_close("moke angle first row", moke[3][0], 0.03810480895653044);
        assert_close(
            "moke angle last row",
            moke[3][GOLDEN_ROWS - 1],
            -0.145564597008934,
        );
        assert_close(
            "moke Vm last row",
            moke[4][GOLDEN_ROWS - 1],
            1.0581047092921483,
        );
        assert_close(
            "sensor integral last row",
            sensor[2][GOLDEN_SENSOR_ROWS - 1],
            0.11999999999998619,
        );
    }

    #[test]
    fn analyze_without_signal_entries_runs_no_signal_stage_or_plot() {
        let directory = TemporaryDirectory::new();
        let cfg = analyze_regression_config(&directory, &[]);
        let data = synthetic_recorded_waveform(4);
        let mut steps = Vec::new();
        super::run_analyze_inner_observed(&cfg, &data, &mut |step| steps.push(step)).unwrap();

        assert_eq!(
            steps,
            vec![
                super::AnalyzeStep::Sensor,
                super::AnalyzeStep::ReferencePreparation,
                super::AnalyzeStep::Lockin,
                super::AnalyzeStep::Phase,
                super::AnalyzeStep::Moke,
            ]
        );
        assert!(!cfg.paths().signal_csv().exists());
        assert!(!cfg.paths().signal_plot_dir().exists());
        assert!(cfg.paths().lockin_xy_csv(3).is_file());
        assert!(cfg.paths().moke_csv().is_file());
    }

    #[test]
    fn analyze_with_multiple_signals_keeps_combined_csv_order_and_no_channel_figures() {
        let directory = TemporaryDirectory::new();
        let cfg = analyze_regression_config(&directory, &[4, 8]);
        let data = synthetic_recorded_waveform(8);

        // A per-channel figure left by an older generation must not survive
        // into the new published generation.
        let stale = cfg.paths().signal_plot_dir().join("ch4_mean.png");
        std::fs::create_dir_all(stale.parent().unwrap()).unwrap();
        std::fs::write(&stale, b"\x89PNG\r\n\x1a\nstale").unwrap();

        run_analyze(&cfg, &data).unwrap();

        assert!(!cfg.paths().signal_plot_dir().exists());

        // The combined CSV keeps the configured signal order and headers.
        let signal = read_csv(cfg.paths().signal_csv()).unwrap();
        assert_eq!(signal.len(), 5);
        let header = std::fs::read_to_string(cfg.paths().signal_csv()).unwrap();
        assert!(
            header.starts_with(
                "time (s),ch1 rate (T/s),ch1 integral (T),Ch4 DC4 mean (V),Ch8 DC8 mean (V)"
            ),
            "unexpected signal CSV header: {}",
            header.lines().next().unwrap_or("")
        );

        // No signal plot artifact is registered while plotting is disabled.
        let manifest: toml::Value =
            toml::from_str(&std::fs::read_to_string(cfg.paths().analysis_manifest()).unwrap())
                .unwrap();
        assert!(
            !manifest["artifacts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|artifact| artifact["kind"].as_str() == Some("signal_plot"))
        );
    }

    #[test]
    fn analyze_rerun_keeps_signal_outputs_and_manifest_registration() {
        let directory = TemporaryDirectory::new();
        let cfg = analyze_regression_config(&directory, &[4, 8]);
        let data = synthetic_recorded_waveform(8);
        std::fs::create_dir_all(directory.0.join("acquisition")).unwrap();
        std::fs::write(
            directory.0.join("acquisition/manifest.toml"),
            b"schema_version = 1\n",
        )
        .unwrap();

        run_analyze(&cfg, &data).unwrap();
        let first_signal = std::fs::read(cfg.paths().signal_csv()).unwrap();
        run_analyze(&cfg, &data).unwrap();

        // The rerun keeps the signal output generated by the same pipeline.
        assert_eq!(
            std::fs::read(cfg.paths().signal_csv()).unwrap(),
            first_signal
        );

        let manifest: toml::Value =
            toml::from_str(&std::fs::read_to_string(cfg.paths().analysis_manifest()).unwrap())
                .unwrap();
        let artifacts = manifest["artifacts"].as_array().unwrap();
        let signal_artifacts = artifacts
            .iter()
            .filter(|artifact| artifact["kind"].as_str() == Some("signal"))
            .collect::<Vec<_>>();
        assert_eq!(signal_artifacts.len(), 1);
        assert_eq!(
            signal_artifacts[0]["csv"].as_str(),
            Some("signal/signal.csv")
        );
        assert!(
            !artifacts
                .iter()
                .any(|artifact| artifact["kind"].as_str() == Some("signal_plot"))
        );
        assert!(!artifacts.iter().any(|artifact| {
            artifact
                .get("file")
                .and_then(toml::Value::as_str)
                .is_some_and(|file| file.contains("_mean.png"))
        }));
        assert_eq!(manifest["generation"].as_integer(), Some(2));
        assert_eq!(manifest["published_through"].as_str(), Some("moke"));
        // The retained acquisition input stays readable and referenced.
        assert_eq!(
            std::fs::read(directory.0.join("acquisition/manifest.toml")).unwrap(),
            b"schema_version = 1\n"
        );
        assert_eq!(
            manifest["source_acquisition"].as_str(),
            Some("../acquisition/manifest.toml")
        );
    }

    #[test]
    fn failed_analyze_rerun_keeps_the_previous_generation_intact() {
        let directory = TemporaryDirectory::new();
        let cfg = analyze_regression_config(&directory, &[4]);
        let data = synthetic_recorded_waveform(4);
        run_analyze(&cfg, &data).unwrap();

        let published_signal = std::fs::read(cfg.paths().signal_csv()).unwrap();
        let published_lockin = std::fs::read(cfg.paths().lockin_xy_csv(3)).unwrap();
        let published_manifest = std::fs::read(cfg.paths().analysis_manifest()).unwrap();

        // Break a stage after signal/lock-in have already produced staging
        // output, so a partial publish would be observable.
        let mut broken = cfg.clone();
        broken.moke.use_sensor_ch = 7;
        let error = run_analyze(&broken, &data).unwrap_err();
        assert!(
            format!("{error:#}").contains("missing from channels"),
            "unexpected failure: {error:#}"
        );

        assert_eq!(
            std::fs::read(cfg.paths().signal_csv()).unwrap(),
            published_signal
        );
        assert_eq!(
            std::fs::read(cfg.paths().lockin_xy_csv(3)).unwrap(),
            published_lockin
        );
        assert_eq!(
            std::fs::read(cfg.paths().analysis_manifest()).unwrap(),
            published_manifest
        );
        assert!(!cfg.paths().signal_plot_dir().exists());
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
    fn waveform_read_failure_is_recorded_as_an_analysis_attempt() {
        let directory = TemporaryDirectory::new();
        let mut cfg = crate::test_support::test_config(vec![1], vec![3]);
        cfg.roles.reference_ch = 2;
        cfg.set_artifact_root(directory.0.clone());
        cfg.instruments = Some(crate::config::Instruments {
            function_generator: None,
            oscilloscope: crate::config::Oscilloscope {
                connection: crate::config::Connection::Tcpip {
                    ip: "127.0.0.1".to_string(),
                    port: 55255,
                },
                model: "DHO5108".to_string(),
            },
        });
        std::fs::write(directory.0.join("raw.csv"), b"ch1,ch2,ch3\n0,0,0\n").unwrap();

        analyze(&cfg).unwrap_err();
        let run: toml::Value =
            toml::from_str(&std::fs::read_to_string(cfg.paths().run_manifest()).unwrap()).unwrap();
        assert_eq!(
            run["analysis"]["last_attempt"]["command"].as_str(),
            Some("analysis")
        );
        assert_eq!(
            run["analysis"]["last_attempt"]["status"].as_str(),
            Some("failed")
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

    fn waveform_with_explicit_time(time: Vec<f64>) -> WaveformData {
        let sample_count = time.len();
        WaveformData {
            t: time.into(),
            channels: vec![(0..sample_count).map(|index| index as f64).collect()],
        }
    }

    #[test]
    fn waveform_preflight_rejects_non_finite_explicit_time() {
        for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let error =
                validate_waveform_data(&waveform_with_explicit_time(vec![0.0, 1.0, invalid]))
                    .unwrap_err();
            assert!(
                error.to_string().contains("waveform time is non-finite"),
                "unexpected error for {invalid}: {error}"
            );
        }
    }

    #[test]
    fn waveform_preflight_rejects_repeated_or_decreasing_explicit_time() {
        for time in [
            vec![0.0, 0.0, 1.0],
            vec![0.0, 1.0, 1.0, 2.0],
            vec![0.0, 1.0, 0.5],
        ] {
            let error = validate_waveform_data(&waveform_with_explicit_time(time)).unwrap_err();
            assert!(
                error.to_string().contains("time step"),
                "unexpected time-axis error: {error}"
            );
        }
    }

    #[test]
    fn waveform_preflight_accepts_explicit_time_serialization_roundoff() {
        let data = waveform_with_explicit_time(vec![0.0, 0.1, 0.20000001, 0.3]);

        validate_waveform_data(&data).unwrap();
    }

    #[test]
    fn synthetic_harmonics_pipeline_recovers_folded_angle() {
        let directory = TemporaryDirectory::new();
        let mut cfg = crate::test_support::test_config(vec![1], vec![3]);
        cfg.source_path = directory.0.join("config.toml");
        cfg.set_artifact_root(directory.0.clone());
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
        cfg.phase.m_omega_t0_offset = vec![0.0; 6];
        cfg.moke.moke_type = MokeType::Harmonics;

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

        let columns = read_csv(cfg.paths().moke_csv()).unwrap();
        // Layout: time, sensor rate, sensor integral, angle per signal
        // channel, Vm per signal channel.
        let sensor_columns = cfg.roles.sensor_ch.len();
        let angle = &columns[1 + 2 * sensor_columns];
        let vm = columns.last().unwrap();
        assert!(!angle.is_empty());
        let expected = 0.5 * (2.0 * theta).tan().atan();
        let maximum_error = angle
            .iter()
            .map(|value| (value - expected).abs())
            .fold(0.0_f64, f64::max);
        assert!(
            maximum_error < 2.0e-4,
            "expected {expected}, maximum error was {maximum_error}"
        );
        assert!(
            vm.iter().all(|value| value.is_finite() && *value >= 0.0),
            "Vm must be a finite magnitude"
        );
        assert!(cfg.paths().analysis_manifest().is_file());
    }

    #[test]
    fn run_analyze_supports_repeated_runs_without_force() {
        let directory = TemporaryDirectory::new();
        let mut cfg = crate::test_support::test_config(vec![1], vec![3]);
        cfg.source_path = directory.0.join("config.toml");
        cfg.set_artifact_root(directory.0.clone());
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
        cfg.phase.m_omega_t0_offset = vec![0.0; 6];
        cfg.moke.moke_type = MokeType::Harmonics;

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

        std::fs::create_dir_all(directory.0.join("acquisition")).unwrap();
        std::fs::write(
            directory.0.join("acquisition/manifest.toml"),
            b"schema_version = 1\n",
        )
        .unwrap();
        crate::commands::run_dir::prepare(&cfg).unwrap();
        let acquisition_config = std::fs::read(cfg.paths().source_config()).unwrap();

        // First run succeeds
        run_analyze(&cfg, &data).unwrap();
        assert!(cfg.paths().analysis_manifest().is_file());

        // A changed analysis config is a new generation; acquisition snapshots stay immutable.
        cfg.source_text = Some("version = 3\n# revised analysis\n".to_string());
        cfg.lockin.stride_samples = 25;
        run_analyze(&cfg, &data).unwrap();
        assert!(cfg.paths().analysis_manifest().is_file());
        assert!(!cfg.paths().moke_csv().with_extension("npy").exists());
        assert_eq!(
            std::fs::read(cfg.paths().source_config()).unwrap(),
            acquisition_config
        );
        assert_eq!(
            std::fs::read_to_string(cfg.paths().analysis_source_config()).unwrap(),
            "version = 3\n# revised analysis\n"
        );

        // Parse manifest to verify content
        let manifest_content = std::fs::read_to_string(cfg.paths().analysis_manifest()).unwrap();
        let manifest: toml::Value = toml::from_str(&manifest_content).unwrap();
        assert_eq!(
            manifest["schema_version"].as_integer().unwrap(),
            i64::from(crate::lockin::provenance::ANALYSIS_MANIFEST_SCHEMA_VERSION)
        );
        assert_eq!(manifest["generation"].as_integer(), Some(2));
        assert_eq!(manifest["published_through"].as_str(), Some("moke"));
        assert_eq!(
            manifest["config_source"].as_str(),
            Some("config.source.toml")
        );
        assert_eq!(
            manifest["config_resolved"].as_str(),
            Some("config.resolved.toml")
        );
        assert!(manifest["timestamp"].as_str().is_some());
        assert!(manifest["lockin"].as_table().is_some());
        assert_eq!(manifest["reference"]["channel"].as_integer(), Some(2));
        assert!(manifest["reference"]["frequency_hz"].as_float().unwrap() > 0.0);
        assert_eq!(
            manifest["source_acquisition"].as_str(),
            Some("../acquisition/manifest.toml")
        );
        assert!(manifest.get("source_waveform").is_none());
        let artifacts = manifest["artifacts"].as_array().unwrap();
        let moke_artifact = artifacts
            .iter()
            .find(|artifact| artifact["kind"].as_str() == Some("moke"))
            .unwrap();
        assert!(moke_artifact["rows"].as_integer().unwrap() > 0);
        assert!(moke_artifact["columns"].as_integer().unwrap() > 0);
        assert_eq!(moke_artifact["dtype"].as_str(), Some("<f8"));
        assert_eq!(moke_artifact["order"].as_str(), Some("C"));

        let outputs = manifest["outputs"].as_array().unwrap();
        assert!(
            outputs
                .iter()
                .any(|v| v["file"].as_str().unwrap() == "moke/moke.csv")
        );
        assert!(
            !outputs
                .iter()
                .any(|v| v["file"].as_str().unwrap() == "moke/moke.npy")
        );

        // Third run with save_npy = true succeeds
        cfg.lockin.save_npy = true;
        run_analyze(&cfg, &data).unwrap();

        // Verify NPY is generated
        assert!(cfg.paths().moke_csv().with_extension("npy").exists());
        let manifest_content_2 = std::fs::read_to_string(cfg.paths().analysis_manifest()).unwrap();
        let manifest_2: toml::Value = toml::from_str(&manifest_content_2).unwrap();
        let outputs_2 = manifest_2["outputs"].as_array().unwrap();
        assert!(
            outputs_2
                .iter()
                .any(|v| v["file"].as_str().unwrap() == "moke/moke.csv")
        );
        assert!(
            outputs_2
                .iter()
                .any(|v| v["file"].as_str().unwrap() == "moke/moke.npy")
        );
    }

    #[test]
    fn test_provenance_input_resolution() {
        let temp_dir = TemporaryDirectory::new();
        let root = &temp_dir.0;

        // 1. Canonical RAW
        let canonical_raw_dir = root.join("acquisition");
        std::fs::create_dir_all(&canonical_raw_dir).unwrap();
        std::fs::write(
            canonical_raw_dir.join("manifest.toml"),
            b"schema_version = 1\n",
        )
        .unwrap();

        let resolver = crate::config::ArtifactResolver::new(root);
        let (source_acq, source_wf) =
            crate::lockin::provenance::analysis_sources(root, &resolver).unwrap();
        assert_eq!(source_acq.as_deref(), Some("../acquisition/manifest.toml"));
        assert!(source_wf.is_none());

        // 2. Canonical CSV
        std::fs::remove_dir_all(&canonical_raw_dir).unwrap();
        let canonical_csv_dir = root.join("acquisition/waveforms");
        std::fs::create_dir_all(&canonical_csv_dir).unwrap();
        std::fs::write(canonical_csv_dir.join("waveform.csv"), b"time,ch1\n").unwrap();

        let (source_acq, source_wf) =
            crate::lockin::provenance::analysis_sources(root, &resolver).unwrap();
        assert!(source_acq.is_none());
        assert_eq!(
            source_wf.as_deref(),
            Some("../acquisition/waveforms/waveform.csv")
        );

        // 3. Legacy RAW
        std::fs::remove_dir_all(root.join("acquisition")).unwrap();
        let legacy_raw_dir = root.join("raw_waveform");
        std::fs::create_dir_all(&legacy_raw_dir).unwrap();
        std::fs::write(legacy_raw_dir.join("metadata.toml"), b"version = 1\n").unwrap();

        let (source_acq, source_wf) =
            crate::lockin::provenance::analysis_sources(root, &resolver).unwrap();
        assert_eq!(source_acq.as_deref(), Some("../raw_waveform/metadata.toml"));
        assert!(source_wf.is_none());

        // 4. Legacy CSV
        std::fs::remove_dir_all(&legacy_raw_dir).unwrap();
        std::fs::write(root.join("raw.csv"), b"time,ch1\n").unwrap();

        let (source_acq, source_wf) =
            crate::lockin::provenance::analysis_sources(root, &resolver).unwrap();
        assert!(source_acq.is_none());
        assert_eq!(source_wf.as_deref(), Some("../raw.csv"));

        // 5. Auto resolution
        std::fs::remove_file(root.join("raw.csv")).unwrap();
        let acq_dir = root.join("acquisition");
        std::fs::create_dir_all(acq_dir.join("waveforms")).unwrap();
        std::fs::write(acq_dir.join("manifest.toml"), b"schema_version = 1\n").unwrap();
        std::fs::write(acq_dir.join("waveforms/waveform.csv"), b"time,ch1\n").unwrap();

        let (source_acq, source_wf) =
            crate::lockin::provenance::analysis_sources(root, &resolver).unwrap();
        assert_eq!(source_acq.as_deref(), Some("../acquisition/manifest.toml"));
        assert!(source_wf.is_none());

        // Verification of paths
        if let Some(path) = &source_acq {
            assert!(!path.starts_with('/'));
            assert!(!path.contains('\\'));
            assert!(path.starts_with("../"));
        }
    }

    #[test]
    fn test_analyze_overwrite_safety_and_cleanup() {
        let directory = TemporaryDirectory::new();
        let mut cfg = crate::test_support::test_config(vec![1], vec![3]);
        cfg.roles.reference_ch = 2;
        cfg.set_artifact_root(directory.0.clone());
        let paths = cfg.paths();
        std::fs::create_dir_all(paths.run_dir.join("plots")).unwrap();
        std::fs::write(paths.run_dir.join("plots/user-created.png"), b"user").unwrap();
        for file in [
            paths.lockin_xy_csv(3),
            paths.lockin_rotated_csv(3),
            paths.moke_csv(),
            paths.reference_fit_plot(),
            paths.lockin_xy_combined_plot(),
            paths.phase_rotated_combined_plot(),
            paths.moke_plot(),
            paths.analysis_manifest(),
        ] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"old").unwrap();
        }

        let li = crate::commands::run_dir::prepare_analysis_staging(
            &cfg,
            crate::commands::run_dir::AnalysisStage::Li,
        )
        .unwrap();
        assert!(!li.paths().lockin_xy_csv(3).exists());
        assert!(paths.run_dir.join("plots/user-created.png").is_file());
        std::fs::remove_dir_all(li.paths().analysis_dir()).unwrap();

        let phase = crate::commands::run_dir::prepare_analysis_staging(
            &cfg,
            crate::commands::run_dir::AnalysisStage::Phase,
        )
        .unwrap();
        assert!(phase.paths().lockin_xy_csv(3).is_file());
        assert!(phase.paths().reference_fit_plot().is_file());
        assert!(phase.paths().lockin_xy_combined_plot().is_file());
        assert!(!phase.paths().lockin_rotated_csv(3).exists());
        assert!(!phase.paths().phase_rotated_combined_plot().exists());
        assert!(!phase.paths().moke_csv().exists());
        assert!(!phase.paths().moke_plot().exists());
        std::fs::remove_dir_all(phase.paths().analysis_dir()).unwrap();

        let moke = crate::commands::run_dir::prepare_analysis_staging(
            &cfg,
            crate::commands::run_dir::AnalysisStage::Moke,
        )
        .unwrap();
        assert!(moke.paths().lockin_xy_csv(3).is_file());
        assert!(moke.paths().lockin_rotated_csv(3).is_file());
        assert!(moke.paths().phase_rotated_combined_plot().is_file());
        assert!(!moke.paths().moke_csv().exists());
        assert!(!moke.paths().moke_plot().exists());
    }

    #[test]
    fn test_stage_provenance_cleanup_on_rerun() {
        let directory = TemporaryDirectory::new();
        let mut cfg = crate::test_support::test_config(vec![1], vec![3]);
        cfg.roles.reference_ch = 2;
        cfg.set_artifact_root(directory.0.clone());
        let paths = cfg.paths();

        // 1. Setup a fully analyzed manifest with li, phase, moke stages
        let manifest_path = paths.analysis_manifest();
        std::fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();

        // Also create files so that prepare_analysis_staging carries forward what is not deleted
        for file in [
            paths.lockin_xy_csv(3),
            paths.lockin_rotated_csv(3),
            paths.moke_csv(),
        ] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"data").unwrap();
        }
        for file in [
            paths.reference_fit_plot(),
            paths.lockin_xy_combined_plot(),
            paths.phase_rotated_combined_plot(),
            paths.moke_plot(),
        ] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"\x89PNG\r\n\x1a\n").unwrap();
        }

        let initial_manifest = r#"schema_version = 1
pmoke_version = "0.2.0"
timestamp = "2026-07-12T19:00:00Z"
analyzed_at = "2026-07-12T19:00:00Z"
source_config = "../config.resolved.toml"

[stages.li]
completed_at = "2026-07-12T19:00:00Z"
pmoke_version = "0.2.0"

[stages.phase]
completed_at = "2026-07-12T19:00:01Z"
pmoke_version = "0.2.0"

[stages.moke]
completed_at = "2026-07-12T19:00:02Z"
pmoke_version = "0.2.0"

[stages.kerr]
completed_at = "2026-07-12T19:00:02Z"
pmoke_version = "0.1.0"

[[artifacts]]
kind = "lockin_xy"
channel = 3
csv = "lockin/ch3_xy.csv"

[[artifacts]]
kind = "lockin_rotated"
channel = 3
csv = "lockin/ch3_rotated.csv"

[[artifacts]]
kind = "moke"
csv = "moke/moke.csv"

[[artifacts]]
kind = "kerr"
csv = "kerr/kerr.csv"
"#;
        std::fs::write(&manifest_path, initial_manifest).unwrap();
        let diagnostic_config = paths.diagnostic_source_config("reference");
        std::fs::create_dir_all(diagnostic_config.parent().unwrap()).unwrap();
        std::fs::write(&diagnostic_config, b"version = 3\n").unwrap();

        // 2. Re-run phase staging
        let phase_cfg = crate::commands::run_dir::prepare_analysis_staging(
            &cfg,
            crate::commands::run_dir::AnalysisStage::Phase,
        )
        .unwrap();
        assert_eq!(
            std::fs::read(phase_cfg.paths().diagnostic_source_config("reference")).unwrap(),
            b"version = 3\n"
        );
        crate::commands::run_dir::write_analysis_config_snapshots(&phase_cfg).unwrap();
        crate::lockin::provenance::refresh_analysis_manifest_outputs(&phase_cfg, "phase").unwrap();

        // Check stages after phase re-run: stages.li exists, stages.phase updated, stages.moke/export_npy deleted
        let manifest_content =
            std::fs::read_to_string(phase_cfg.paths().analysis_manifest()).unwrap();
        let manifest: toml::Value = toml::from_str(&manifest_content).unwrap();
        assert!(manifest.get("source_config").is_none());
        assert!(manifest["config_source_sha256"].is_str());
        assert!(manifest["config_resolved_sha256"].is_str());
        let stages = manifest["stages"].as_table().unwrap();
        assert!(stages.contains_key("li"));
        assert!(stages.contains_key("phase"));
        assert!(!stages.contains_key("moke"));
        assert!(!stages.contains_key("kerr"));
        assert!(!stages.contains_key("export_npy"));

        // Check artifacts after phase: lockin_rotated, moke, and legacy kerr should be gone because they are not carried forward
        let artifacts = manifest["artifacts"].as_array().unwrap();
        assert!(
            artifacts
                .iter()
                .any(|a| a["kind"].as_str() == Some("lockin_xy"))
        );
        assert!(
            !artifacts
                .iter()
                .any(|a| a["kind"].as_str() == Some("lockin_rotated"))
        );
        assert!(!artifacts.iter().any(|a| a["kind"].as_str() == Some("moke")));
        assert!(!artifacts.iter().any(|a| a["kind"].as_str() == Some("kerr")));

        // 3. Re-run li staging from the original full state
        std::fs::write(&manifest_path, initial_manifest).unwrap();
        let li_cfg = crate::commands::run_dir::prepare_analysis_staging(
            &cfg,
            crate::commands::run_dir::AnalysisStage::Li,
        )
        .unwrap();
        crate::commands::run_dir::write_analysis_config_snapshots(&li_cfg).unwrap();

        let reference = crate::lockin::reference::ref_analysis::RefFitParams {
            f_ref: 1000.0,
            a_ref: 1.0,
            omega_tref: 0.0,
            f_ref_rel_uncertainty: None,
        };
        let time = (0..2000).map(|i| i as f64 * 1e-5).collect::<Vec<_>>();
        let signal = vec![0.0; 2000];
        let processor = crate::lockin::lockin_core::LockinProcessor::new(
            &time,
            &signal,
            1000.0,
            0.0,
            &cfg.lockin,
        )
        .unwrap();
        let provenance = crate::lockin::provenance::LockinProvenance::from_processor(&processor);
        // Ensure there is at least one lockin csv file in staging so it gets described
        let staging_lockin_csv = li_cfg.paths().lockin_xy_csv(3);
        std::fs::create_dir_all(staging_lockin_csv.parent().unwrap()).unwrap();
        std::fs::write(&staging_lockin_csv, b"time,ch3 rate,ch3 integral\n0,0,0\n").unwrap();

        crate::lockin::provenance::write_analysis_metadata(
            &li_cfg,
            &li_cfg.paths(),
            &cfg.resolver(),
            &reference,
            &provenance,
            2,
        )
        .unwrap();

        let manifest_content_li =
            std::fs::read_to_string(li_cfg.paths().analysis_manifest()).unwrap();
        let manifest_li: toml::Value = toml::from_str(&manifest_content_li).unwrap();
        let stages_li = manifest_li["stages"].as_table().unwrap();
        assert!(stages_li.contains_key("li"));
        assert!(!stages_li.contains_key("phase"));
        assert!(!stages_li.contains_key("moke"));
        assert!(!stages_li.contains_key("kerr"));
    }
}
