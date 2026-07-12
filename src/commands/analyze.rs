use crate::{
    config::Config,
    kerr::run_kerr_analysis,
    lockin::run_li,
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

fn run_analyze_inner(cfg: &Config, data: &WaveformData) -> Result<()> {
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
    let (t_stride, sensor_rate_stride, sensor_integral_stride, li_results, reference, provenance) =
        run_li(&cfg_staging, &data.t, &data.channels)?;

    // run phase analysis here
    let ch = cfg_staging.phase_signal_ch();

    if !ch.is_empty() {
        let li_rotated_results = run_phase_analysis(
            &cfg_staging,
            &t_stride,
            &sensor_rate_stride,
            &sensor_integral_stride,
            &li_results,
        )?;
        drop(li_results);

        // run Kerr analysis here
        run_kerr_analysis(
            &cfg_staging,
            &t_stride,
            &sensor_rate_stride,
            &sensor_integral_stride,
            &li_rotated_results,
        )?;
    } else {
        ui::skipped("phase analysis: no channels specified");
    }

    crate::lockin::provenance::write_analysis_metadata(
        &cfg_staging,
        &cfg_staging.paths(),
        &cfg.resolver(),
        &reference,
        &provenance,
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
    use super::{analyze, run_analyze, validate_waveform_data};
    use crate::config::{KerrType, LockinLpfKind, Window};
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

    #[test]
    fn synthetic_harmonics_pipeline_recovers_folded_kerr_angle() {
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

        let columns = read_csv(cfg.paths().kerr_csv()).unwrap();
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
        assert!(!cfg.paths().kerr_csv().with_extension("npy").exists());
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
        assert_eq!(manifest["published_through"].as_str(), Some("kerr"));
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
        let kerr_artifact = artifacts
            .iter()
            .find(|artifact| artifact["kind"].as_str() == Some("kerr"))
            .unwrap();
        assert!(kerr_artifact["rows"].as_integer().unwrap() > 0);
        assert!(kerr_artifact["columns"].as_integer().unwrap() > 0);
        assert_eq!(kerr_artifact["dtype"].as_str(), Some("<f8"));
        assert_eq!(kerr_artifact["order"].as_str(), Some("C"));

        let outputs = manifest["outputs"].as_array().unwrap();
        assert!(
            outputs
                .iter()
                .any(|v| v["file"].as_str().unwrap() == "kerr/kerr.csv")
        );
        assert!(
            !outputs
                .iter()
                .any(|v| v["file"].as_str().unwrap() == "kerr/kerr.npy")
        );

        // Third run with save_npy = true succeeds
        cfg.lockin.save_npy = true;
        run_analyze(&cfg, &data).unwrap();

        // Verify NPY is generated
        assert!(cfg.paths().kerr_csv().with_extension("npy").exists());
        let manifest_content_2 = std::fs::read_to_string(cfg.paths().analysis_manifest()).unwrap();
        let manifest_2: toml::Value = toml::from_str(&manifest_content_2).unwrap();
        let outputs_2 = manifest_2["outputs"].as_array().unwrap();
        assert!(
            outputs_2
                .iter()
                .any(|v| v["file"].as_str().unwrap() == "kerr/kerr.csv")
        );
        assert!(
            outputs_2
                .iter()
                .any(|v| v["file"].as_str().unwrap() == "kerr/kerr.npy")
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
            paths.kerr_csv(),
            paths.reference_fit_plot(),
            paths.lockin_xy_combined_plot(),
            paths.phase_rotated_combined_plot(),
            paths.kerr_plot(),
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
        assert!(!phase.paths().kerr_csv().exists());
        assert!(!phase.paths().kerr_plot().exists());
        std::fs::remove_dir_all(phase.paths().analysis_dir()).unwrap();

        let kerr = crate::commands::run_dir::prepare_analysis_staging(
            &cfg,
            crate::commands::run_dir::AnalysisStage::Kerr,
        )
        .unwrap();
        assert!(kerr.paths().lockin_xy_csv(3).is_file());
        assert!(kerr.paths().lockin_rotated_csv(3).is_file());
        assert!(kerr.paths().phase_rotated_combined_plot().is_file());
        assert!(!kerr.paths().kerr_csv().exists());
        assert!(!kerr.paths().kerr_plot().exists());
    }

    #[test]
    fn test_stage_provenance_cleanup_on_rerun() {
        let directory = TemporaryDirectory::new();
        let mut cfg = crate::test_support::test_config(vec![1], vec![3]);
        cfg.roles.reference_ch = 2;
        cfg.set_artifact_root(directory.0.clone());
        let paths = cfg.paths();

        // 1. Setup a fully analyzed manifest with li, phase, kerr stages
        let manifest_path = paths.analysis_manifest();
        std::fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();

        // Also create files so that prepare_analysis_staging carries forward what is not deleted
        for file in [
            paths.lockin_xy_csv(3),
            paths.lockin_rotated_csv(3),
            paths.kerr_csv(),
        ] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"data").unwrap();
        }
        for file in [
            paths.reference_fit_plot(),
            paths.lockin_xy_combined_plot(),
            paths.phase_rotated_combined_plot(),
            paths.kerr_plot(),
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

[stages.kerr]
completed_at = "2026-07-12T19:00:02Z"
pmoke_version = "0.2.0"

[[artifacts]]
kind = "lockin_xy"
channel = 3
csv = "lockin/ch3_xy.csv"

[[artifacts]]
kind = "lockin_rotated"
channel = 3
csv = "lockin/ch3_rotated.csv"

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

        // Check stages after phase re-run: stages.li exists, stages.phase updated, stages.kerr/export_npy deleted
        let manifest_content =
            std::fs::read_to_string(phase_cfg.paths().analysis_manifest()).unwrap();
        let manifest: toml::Value = toml::from_str(&manifest_content).unwrap();
        assert!(manifest.get("source_config").is_none());
        assert!(manifest["config_source_sha256"].is_str());
        assert!(manifest["config_resolved_sha256"].is_str());
        let stages = manifest["stages"].as_table().unwrap();
        assert!(stages.contains_key("li"));
        assert!(stages.contains_key("phase"));
        assert!(!stages.contains_key("kerr"));
        assert!(!stages.contains_key("export_npy"));

        // Check artifacts after phase: lockin_rotated and kerr should be gone because they are not carried forward
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
        assert!(!stages_li.contains_key("kerr"));
    }
}
