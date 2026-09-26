//! Row-reuse (P-B/Q1) evidence: the shift+refill-tail design fill in
//! `estimate_joint_with_scratch` reproduces the direct entry points
//! bit-for-bit on overlapping strided windows, across all noise modes,
//! with gap-refill, key-change, and invalid-input fallbacks.
//!
//! Byte-identity uses exact `==` (not approximate): reuse copies exact
//! row values and recomputes only tail rows with the identical closed-form,
//! so any divergence is a defect. Beta drift is additionally bounded by
//! the 1e-12 acceptance band.

use pmoke_analysis_core::{
    CorrelationKernel, HarmonicSignalModel, JointHarmonicSettings, JointScratch,
    JointSolverTolerances, NoiseMode, NoiseModel, PreparedNoisePlan, estimate_joint,
    estimate_joint_with_scratch,
};

const F_REF: f64 = 1.17e6;
const DT: f64 = 17.8e-9;
const PHASE: f64 = 0.3;
const N: usize = 512;
const STRIDE: usize = 100;
const WINDOWS: usize = 10;

fn model() -> HarmonicSignalModel {
    HarmonicSignalModel {
        fit_harmonics: (1..=12).collect(),
        output_harmonics: (1..=6).collect(),
        envelope_degree: 0,
    }
}

fn identity_noise() -> NoiseModel {
    NoiseModel {
        mode: NoiseMode::Identity,
        reference_variance_v2: 0.01,
        variance_bins: None,
        correlation: None,
    }
}

fn diagonal_noise() -> NoiseModel {
    NoiseModel {
        mode: NoiseMode::PhaseDiagonal,
        reference_variance_v2: 0.01,
        variance_bins: Some(vec![1.0, 1.1, 0.9, 1.2, 1.0, 1.05, 0.95, 1.15]),
        correlation: None,
    }
}

fn correlated_noise(phase_bins: bool) -> NoiseModel {
    NoiseModel {
        mode: if phase_bins {
            NoiseMode::PhaseCorrelated
        } else {
            NoiseMode::StationaryCorrelated
        },
        reference_variance_v2: 0.01,
        variance_bins: phase_bins.then(|| vec![1.0, 1.1, 0.9, 1.2, 1.0, 1.05, 0.95, 1.15]),
        correlation: Some(CorrelationKernel {
            lags: vec![1.0, 0.5],
            lag_step_s: DT,
        }),
    }
}

fn window_fixture(start_sample: usize) -> (Vec<f64>, Vec<f64>) {
    let times: Vec<f64> = (0..N).map(|i| (start_sample + i) as f64 * DT).collect();
    let mut state: u64 = 0x9E3779B97F4A7C15 ^ ((start_sample as u64).wrapping_mul(0x9E3779B1));
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state as f64 / u64::MAX as f64) - 0.5
    };
    let signal: Vec<f64> = times
        .iter()
        .map(|&t| {
            (std::f64::consts::TAU * F_REF * t + 0.3).sin()
                + 0.2 * (std::f64::consts::TAU * 3.0 * F_REF * t).sin()
                + 0.05 * next()
        })
        .collect();
    (times, signal)
}

fn assert_estimates_bit_identical(
    a: &pmoke_analysis_core::JointEstimate,
    b: &pmoke_analysis_core::JointEstimate,
    context: &str,
) {
    assert_eq!(a.beta, b.beta, "{context}: beta differs");
    assert_eq!(a.xy, b.xy, "{context}: xy differs");
    assert_eq!(
        a.covariance_beta, b.covariance_beta,
        "{context}: covariance_beta differs"
    );
    assert_eq!(
        a.covariance_xy, b.covariance_xy,
        "{context}: covariance_xy differs"
    );
    assert_eq!(a.rank, b.rank, "{context}: rank differs");
    assert!(
        a.condition == b.condition,
        "{context}: condition differs ({} vs {})",
        a.condition,
        b.condition
    );
    assert!(
        a.residual_rms == b.residual_rms,
        "{context}: residual_rms differs ({} vs {})",
        a.residual_rms,
        b.residual_rms
    );
    assert!(
        a.jitter_applied_v2 == b.jitter_applied_v2,
        "{context}: jitter differs"
    );
}

