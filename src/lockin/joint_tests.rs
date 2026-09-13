use super::*;
use crate::config::{
    GlsCovarianceOutput, GlsFailurePolicy, GlsNoiseMode, JointHarmonicGlsConfig, LockinEstimator,
    LockinWindow,
};
use crate::lockin::lockin_core::LockinProcessor;
use crate::test_support::test_config;
use crate::utils::time_axis::TimeAxisRef;
use std::f64::consts::PI;

const SHA_A: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn tone_waveform() -> (Vec<f64>, Vec<f64>) {
    tone_waveform_with(3_000)
}

fn tone_waveform_with(samples: usize) -> (Vec<f64>, Vec<f64>) {
    let dt = 1.0e-5;
    let f_ref = 1_000.0;
    let time: Vec<f64> = (0..samples).map(|index| index as f64 * dt).collect();
    let signal: Vec<f64> = time
        .iter()
        .map(|&t| {
            1.0 * (2.0 * PI * f_ref * t + 0.3).sin() + 0.2 * (2.0 * PI * 3.0 * f_ref * t).sin()
        })
        .collect();
    (time, signal)
}

fn joint_lockin() -> crate::config::Lockin {
    let mut lockin = test_config(vec![1], vec![3]).lockin;
    lockin.workers = 1;
    lockin.stride_samples = 10;
    lockin.lpf_half_window_cycles = 1.0;
    lockin.window = LockinWindow::legacy_boxcar(1.0);
    lockin.estimator = LockinEstimator::JointHarmonicGls(JointHarmonicGlsConfig {
        fit_harmonics: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        output_harmonics: vec![1, 2, 3, 4, 5, 6],
        envelope_degree: 0,
        noise_mode: GlsNoiseMode::Identity,
        covariance_output: GlsCovarianceOutput::Diagonal,
        failure_policy: GlsFailurePolicy::Error,
        calibrations: vec![crate::config::EstimatorCalibration {
            channel: 3,
            path: "synthetic".to_string(),
            sha256: SHA_A.to_string(),
        }],
    });
    lockin
}

fn test_inputs<'a>(
    lockin: &'a crate::config::Lockin,
    gls: &'a JointHarmonicGlsConfig,
    time: &'a [f64],
) -> JointRunInputs<'a> {
    JointRunInputs {
        lockin,
        gls,
        t: TimeAxisRef::Explicit(time),
        f_ref: 1_000.0,
        omega_tref: 0.0,
        sample_rate: 100_000.0,
    }
}

#[test]
fn identity_recovers_tone_matching_boxcar() {
    let (time, signal) = tone_waveform();
    let lockin = joint_lockin();
    let source = SyntheticNoiseModelSource::identity(0.01);
    let LockinEstimator::JointHarmonicGls(gls) = &lockin.estimator else {
        unreachable!()
    };
    let inputs = test_inputs(&lockin, gls, &time);
    let output = run_joint_li(&inputs, &[3], &[signal.as_slice()], &source).unwrap();

    assert_eq!(output.result.len(), 1);
    assert_eq!(output.result[0].len(), 12);
    let grid = output.result[0][0].len();
    assert!(grid > 100);
    assert_eq!(output.quality.len(), 1);
    assert_eq!(output.quality[0].len(), grid);
    assert_eq!(output.base_index_range, output.output_index_range);

    // Boxcar reference over the identical support grid.
    let mut boxcar_lockin = test_config(vec![1], vec![3]).lockin;
    boxcar_lockin.stride_samples = 10;
    boxcar_lockin.lpf_half_window_cycles = 1.0;
    boxcar_lockin.window = LockinWindow::legacy_boxcar(1.0);
    boxcar_lockin.estimator = LockinEstimator::BoxcarLegacy;
    let processor = LockinProcessor::new(
        TimeAxisRef::Explicit(&time),
        &signal,
        1_000.0,
        0.0,
        &boxcar_lockin,
    )
    .unwrap();
    for harmonic in 1..=6usize {
        let reference = processor.compute_harmonic_detailed(harmonic, false);
        assert_eq!(reference.li_x.len(), grid);
        for (index, (&got, &want)) in output.result[0][2 * (harmonic - 1)]
            .iter()
            .zip(reference.li_x.iter())
            .enumerate()
        {
            let tolerance = 1e-9 * want.abs().max(1.0);
            assert!(
                (got - want).abs() <= tolerance,
                "h{harmonic} x[{index}]: joint {got} vs boxcar {want}"
            );
        }
        for (index, (&got, &want)) in output.result[0][2 * (harmonic - 1) + 1]
            .iter()
            .zip(reference.li_y.iter())
            .enumerate()
        {
            let tolerance = 1e-9 * want.abs().max(1.0);
            assert!(
                (got - want).abs() <= tolerance,
                "h{harmonic} y[{index}]: joint {got} vs boxcar {want}"
            );
        }
    }

    // Quality diagnostics: exact fit, full rank, no jitter.
    for row in &output.quality[0] {
        assert!(row.residual_rms < 1e-9, "residual {}", row.residual_rms);
        assert_eq!(row.rank, 25);
        assert!(row.condition.is_finite() && row.condition > 0.0);
        assert_eq!(row.jitter_applied_v2, 0.0);
        assert_eq!(row.noise_mode, "identity");
    }

    // h1 recovers the tone amplitude as a half-amplitude (boxcar scale).
    let x1 = output.result[0][0][grid / 2];
    let y1 = output.result[0][1][grid / 2];
    let amplitude = (x1 * x1 + y1 * y1).sqrt();
    assert!(
        (amplitude - 0.5).abs() < 1e-9,
        "h1 half-amplitude {amplitude}"
    );
}

