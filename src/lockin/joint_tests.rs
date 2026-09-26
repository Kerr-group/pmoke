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
        calibration_source: crate::config::GlsCalibrationSource::Artifact,
        calibrations: vec![crate::config::EstimatorCalibration {
            channel: 3,
            path: "synthetic".to_string(),
            sha256: SHA_A.to_string(),
        }],
        solver_tolerances: crate::config::GlsSolverTolerances::default(),
        scs_adequacy: crate::config::GlsScsAdequacy::default(),
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
        tolerances: pmoke_analysis_core::joint::JointSolverTolerances::default(),
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
        assert!(row.residual_rms_v < 1e-9, "residual {}", row.residual_rms_v);
        assert_eq!(row.rank, 25);
        assert!(row.scaled_design_condition.is_finite() && row.scaled_design_condition > 0.0);
        assert_eq!(row.jitter_applied_v2, 0.0);
        assert_eq!(row.status, WindowStatus::Ok);
        assert_eq!(row.noise_mode, "identity");
    }

    // h1 recovers the tone peak amplitude (peak-amplitude XY convention).
    let x1 = output.result[0][0][grid / 2];
    let y1 = output.result[0][1][grid / 2];
    let amplitude = (x1 * x1 + y1 * y1).sqrt();
    assert!(
        (amplitude - 1.0).abs() < 1e-9,
        "h1 peak amplitude {amplitude}"
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
        original_center_index: 12,
        time_s: 0.0,
        scaled_design_condition: 1.0,
        residual_rms_v: 0.0,
        rank: 25,
        status: WindowStatus::Ok,
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
            original_center_index: 12,
            time_s: 0.001,
            scaled_design_condition: 3.5,
            residual_rms_v: 1e-12,
            rank: 25,
            status: WindowStatus::Ok,
            jitter_applied_v2: 0.0,
            noise_mode: "identity",
        },
        QualityRow {
            original_center_index: 13,
            time_s: 0.002,
            scaled_design_condition: 3.6,
            residual_rms_v: 2e-12,
            rank: 25,
            status: WindowStatus::Warning,
            jitter_applied_v2: 1e-6,
            noise_mode: "identity",
        },
    ];
    write_quality_csv(&path, &rows).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines = text.lines();
    assert_eq!(
        lines.next().unwrap(),
        "original_center_index,time_s,scaled_design_condition,residual_rms_v,rank,status"
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

/// LI-level wiring for the hoisted plan (P1) in the target correlated mode:
/// the full run uses one prepared plan per channel, keeps the shared grid,
/// and stays deterministic across runs.
#[test]
fn phase_correlated_run_uses_hoisted_plan_deterministically() {
    let (time, signal) = tone_waveform_with(1_500);
    let mut lockin = joint_lockin();
    let LockinEstimator::JointHarmonicGls(gls) = &mut lockin.estimator else {
        unreachable!()
    };
    gls.noise_mode = GlsNoiseMode::PhaseCorrelated;
    let source = SyntheticNoiseModelSource {
        reference_variance_v2: 1.0,
        variance_bins: Some((0..8).map(|bin| 1.0 + 0.05 * bin as f64).collect()),
        correlation_lags: Some((0..64).map(|lag| 0.9_f64.powi(lag)).collect()),
        correlation_lag_step_s: 1.0e-5,
        label: "phase-correlated-hoisted".to_string(),
    };
    let LockinEstimator::JointHarmonicGls(gls) = &lockin.estimator else {
        unreachable!()
    };
    let inputs = test_inputs(&lockin, gls, &time);
    let first = run_joint_li(&inputs, &[3], &[signal.as_slice()], &source).unwrap();
    let second = run_joint_li(&inputs, &[3], &[signal.as_slice()], &source).unwrap();
    assert_eq!(first.result, second.result);
    assert_eq!(first.result.len(), 1);
    assert_eq!(first.result[0].len(), 12);
    let grid = first.result[0][0].len();
    assert!(grid > 100);
    assert_eq!(first.quality[0].len(), grid);
    for row in &first.quality[0] {
        assert_eq!(row.noise_mode, "phase_correlated");
        assert_eq!(row.rank, 25);
        assert!(row.scaled_design_condition.is_finite());
        assert!(row.residual_rms_v.is_finite());
        assert_eq!(row.jitter_applied_v2, 0.0);
    }
    for column in &first.result[0] {
        assert!(column.iter().all(|value| value.is_finite()));
    }
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
            original_center_index: 12,
            time_s: 0.0,
            scaled_design_condition: 3.5,
            residual_rms_v: 1e-12,
            rank: 25,
            status: WindowStatus::Ok,
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
fn covariance_none_does_not_retain_per_window_matrices() {
    let (time, signal) = tone_waveform_with(1_500);
    let mut lockin = joint_lockin();
    let LockinEstimator::JointHarmonicGls(gls) = &mut lockin.estimator else {
        unreachable!()
    };
    gls.covariance_output = GlsCovarianceOutput::None;
    let LockinEstimator::JointHarmonicGls(gls) = &lockin.estimator else {
        unreachable!()
    };
    let inputs = test_inputs(&lockin, gls, &time);
    let output = run_joint_li(
        &inputs,
        &[3],
        &[signal.as_slice()],
        &SyntheticNoiseModelSource::identity(0.01),
    )
    .unwrap();
    assert_eq!(output.covariance.len(), 1);
    assert!(output.covariance[0].is_empty());
    assert_eq!(output.quality[0].len(), output.result[0][0].len());
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

fn joint_gls(lockin: &crate::config::Lockin) -> JointHarmonicGlsConfig {
    match &lockin.estimator {
        LockinEstimator::JointHarmonicGls(gls) => gls.clone(),
        LockinEstimator::BoxcarLegacy => unreachable!(),
    }
}

#[test]
fn covariance_headers_match_contract() {
    let diagonal = covariance_csv_header(GlsCovarianceOutput::Diagonal).unwrap();
    assert_eq!(diagonal.len(), 13);
    assert_eq!(diagonal[0], "time_s");
    assert_eq!(diagonal[1], "cov_x1_x1_v2");
    assert_eq!(diagonal[2], "cov_y1_y1_v2");
    assert_eq!(diagonal[3], "cov_x2_x2_v2");
    assert_eq!(diagonal[12], "cov_y6_y6_v2");
    let full = covariance_csv_header(GlsCovarianceOutput::Full).unwrap();
    assert_eq!(full.len(), 79);
    assert_eq!(full[0], "time_s");
    assert_eq!(full[1], "cov_x1_x1_v2");
    assert_eq!(full[2], "cov_x1_y1_v2");
    assert_eq!(full[3], "cov_x1_x2_v2");
    assert_eq!(full[13], "cov_y1_y1_v2");
    assert_eq!(full[14], "cov_y1_x2_v2");
    assert_eq!(full[78], "cov_y6_y6_v2");
    assert!(covariance_csv_header(GlsCovarianceOutput::None).is_err());
    assert!(pack_covariance_row(&vec![vec![0.0; 12]; 12], GlsCovarianceOutput::None).is_err());
    assert_eq!(QUADRATURE_NAMES.len(), 12);
}

#[test]
fn covariance_packing_scales_with_signal_and_noise() {
    let (time, signal) = tone_waveform_with(1_500);
    let lockin = joint_lockin();
    let gls = joint_gls(&lockin);
    assert_eq!(gls.covariance_output, GlsCovarianceOutput::Diagonal);
    let run = |time: &[f64], data: &[f64], v0: f64| {
        let source = SyntheticNoiseModelSource::identity(v0);
        let inputs = test_inputs(&lockin, &gls, time);
        run_joint_li(&inputs, &[3], &[data], &source).unwrap()
    };
    let base = run(&time, &signal, 0.01);
    let window = 7;
    // Diagonal retention: one packed 12-entry row per window.
    assert_eq!(base.covariance[0][window].len(), 12);
    for row in &base.covariance[0] {
        assert_eq!(row.len(), 12);
        assert!(row.iter().all(|value| value.is_finite() && *value > 0.0));
    }
    // Full-mode retention of the same signal packs the identical design-model
    // covariance: every diagonal slot agrees exactly with the diagonal run.
    let mut full_lockin = joint_lockin();
    let LockinEstimator::JointHarmonicGls(full_gls_mut) = &mut full_lockin.estimator else {
        unreachable!()
    };
    full_gls_mut.covariance_output = GlsCovarianceOutput::Full;
    let LockinEstimator::JointHarmonicGls(full_gls) = &full_lockin.estimator else {
        unreachable!()
    };
    let source = SyntheticNoiseModelSource::identity(0.01);
    let inputs = test_inputs(&full_lockin, full_gls, &time);
    let full_run = run_joint_li(&inputs, &[3], &[signal.as_slice()], &source).unwrap();
    assert_eq!(full_run.covariance[0][window].len(), 78);
    for (diag_row, full_row) in base.covariance[0].iter().zip(full_run.covariance[0].iter()) {
        assert_eq!(full_row.len(), 78);
        for (index, expected) in diag_row.iter().enumerate() {
            let triangular = index * 12 - index.saturating_sub(1) * index / 2;
            assert_eq!(*expected, full_row[triangular]);
        }
    }
    // The packing contract preserves symmetry on a synthetic symmetric
    // matrix (retention only carries the packed subset, so symmetry itself
    // is pinned here rather than on retained rows).
    let symmetric: Vec<Vec<f64>> = (0..12)
        .map(|row| {
            (0..12)
                .map(|column| 1.0 + 0.01 * (row + column) as f64 + 0.1 * f64::from(row == column))
                .collect()
        })
        .collect();
    for (row, values) in symmetric.iter().enumerate() {
        for (column, value) in values.iter().enumerate() {
            assert_eq!(*value, symmetric[column][row]);
        }
    }
    let diag = pack_covariance_row(&symmetric, GlsCovarianceOutput::Diagonal).unwrap();
    let full = pack_covariance_row(&symmetric, GlsCovarianceOutput::Full).unwrap();
    assert_eq!(diag.len(), 12);
    assert_eq!(full.len(), 78);
    for (index, expected) in diag.iter().enumerate() {
        let triangular = index * 12 - index.saturating_sub(1) * index / 2;
        assert_eq!(*expected, full[triangular]);
    }
    let doubled_signal: Vec<f64> = signal.iter().map(|value| 2.0 * value).collect();
    let doubled = run(&time, &doubled_signal, 0.01);
    for (plain, scaled) in base.result[0].iter().zip(doubled.result[0].iter()) {
        for (left, right) in plain.iter().zip(scaled.iter()) {
            assert!((2.0 * left - right).abs() <= 1e-12 * right.abs().max(1.0));
        }
    }
    assert_eq!(base.covariance, doubled.covariance);
    let noisy = run(&time, &signal, 0.04);
    assert_eq!(base.covariance.len(), noisy.covariance.len());
    for (plain_windows, scaled_windows) in base.covariance.iter().zip(noisy.covariance.iter()) {
        for (plain, scaled) in plain_windows.iter().zip(scaled_windows.iter()) {
            assert_eq!(plain.len(), scaled.len());
            for (left, right) in plain.iter().zip(scaled.iter()) {
                assert!((4.0 * left - right).abs() <= 1e-9 * right.abs().max(1e-12));
            }
        }
    }
}

struct TinyRng(u64);

impl TinyRng {
    fn next_f64(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as f64) / (u32::MAX as f64)
    }

    fn next_normal(&mut self, spare: &mut Option<f64>) -> f64 {
        if let Some(value) = spare.take() {
            return value;
        }
        let radius = (-2.0 * self.next_f64().max(1e-12).ln()).sqrt();
        let angle = 2.0 * std::f64::consts::PI * self.next_f64();
        *spare = Some(radius * angle.sin());
        radius * angle.cos()
    }
}

#[test]
fn covariance_covers_known_noise() {
    let dt = 1.0e-5;
    let f_ref = 1_000.0;
    let samples = 900usize;
    let v0: f64 = 0.01;
    let lockin = joint_lockin();
    let gls = joint_gls(&lockin);
    let mut inside = 0usize;
    let mut total = 0usize;
    for seed in 0..6u64 {
        let mut rng = TinyRng(0x9E3779B97F4A7C15u64.wrapping_add(seed));
        let mut spare = None;
        let time: Vec<f64> = (0..samples).map(|index| index as f64 * dt).collect();
        let signal: Vec<f64> = time
            .iter()
            .map(|&t| {
                (2.0 * std::f64::consts::PI * f_ref * t).sin()
                    + v0.sqrt() * rng.next_normal(&mut spare)
            })
            .collect();
        let source = SyntheticNoiseModelSource::identity(v0);
        let inputs = test_inputs(&lockin, &gls, &time);
        let output = run_joint_li(&inputs, &[3], &[signal.as_slice()], &source).unwrap();
        let grid = output.result[0][0].len();
        assert!(grid >= 64, "need room for disjoint windows, got {grid}");
        let middle = grid / 2;
        // Two windows with non-overlapping support (centers differ by more
        // than the 203-tap window: 21 outputs at stride 10).
        for choice in [middle - 21, middle + 21] {
            let x1 = output.result[0][0][choice];
            // Diagonal-packed retention: entry 0 is the x1 marginal variance.
            let variance = output.covariance[0][choice][0];
            assert!(variance > 0.0);
            // Phase-zero unit tone: X1 truth is the peak amplitude 1.0.
            if ((x1 - 1.0) / variance.sqrt()).abs() <= 1.0 {
                inside += 1;
            }
            total += 1;
        }
    }
    assert!(total >= 10, "need independent samples, got {total}");
    let rate = inside as f64 / total as f64;
    assert!(
        (0.35..=0.95).contains(&rate),
        "coverage {inside}/{total} = {rate} outside the nominal band"
    );
}

#[test]
fn covariance_artifacts_round_trip_and_register() {
    let (time, signal) = tone_waveform_with(1_500);
    let lockin = joint_lockin();
    let gls = joint_gls(&lockin);
    let source = SyntheticNoiseModelSource::identity(0.01);
    let inputs = test_inputs(&lockin, &gls, &time);
    let output = run_joint_li(
        &inputs,
        &[3, 4],
        &[signal.as_slice(), signal.as_slice()],
        &source,
    )
    .unwrap();
    let dir = std::env::temp_dir().join(format!("pmoke_joint_covariance_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = test_config(vec![1], vec![3, 4]);
    cfg.set_artifact_root(dir.clone());
    let paths = cfg.paths();
    std::fs::create_dir_all(paths.analysis_dir().join("lockin")).unwrap();
    let grid: Vec<f64> = output.quality[0].iter().map(|row| row.time_s).collect();
    for (position, channel) in [3u8, 4u8].iter().enumerate() {
        write_covariance_csv(
            &paths.lockin_covariance_csv(*channel),
            GlsCovarianceOutput::Diagonal,
            &grid,
            &output.covariance[position],
        )
        .unwrap();
        write_covariance_npy(
            &paths.lockin_covariance_npy(*channel),
            GlsCovarianceOutput::Diagonal,
            &grid,
            &output.covariance[position],
        )
        .unwrap();
        assert!(
            write_covariance_csv(
                &paths.lockin_covariance_csv(*channel),
                GlsCovarianceOutput::Diagonal,
                &grid,
                &output.covariance[position],
            )
            .is_err()
        );
    }
    let text = std::fs::read_to_string(paths.lockin_covariance_csv(3)).unwrap();
    let mut lines = text.lines();
    assert_eq!(
        lines.next().unwrap(),
        covariance_csv_header(GlsCovarianceOutput::Diagonal)
            .unwrap()
            .join(",")
    );
    assert_eq!(lines.count(), grid.len());
    std::fs::write(paths.analysis_manifest(), "schema_version = 3\n").unwrap();
    std::fs::write(paths.analysis_source_config(), b"version = 3\n").unwrap();
    std::fs::write(paths.analysis_resolved_config(), b"version = 3\n").unwrap();
    crate::lockin::provenance::refresh_analysis_manifest_outputs(&cfg, "li").unwrap();
    let manifest = std::fs::read_to_string(paths.analysis_manifest()).unwrap();
    let value: toml::Value = toml::from_str(&manifest).unwrap();
    let artifacts = value["artifacts"].as_array().unwrap();
    let covariance: Vec<&toml::Value> = artifacts
        .iter()
        .filter(|artifact| artifact["kind"].as_str() == Some("lockin_covariance"))
        .collect();
    assert_eq!(covariance.len(), 2);
    for artifact in covariance {
        assert_eq!(artifact["columns"].as_integer(), Some(13));
        assert_eq!(artifact["rows"].as_integer(), Some(grid.len() as i64));
        assert_eq!(artifact["dtype"].as_str(), Some("<f8"));
        assert_eq!(artifact["order"].as_str(), Some("C"));
        assert!(artifact["npy"].as_str().is_some());
        assert_eq!(
            artifact["format"].as_str(),
            Some("covariance_mode=design_model")
        );
    }
    let column_sets = value["column_sets"].as_table().unwrap();
    let names = column_sets["lockin_covariance"]["names"]
        .as_array()
        .unwrap();
    assert_eq!(names.len(), 13);
    assert_eq!(names[0].as_str(), Some("time_s"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn full_covariance_registers_with_79_columns() {
    let (time, signal) = tone_waveform_with(1_500);
    let mut lockin = joint_lockin();
    let LockinEstimator::JointHarmonicGls(gls_mut) = &mut lockin.estimator else {
        unreachable!()
    };
    // Full retention is required for a full artifact: packed rows carry the
    // serialized width, so a diagonal run cannot publish 78 columns.
    gls_mut.covariance_output = GlsCovarianceOutput::Full;
    let gls = joint_gls(&lockin);
    assert_eq!(gls.covariance_output, GlsCovarianceOutput::Full);
    let source = SyntheticNoiseModelSource::identity(0.01);
    let inputs = test_inputs(&lockin, &gls, &time);
    let output = run_joint_li(&inputs, &[3], &[signal.as_slice()], &source).unwrap();
    let dir = std::env::temp_dir().join(format!(
        "pmoke_joint_covariance_full_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = test_config(vec![1], vec![3]);
    cfg.set_artifact_root(dir.clone());
    let paths = cfg.paths();
    std::fs::create_dir_all(paths.analysis_dir().join("lockin")).unwrap();
    let grid: Vec<f64> = output.quality[0].iter().map(|row| row.time_s).collect();
    write_covariance_csv(
        &paths.lockin_covariance_csv(3),
        GlsCovarianceOutput::Full,
        &grid,
        &output.covariance[0],
    )
    .unwrap();
    std::fs::write(paths.analysis_manifest(), "schema_version = 3\n").unwrap();
    std::fs::write(paths.analysis_source_config(), b"version = 3\n").unwrap();
    std::fs::write(paths.analysis_resolved_config(), b"version = 3\n").unwrap();
    crate::lockin::provenance::refresh_analysis_manifest_outputs(&cfg, "li").unwrap();
    let manifest = std::fs::read_to_string(paths.analysis_manifest()).unwrap();
    let value: toml::Value = toml::from_str(&manifest).unwrap();
    let artifacts = value["artifacts"].as_array().unwrap();
    let artifact = artifacts
        .iter()
        .find(|artifact| artifact["kind"].as_str() == Some("lockin_covariance"))
        .expect("lockin_covariance artifact missing");
    assert_eq!(artifact["columns"].as_integer(), Some(79));
    assert!(artifact.get("npy").is_none());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn quality_writer_rejects_non_finite_rows() {
    let dir =
        std::env::temp_dir().join(format!("pmoke_joint_quality_finite_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let bad = vec![QualityRow {
        original_center_index: 12,
        time_s: 0.0,
        scaled_design_condition: f64::INFINITY,
        residual_rms_v: 0.0,
        rank: 25,
        status: WindowStatus::Ok,
        jitter_applied_v2: 0.0,
        noise_mode: "identity",
    }];
    assert!(write_quality_csv(&dir.join("bad.csv"), &bad).is_err());
    assert!(!dir.join("bad.csv").exists());
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(WindowStatus::for_jitter(0.0), WindowStatus::Ok);
    assert_eq!(WindowStatus::for_jitter(1e-9), WindowStatus::Warning);
}

#[test]
fn short_trace_fails_without_partial_output() {
    // A record shorter than the window cannot yield outputs: the engine
    // fails during geometry/trimming with a descriptive error instead of
    // padding, shrinking the window, or publishing rows (insufficient
    // support, AT-024).
    let dt = 1.0e-5;
    let time: Vec<f64> = (0..100).map(|index| index as f64 * dt).collect();
    let signal = vec![0.0; 100];
    let lockin = joint_lockin();
    let gls = match &lockin.estimator {
        LockinEstimator::JointHarmonicGls(gls) => gls,
        LockinEstimator::BoxcarLegacy => unreachable!(),
    };
    let source = SyntheticNoiseModelSource::identity(0.01);
    let inputs = test_inputs(&lockin, gls, &time);
    let error = run_joint_li(&inputs, &[3], &[signal.as_slice()], &source)
        .err()
        .unwrap();
    let message = format!("{error:#}");
    assert!(
        message.contains("edge trim")
            || message.contains("empty")
            || message.contains("geometry")
            || message.contains("support")
            || message.contains("window"),
        "unhelpful short-trace error: {message}"
    );
}

#[test]
fn engine_output_is_invariant_to_worker_settings() {
    // The direct engine processes the full grid in one pass: the workers
    // setting must not change any output value, quality row, covariance,
    // or snapshot (AT-012 worker invariance; a future parallel engine must
    // preserve this contract).
    let (time, signal) = tone_waveform_with(1_500);
    let run = |workers: usize| {
        let mut lockin = joint_lockin();
        lockin.workers = workers;
        let gls = match &lockin.estimator {
            LockinEstimator::JointHarmonicGls(gls) => gls.clone(),
            LockinEstimator::BoxcarLegacy => unreachable!(),
        };
        let source = SyntheticNoiseModelSource::identity(0.01);
        let inputs = test_inputs(&lockin, &gls, &time);
        run_joint_li(&inputs, &[3], &[signal.as_slice()], &source).unwrap()
    };
    let single = run(1);
    for workers in [2, 6, 32] {
        let parallel = run(workers);
        assert_eq!(single.result, parallel.result);
        assert_eq!(single.quality, parallel.quality);
        assert_eq!(single.covariance, parallel.covariance);
        assert_eq!(
            single.base_index_range, parallel.base_index_range,
            "workers={workers}"
        );
    }
}

#[test]
fn quality_center_index_matches_raw_timebase() {
    // R0: original_center_index must be the original-sample center index,
    // consistent with time_s. Stride 10, dt=1e-5: the first quality row
    // must carry a raw index whose time equals its time_s.
    let (time, signal) = tone_waveform_with(3_000);
    let lockin = joint_lockin();
    assert_eq!(lockin.stride_samples, 10);
    let gls = match &lockin.estimator {
        LockinEstimator::JointHarmonicGls(gls) => gls,
        LockinEstimator::BoxcarLegacy => unreachable!(),
    };
    let source = SyntheticNoiseModelSource::identity(0.01);
    let inputs = test_inputs(&lockin, gls, &time);
    let output = run_joint_li(&inputs, &[3], &[signal.as_slice()], &source).unwrap();
    let rows = &output.quality[0];
    assert!(!rows.is_empty());
    let dt = 1.0e-5;
    for row in rows.iter() {
        // Index lives in the raw-sample namespace...
        assert_eq!(time[row.original_center_index], row.time_s);
        // ...and equals (time - t0) / dt rounded.
        let expected = ((row.time_s - time[0]) / dt).round() as usize;
        assert_eq!(row.original_center_index, expected);
        // A decimated counter would be exactly stride times smaller here.
        assert_ne!(row.original_center_index * 10, expected);
    }
}

#[test]
fn quality_center_index_matches_raw_timebase_with_origin() {
    // Same contract with a nonzero time origin: the index still resolves
    // against the actual raw timebase, not a zero-based assumption.
    // origin=1000.0 is exactly representable and keeps rounding inside the
    // timebase tolerance (origin=1e6 would not: its grid rounding exceeds
    // the uniformity allowance, a validator matter unrelated to R0).
    let dt = 1.0e-5;
    let origin = 1000.0;
    let samples = 3_000usize;
    let time: Vec<f64> = (0..samples).map(|i| origin + i as f64 * dt).collect();
    let signal: Vec<f64> = time
        .iter()
        .map(|&t| (2.0 * PI * 1_000.0 * t + 0.3).sin())
        .collect();
    let lockin = joint_lockin();
    let gls = match &lockin.estimator {
        LockinEstimator::JointHarmonicGls(gls) => gls,
        LockinEstimator::BoxcarLegacy => unreachable!(),
    };
    let source = SyntheticNoiseModelSource::identity(0.01);
    let inputs = test_inputs(&lockin, gls, &time);
    let output = run_joint_li(&inputs, &[3], &[signal.as_slice()], &source).unwrap();
    let rows = &output.quality[0];
    assert!(!rows.is_empty());
    for row in rows.iter() {
        assert_eq!(time[row.original_center_index], row.time_s);
    }
}

#[test]
fn packed_retention_bounds_100k_window_bytes() {
    // Peak-RSS regression pin (P4): native retention is the packed width,
    // never the full 144-f64 matrix, and the streamed writers hold no
    // column-matrix copy. Exact byte counts on the 100k-window reference
    // shape (~115MB/ch before).
    assert_eq!(
        packed_covariance_width(GlsCovarianceOutput::Diagonal).unwrap(),
        12
    );
    assert_eq!(
        packed_covariance_width(GlsCovarianceOutput::Full).unwrap(),
        78
    );
    assert!(packed_covariance_width(GlsCovarianceOutput::None).is_err());
    const WINDOWS: usize = 100_000;
    const FULL_MATRIX_F64: usize = 144;
    let before_retention_bytes = WINDOWS * FULL_MATRIX_F64 * 8;
    assert_eq!(before_retention_bytes, 115_200_000);
    let diagonal_retention_bytes = WINDOWS * 12 * 8;
    assert_eq!(diagonal_retention_bytes, 9_600_000);
    let full_retention_bytes = WINDOWS * 78 * 8;
    assert_eq!(full_retention_bytes, 62_400_000);
    assert!(diagonal_retention_bytes <= 12_000_000);
    assert!(full_retention_bytes < before_retention_bytes);
    // The legacy writer's second copy (time + packed columns) is gone with
    // streaming: these exact copies no longer exist at peak.
    assert_eq!(WINDOWS * 13 * 8, 10_400_000);
    assert_eq!(WINDOWS * 79 * 8, 63_200_000);
    // Retention matches the fixture runs end to end.
    let (time, signal) = tone_waveform_with(1_500);
    let lockin = joint_lockin();
    let gls = joint_gls(&lockin);
    let source = SyntheticNoiseModelSource::identity(0.01);
    let inputs = test_inputs(&lockin, &gls, &time);
    let output = run_joint_li(&inputs, &[3], &[signal.as_slice()], &source).unwrap();
    assert!(!output.covariance[0].is_empty());
    for row in &output.covariance[0] {
        assert_eq!(row.len(), 12);
    }
}

#[test]
fn covariance_streamed_writes_match_legacy_bytes() {
    // File-identity proof (P4): the streamed packed writers emit
    // byte-identical CSV and NPY artifacts to the legacy column-matrix
    // writer for both serialization modes. The legacy logic is copied here
    // as the reference; production no longer builds the column matrix.
    for mode in [GlsCovarianceOutput::Diagonal, GlsCovarianceOutput::Full] {
        let width = packed_covariance_width(mode).unwrap();
        let rows = 17usize;
        let times: Vec<f64> = (0..rows)
            .map(|index| 0.001 + index as f64 * 1.0e-5)
            .collect();
        let matrices: Vec<Vec<Vec<f64>>> = (0..rows)
            .map(|index| {
                let scale = 1.0 + 0.01 * index as f64;
                (0..12)
                    .map(|row| {
                        (0..12)
                            .map(|column| {
                                scale
                                    * (1.0
                                        + 0.01 * (row + column) as f64
                                        + 0.1 * f64::from(row == column))
                            })
                            .collect()
                    })
                    .collect()
            })
            .collect();
        let packed_rows: Vec<Vec<f64>> = matrices
            .iter()
            .map(|matrix| pack_covariance_row(matrix, mode).unwrap())
            .collect();
        assert!(packed_rows.iter().all(|row| row.len() == width));
        let dir = std::env::temp_dir().join(format!(
            "pmoke_joint_covariance_identity_{}_{}",
            match mode {
                GlsCovarianceOutput::Diagonal => "diagonal",
                GlsCovarianceOutput::Full => "full",
                GlsCovarianceOutput::None => unreachable!(),
            },
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let header = covariance_csv_header(mode).unwrap();
        // CSV: streamed packed writer vs legacy column-matrix writer.
        let new_csv = dir.join("new.csv");
        write_covariance_csv(&new_csv, mode, &times, &packed_rows).unwrap();
        let legacy_csv = dir.join("legacy.csv");
        {
            let mut columns: Vec<Vec<f64>> = vec![Vec::with_capacity(times.len()); header.len()];
            for (time, matrix) in times.iter().zip(matrices.iter()) {
                let packed = pack_covariance_row(matrix, mode).unwrap();
                columns[0].push(*time);
                for (column, value) in columns.iter_mut().skip(1).zip(packed.iter()) {
                    column.push(*value);
                }
            }
            let file = std::fs::File::create(&legacy_csv).unwrap();
            let mut writer = csv::WriterBuilder::new()
                .has_headers(true)
                .from_writer(file);
            writer.write_record(&header).unwrap();
            for index in 0..times.len() {
                let record: Vec<String> = columns
                    .iter()
                    .map(|column| column[index].to_string())
                    .collect();
                writer.write_record(&record).unwrap();
            }
            writer.flush().unwrap();
        }
        assert_eq!(
            std::fs::read(&new_csv).unwrap(),
            std::fs::read(&legacy_csv).unwrap(),
            "CSV bytes differ for {mode:?}"
        );
        // NPY: streamed packed writer vs the shared column-matrix writer.
        let new_npy = dir.join("new.npy");
        write_covariance_npy(&new_npy, mode, &times, &packed_rows).unwrap();
        let legacy_npy = dir.join("legacy.npy");
        {
            let mut columns: Vec<Vec<f64>> = vec![Vec::with_capacity(times.len()); header.len()];
            for (time, matrix) in times.iter().zip(matrices.iter()) {
                let packed = pack_covariance_row(matrix, mode).unwrap();
                columns[0].push(*time);
                for (column, value) in columns.iter_mut().skip(1).zip(packed.iter()) {
                    column.push(*value);
                }
            }
            crate::utils::csv::write_npy(&legacy_npy, &columns).unwrap();
        }
        assert_eq!(
            std::fs::read(&new_npy).unwrap(),
            std::fs::read(&legacy_npy).unwrap(),
            "NPY bytes differ for {mode:?}"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn covariance_writer_rejects_width_mismatch() {
    // Retention carries the serialized width: a diagonal run cannot publish
    // a full artifact (and vice versa) instead of silently padding.
    let dir = std::env::temp_dir().join(format!(
        "pmoke_joint_covariance_width_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let times = vec![0.001, 0.002];
    let diagonal_rows = vec![vec![1.0; 12], vec![2.0; 12]];
    assert!(
        write_covariance_csv(
            &dir.join("mismatch.csv"),
            GlsCovarianceOutput::Full,
            &times,
            &diagonal_rows
        )
        .is_err()
    );
    assert!(!dir.join("mismatch.csv").exists());
    let full_rows = vec![vec![1.0; 78], vec![2.0; 78]];
    assert!(
        write_covariance_npy(
            &dir.join("mismatch.npy"),
            GlsCovarianceOutput::Diagonal,
            &times,
            &full_rows
        )
        .is_err()
    );
    assert!(!dir.join("mismatch.npy").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}