fn beta_max_abs(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (x - y).abs())
        .fold(0.0_f64, f64::max)
}

/// Overlapping strided windows (stride 100 vs N 512 = 80.5% rows shared)
/// reproduce the direct path bit-for-bit in every noise mode, with beta
/// drift exactly zero (inside the 1e-12 band).
#[test]
fn row_reuse_overlapping_windows_match_direct_bit_identical() {
    let sample_rate = 1.0 / DT;
    let tolerances = JointSolverTolerances::default();
    let model = model();
    for (mode_name, noise) in [
        ("identity", identity_noise()),
        ("phase_diagonal", diagonal_noise()),
        ("stationary_correlated", correlated_noise(false)),
        ("phase_correlated", correlated_noise(true)),
    ] {
        let settings = JointHarmonicSettings {
            model: model.clone(),
            noise: noise.clone(),
            tolerances,
        };
        let plan = PreparedNoisePlan::prepare(&noise, N, tolerances).unwrap();
        let mut scratch = JointScratch::new();
        let mut worst_drift = 0.0_f64;
        for window in 0..WINDOWS {
            let (times, signal) = window_fixture(window * STRIDE);
            let direct =
                estimate_joint(&times, &signal, F_REF, PHASE, sample_rate, &settings).unwrap();
            let reused = estimate_joint_with_scratch(
                &times,
                &signal,
                F_REF,
                PHASE,
                sample_rate,
                &model,
                tolerances,
                &plan,
                &mut scratch,
            )
            .unwrap();
            assert_estimates_bit_identical(
                &direct,
                &reused,
                &format!("{mode_name} window {window}: direct vs row-reuse"),
            );
            worst_drift = worst_drift.max(beta_max_abs(&direct.beta, &reused.beta));
        }
        assert!(
            worst_drift <= 1e-12,
            "{mode_name}: beta drift {worst_drift:e} exceeds 1e-12 band"
        );
        assert!(
            worst_drift == 0.0,
            "{mode_name}: expected bit-identical drift 0, got {worst_drift:e}"
        );
    }
}

/// Non-consecutive windows (gap refill) still reproduce the direct path:
/// the cache falls back to a full fill with identical values.
#[test]
fn row_reuse_gap_refill_matches_direct_bit_identical() {
    let sample_rate = 1.0 / DT;
    let tolerances = JointSolverTolerances::default();
    let model = model();
    let noise = identity_noise();
    let settings = JointHarmonicSettings {
        model: model.clone(),
        noise: noise.clone(),
        tolerances,
    };
    let plan = PreparedNoisePlan::prepare(&noise, N, tolerances).unwrap();
    let mut scratch = JointScratch::new();
    // Consecutive, then a large gap (>> N, no overlap), then consecutive.
    let starts = [0usize, 100, 200, 5000, 5100, 5200, 0];
    for (i, start) in starts.iter().enumerate() {
        let (times, signal) = window_fixture(*start);
        let direct = estimate_joint(&times, &signal, F_REF, PHASE, sample_rate, &settings).unwrap();
        let reused = estimate_joint_with_scratch(
            &times,
            &signal,
            F_REF,
            PHASE,
            sample_rate,
            &model,
            tolerances,
            &plan,
            &mut scratch,
        )
        .unwrap();
        assert_estimates_bit_identical(
            &direct,
            &reused,
            &format!("gap window {i} (start {start}): direct vs row-reuse"),
        );
    }
}