#[test]
fn failure_is_typed_and_leaves_no_files() {
    let (time, signal) = tone_waveform_with(1_500);
    let lockin = joint_lockin();
    let gls = match &lockin.estimator {
        LockinEstimator::JointHarmonicGls(gls) => gls.clone(),
        LockinEstimator::BoxcarLegacy => unreachable!(),
    };
    // Indefinite kernel with zero jitter budget: the solver must fail.
    let source = SyntheticNoiseModelSource {
        reference_variance_v2: 0.01,
        variance_bins: None,
        correlation_lags: Some(vec![1.0, 1.0]),
        correlation_lag_step_s: 1.0e-5,
        label: "indefinite".to_string(),
    };
    let mut gls_correlated = gls.clone();
    gls_correlated.noise_mode = GlsNoiseMode::StationaryCorrelated;
    let mut lockin_correlated = joint_lockin();
    lockin_correlated.estimator = LockinEstimator::JointHarmonicGls(gls_correlated);
    let gls_ref = match &lockin_correlated.estimator {
        LockinEstimator::JointHarmonicGls(gls) => gls,
        LockinEstimator::BoxcarLegacy => unreachable!(),
    };
    let inputs = test_inputs(&lockin_correlated, gls_ref, &time);
    let error = run_joint_li(&inputs, &[3], &[signal.as_slice()], &source)
        .err()
        .unwrap();
    let message = format!("{error:#}");
    assert!(message.contains("stage=lockin"), "{message}");
    assert!(message.contains("channel=3"), "{message}");
    assert!(message.contains("window="), "{message}");
    assert!(message.contains("refusing to fall back"), "{message}");

    // Missing per-mode data fails at the loader with the channel attached.
    let bare = SyntheticNoiseModelSource::identity(0.01);
    let mut gls_diagonal = gls.clone();
    gls_diagonal.noise_mode = GlsNoiseMode::PhaseDiagonal;
    let inputs = test_inputs(&lockin, &gls_diagonal, &time);
    let error = run_joint_li(&inputs, &[3], &[signal.as_slice()], &bare)
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("channel 3"), "{error:#}");

    // The quality writer never overwrites: a pre-existing file aborts.
    let dir = std::env::temp_dir().join(format!("pmoke_joint_quality_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ch3_quality.csv");
    std::fs::write(&path, b"old").unwrap();
    let rows = vec![QualityRow {
        time_s: 0.0,
        residual_rms: 0.0,
        rank: 25,
        condition: 1.0,
        jitter_applied_v2: 0.0,
        noise_mode: "identity",
    }];
    assert!(write_quality_csv(&path, &rows).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"old");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn quality_csv_shape_round_trips() {
    let dir =
        std::env::temp_dir().join(format!("pmoke_joint_quality_{}_shape", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("ch3_quality.csv");
    let rows = vec![
        QualityRow {
            time_s: 0.001,
            residual_rms: 1e-12,
            rank: 25,
            condition: 3.5,
            jitter_applied_v2: 0.0,
            noise_mode: "identity",
        },
        QualityRow {
            time_s: 0.002,
            residual_rms: 2e-12,
            rank: 25,
            condition: 3.6,
            jitter_applied_v2: 0.0,
            noise_mode: "identity",
        },
    ];
    write_quality_csv(&path, &rows).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines = text.lines();
    assert_eq!(
        lines.next().unwrap(),
        "time_s,residual_rms,rank,condition,jitter_applied_v2,noise_mode"
    );
    assert_eq!(lines.count(), 2);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn provenance_names_the_joint_estimator() {
    let (time, signal) = tone_waveform_with(1_500);
    let lockin = joint_lockin();
    let source = SyntheticNoiseModelSource::identity(0.01);
    let gls = match &lockin.estimator {
        LockinEstimator::JointHarmonicGls(gls) => gls,
        LockinEstimator::BoxcarLegacy => unreachable!(),
    };
    let inputs = test_inputs(&lockin, gls, &time);
    let output = run_joint_li(&inputs, &[3], &[signal.as_slice()], &source).unwrap();
    let encoded = serde_json::to_value(&output.provenance).unwrap();
    assert_eq!(encoded["kind"], "joint_harmonic_gls");
    assert_eq!(encoded["estimator"], "joint_harmonic_gls");
    assert_eq!(encoded["noise_mode"], "identity");
    assert_eq!(encoded["solver"], JOINT_SOLVER_ID);
    assert!(encoded["estimated_enbw_hz"].is_null());
    assert!(encoded["model_digests"][0]["channel"] == 3);
}

#[test]
fn joint_execution_is_deterministic_across_channels() {
    let (time, signal) = tone_waveform_with(1_500);
    let lockin = joint_lockin();
    let source = SyntheticNoiseModelSource::identity(0.01);
    let gls = match &lockin.estimator {
        LockinEstimator::JointHarmonicGls(gls) => gls,
        LockinEstimator::BoxcarLegacy => unreachable!(),
    };
    let run = |channels: &[u8], data: &[&[f64]]| {
        let inputs = test_inputs(&lockin, gls, &time);
        run_joint_li(&inputs, channels, data, &source).unwrap()
    };
    let first = run(&[3], &[signal.as_slice()]);
    let second = run(&[3], &[signal.as_slice()]);
    assert_eq!(first.result, second.result);
    // Two identical channels share ranges and agree exactly.
    let dual = run(&[3, 4], &[signal.as_slice(), signal.as_slice()]);
    assert_eq!(dual.base_index_range, first.base_index_range);
    assert_eq!(dual.output_index_range, first.output_index_range);
    assert_eq!(dual.result[0], dual.result[1]);
    assert_eq!(dual.result[0], first.result[0]);
}

#[test]
fn stage_fingerprint_distinguishes_estimators() {
    let boxcar = test_config(vec![1], vec![3]);
    let mut gls_cfg = test_config(vec![1], vec![3]);
    gls_cfg.lockin = joint_lockin();
    for stage in ["li", "phase", "moke", "signal"] {
        let boxcar_print =
            crate::lockin::provenance::stage_config_fingerprint(&boxcar, stage).unwrap();
        let gls_print =
            crate::lockin::provenance::stage_config_fingerprint(&gls_cfg, stage).unwrap();
        assert_ne!(boxcar_print, gls_print, "stage {stage}");
        assert_eq!(
            gls_print,
            crate::lockin::provenance::stage_config_fingerprint(&gls_cfg, stage).unwrap()
        );
    }
    // Model digest changes invalidate.
    let mut gls_other = test_config(vec![1], vec![3]);
    gls_other.lockin = joint_lockin();
    if let LockinEstimator::JointHarmonicGls(gls) = &mut gls_other.lockin.estimator {
        gls.fit_harmonics.pop();
        gls.fit_harmonics.push(13);
    }
    assert_ne!(
        crate::lockin::provenance::stage_config_fingerprint(&gls_cfg, "li").unwrap(),
        crate::lockin::provenance::stage_config_fingerprint(&gls_other, "li").unwrap()
    );
}

#[test]
fn quality_artifact_registers_through_manifest_refresh() {
    let dir = std::env::temp_dir().join(format!("pmoke_joint_manifest_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = test_config(vec![1], vec![3]);
    cfg.set_artifact_root(dir.clone());
    let paths = cfg.paths();
    std::fs::create_dir_all(paths.analysis_dir().join("lockin")).unwrap();
    // Minimal sibling xy artifact so the refresh publishes through "li".
    std::fs::write(paths.lockin_xy_csv(3), "time (s),x1\n0.0,0.5\n0.001,0.5\n").unwrap();
    write_quality_csv(
        &paths.lockin_quality_csv(3),
        &[QualityRow {
            time_s: 0.0,
            residual_rms: 1e-12,
            rank: 25,
            condition: 3.5,
            jitter_applied_v2: 0.0,
            noise_mode: "identity",
        }],
    )
    .unwrap();
    std::fs::write(paths.analysis_manifest(), "schema_version = 3\n").unwrap();
    std::fs::write(paths.analysis_source_config(), b"version = 3\n").unwrap();
    std::fs::write(paths.analysis_resolved_config(), b"version = 3\n").unwrap();
    crate::lockin::provenance::refresh_analysis_manifest_outputs(&cfg, "li").unwrap();

    let manifest = std::fs::read_to_string(paths.analysis_manifest()).unwrap();
    let value: toml::Value = toml::from_str(&manifest).unwrap();
    let artifacts = value["artifacts"].as_array().unwrap();
    let quality = artifacts
        .iter()
        .find(|artifact| artifact["kind"].as_str() == Some("lockin_quality"))
        .expect("lockin_quality artifact missing");
    assert_eq!(quality["channel"].as_integer(), Some(3));
    assert_eq!(quality["rows"].as_integer(), Some(1));
    assert_eq!(quality["columns"].as_integer(), Some(6));
    assert!(quality.get("dtype").is_none(), "{quality}");
    assert!(quality.get("order").is_none(), "{quality}");
    assert!(quality.get("npy").is_none(), "{quality}");
    let xy = artifacts
        .iter()
        .find(|artifact| artifact["kind"].as_str() == Some("lockin_xy"))
        .expect("lockin_xy artifact missing");
    assert_eq!(xy["dtype"].as_str(), Some("<f8"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn window_support_matches_boxcar_taps() {
    // A unit impulse at input j may influence only outputs whose window
    // covers j: |center - j| <= n_half + 1 with centers on the strided grid.
    // The marginal tap (distance exactly n_half + 1) pins the support: a
    // one-tap mutant silences the marginal output. Note the boxcar gives
    // those two edge taps zero quadrature weight when edge_dt == 0 while
    // the GLS fits them; the contract is sample-set equality, exact for
    // bandlimited content.
    for (impulse_at, inside_center, outside_center) in [
        (549usize, 650usize, 660usize),
        (751usize, 650usize, 640usize),
    ] {
        check_impulse_support(impulse_at, inside_center, outside_center);
    }
}

#[allow(clippy::too_many_lines)]
fn check_impulse_support(impulse_at: usize, inside_center: usize, outside_center: usize) {
    let dt = 1.0e-5;
    let samples = 3_000usize;
    let time: Vec<f64> = (0..samples).map(|index| index as f64 * dt).collect();
    let mut signal = vec![0.0; samples];
    signal[impulse_at] = 1.0;

    let lockin = joint_lockin();
    let source = SyntheticNoiseModelSource::identity(0.01);
    let gls = match &lockin.estimator {
        LockinEstimator::JointHarmonicGls(gls) => gls,
        LockinEstimator::BoxcarLegacy => unreachable!(),
    };
    let inputs = test_inputs(&lockin, gls, &time);
    let output = run_joint_li(&inputs, &[3], &[signal.as_slice()], &source).unwrap();
    let grid = output.result[0][0].len();
    // centers c = (i_start + o) * stride with i_start = 12, stride = 10.
    let support_radius = 100 + 1;
    let peak_at = |center: usize| {
        let output_index = center / 10 - 12;
        output.result[0]
            .iter()
            .map(|column| column[output_index].abs())
            .fold(0.0f64, f64::max)
    };
    // The marginal pair pins the support exactly.
    assert!(
        peak_at(inside_center) > 1e-6,
        "marginal output at center {inside_center} missed impulse {impulse_at}"
    );
    assert!(
        peak_at(outside_center) < 1e-9,
        "output at center {outside_center} leaked outside support"
    );
    for output_index in 0..grid {
        let center = (12 + output_index) * 10;
        let distance = center.abs_diff(impulse_at);
        let peak = output.result[0]
            .iter()
            .map(|column| column[output_index].abs())
            .fold(0.0f64, f64::max);
        if distance <= support_radius {
            assert!(
                peak > 1e-6,
                "output {output_index} (center {center}) missed the impulse: {peak}"
            );
        } else {
            assert!(
                peak < 1e-9,
                "output {output_index} (center {center}) leaked outside support: {peak}"
            );
        }
    }

    // The legacy estimator honors the identical boundary, except for the two
    // edge taps, which carry zero quadrature weight when edge_dt == 0.
    let mut boxcar_lockin = test_config(vec![1], vec![3]).lockin;
    boxcar_lockin.stride_samples = 10;
    boxcar_lockin.lpf_half_window_cycles = 1.0;
    boxcar_lockin.window = LockinWindow::legacy_boxcar(1.0);
    boxcar_lockin.estimator = LockinEstimator::BoxcarLegacy;
    let processor = LockinProcessor::new(
        TimeAxisRef::Explicit(&time),
        &signal,
        1_000.0,
        0.0,
        &boxcar_lockin,
    )
    .unwrap();
    let reference = processor.compute_harmonic_detailed(1, false);
    assert_eq!(reference.li_x.len(), grid);
    for (output_index, (&x, &y)) in reference.li_x.iter().zip(reference.li_y.iter()).enumerate() {
        let center = (12 + output_index) * 10;
        let distance = center.abs_diff(impulse_at);
        let peak = x.abs().max(y.abs());
        if distance < support_radius {
            assert!(peak > 1e-12, "boxcar output {output_index} missed");
        } else {
            assert!(peak == 0.0, "boxcar output {output_index} leaked");
        }
    }
}