/// A model/frequency/phase key change forces a full-fill fallback with
/// identical values (never stale reuse).
#[test]
fn row_reuse_key_change_matches_direct_bit_identical() {
    let sample_rate = 1.0 / DT;
    let tolerances = JointSolverTolerances::default();
    let model = model();
    let noise = identity_noise();
    let plan = PreparedNoisePlan::prepare(&noise, N, tolerances).unwrap();
    let mut scratch = JointScratch::new();
    let (times0, signal0) = window_fixture(0);
    let settings = JointHarmonicSettings {
        model: model.clone(),
        noise: noise.clone(),
        tolerances,
    };
    let direct0 = estimate_joint(&times0, &signal0, F_REF, PHASE, sample_rate, &settings).unwrap();
    let reused0 = estimate_joint_with_scratch(
        &times0,
        &signal0,
        F_REF,
        PHASE,
        sample_rate,
        &model,
        tolerances,
        &plan,
        &mut scratch,
    )
    .unwrap();
    assert_estimates_bit_identical(&direct0, &reused0, "baseline");

    // Same times, different phase: must not reuse stale rows.
    let other_phase = PHASE + 0.11;
    let direct1 = estimate_joint(
        &times0,
        &signal0,
        F_REF,
        other_phase,
        sample_rate,
        &settings,
    )
    .unwrap();
    let reused1 = estimate_joint_with_scratch(
        &times0,
        &signal0,
        F_REF,
        other_phase,
        sample_rate,
        &model,
        tolerances,
        &plan,
        &mut scratch,
    )
    .unwrap();
    assert_estimates_bit_identical(&direct1, &reused1, "phase key change");

    // Back to the original phase on the next strided window: still exact.
    let (times1, signal1) = window_fixture(STRIDE);
    let direct2 = estimate_joint(&times1, &signal1, F_REF, PHASE, sample_rate, &settings).unwrap();
    let reused2 = estimate_joint_with_scratch(
        &times1,
        &signal1,
        F_REF,
        PHASE,
        sample_rate,
        &model,
        tolerances,
        &plan,
        &mut scratch,
    )
    .unwrap();
    assert_estimates_bit_identical(&direct2, &reused2, "phase restored");
}

/// Invalid inputs fail with the same code on both paths and do not poison
/// the cache: the next valid window still matches bit-for-bit.
#[test]
fn row_reuse_invalid_input_preserves_cache_bit_identical() {
    let sample_rate = 1.0 / DT;
    let tolerances = JointSolverTolerances::default();
    let model = model();
    let noise = identity_noise();
    let settings = JointHarmonicSettings {
        model: model.clone(),
        noise: noise.clone(),
        tolerances,
    };
    let plan = PreparedNoisePlan::prepare(&noise, N, tolerances).unwrap();
    let mut scratch = JointScratch::new();
    let (times, signal) = window_fixture(0);
    let direct_ok = estimate_joint(&times, &signal, F_REF, PHASE, sample_rate, &settings).unwrap();
    let reused_ok = estimate_joint_with_scratch(
        &times,
        &signal,
        F_REF,
        PHASE,
        sample_rate,
        &model,
        tolerances,
        &plan,
        &mut scratch,
    )
    .unwrap();
    assert_estimates_bit_identical(&direct_ok, &reused_ok, "valid baseline");

    // Non-finite time: both paths reject with the same code.
    let mut bad_times = times.clone();
    bad_times[7] = f64::NAN;
    let direct_err = estimate_joint(&bad_times, &signal, F_REF, PHASE, sample_rate, &settings)
        .expect_err("direct must reject NaN time");
    let reused_err = estimate_joint_with_scratch(
        &bad_times,
        &signal,
        F_REF,
        PHASE,
        sample_rate,
        &model,
        tolerances,
        &plan,
        &mut scratch,
    )
    .expect_err("row-reuse must reject NaN time");
    assert_eq!(
        direct_err.code(),
        reused_err.code(),
        "admission code differs on invalid timebase"
    );

    // Next valid strided window still matches exactly (no poison).
    let (times_next, signal_next) = window_fixture(STRIDE);
    let direct_next = estimate_joint(
        &times_next,
        &signal_next,
        F_REF,
        PHASE,
        sample_rate,
        &settings,
    )
    .unwrap();
    let reused_next = estimate_joint_with_scratch(
        &times_next,
        &signal_next,
        F_REF,
        PHASE,
        sample_rate,
        &model,
        tolerances,
        &plan,
        &mut scratch,
    )
    .unwrap();
    assert_estimates_bit_identical(&direct_next, &reused_next, "post-error window");
}
