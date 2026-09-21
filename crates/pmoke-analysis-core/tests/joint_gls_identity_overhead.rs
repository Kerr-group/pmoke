//! Identity-path overhead removal evidence for the joint GLS LI loop:
//! the buffer-reusing estimate (`estimate_joint_with_scratch`) and the
//! shared-R solve+covariance must reproduce the direct entry points
//! bit-for-bit on every noise mode, and the reference-shape pipeline must
//! stay far inside a generous wall-time bound.
//!
//! Byte-identity uses exact `==` (not approximate): both paths execute the
//! same arithmetic in the same order, so any divergence is a defect.

use pmoke_analysis_core::{
    CorrelationKernel, HarmonicSignalModel, JointHarmonicSettings, JointScratch,
    JointSolverTolerances, NoiseMode, NoiseModel, PreparedNoisePlan, covariance_from_qr,
    design_matrix, design_matrix_into, estimate_joint, estimate_joint_with_plan,
    estimate_joint_with_scratch, solve_direct, solve_direct_and_covariance,
};
use std::time::Instant;

const N: usize = 97;
const F_REF: f64 = 1.17e6;
const DT: f64 = 17.8e-9;
const PHASE: f64 = 0.3;

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

/// Deterministic tone + fixed-pattern pseudo-noise starting at `t_start`.
fn window_fixture(start: f64) -> (Vec<f64>, Vec<f64>) {
    let times: Vec<f64> = (0..N).map(|i| start + i as f64 * DT).collect();
    let mut state: u64 = 0x9E3779B97F4A7C15 ^ (start.to_bits().wrapping_mul(0x9E3779B1));
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

/// The optimized estimate (one scratch reused across a strided window sweep,
/// shared-R solve) reproduces the direct entry point bit-for-bit in every
/// noise mode. The sweep mimics the LI loop: absolute window times advance
/// by `stride` samples per window, so regressors genuinely differ.
#[test]
fn scratch_and_shared_r_match_direct_bit_identical() {
    let sample_rate = 1.0 / DT;
    let tolerances = JointSolverTolerances::default();
    let model = model();
    let stride = 100;
    // 2 cycles per window at f_ref: the window holds N samples; consecutive
    // windows start `stride` samples later (non-phase-aligned, like prod).
    let windows = 8;
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
        for window in 0..windows {
            let start = window as f64 * stride as f64 * DT;
            let (times, signal) = window_fixture(start);
            let direct =
                estimate_joint(&times, &signal, F_REF, PHASE, sample_rate, &settings).unwrap();
            let planned = estimate_joint_with_plan(
                &times,
                &signal,
                F_REF,
                PHASE,
                sample_rate,
                &model,
                tolerances,
                &plan,
            )
            .unwrap();
            let optimized = estimate_joint_with_scratch(
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
                &planned,
                &format!("{mode_name} window {window}: direct vs planned"),
            );
            assert_estimates_bit_identical(
                &direct,
                &optimized,
                &format!("{mode_name} window {window}: direct vs scratch"),
            );
        }
    }
}

/// The shared-R combined solve+covariance reproduces the sequential pair
/// bit-for-bit (same factor object, same substitutions).
#[test]
fn shared_r_matches_sequential_pair_bit_identical() {
    let sample_rate = 1.0 / DT;
    let tolerances = JointSolverTolerances::default();
    let model = model();
    for (mode_name, noise) in [
        ("identity", identity_noise()),
        ("phase_diagonal", diagonal_noise()),
        ("stationary_correlated", correlated_noise(false)),
        ("phase_correlated", correlated_noise(true)),
    ] {
        let (times, signal) = window_fixture(0.0);
        let design = design_matrix(&times, F_REF, PHASE, &model, sample_rate).unwrap();
        let system =
            pmoke_analysis_core::whiten(&design, &signal, PHASE, F_REF, &times, &noise, tolerances)
                .unwrap();
        let (beta, rms, rank, condition) =
            solve_direct(&system.design, &system.response, tolerances).unwrap();
        let covariance = covariance_from_qr(&system.design, system.variance_scale).unwrap();
        let (beta2, rms2, rank2, condition2, covariance2) = solve_direct_and_covariance(
            &system.design,
            &system.response,
            tolerances,
            system.variance_scale,
        )
        .unwrap();
        assert_eq!(beta.as_slice(), beta2.as_slice(), "{mode_name}: beta");
        assert!(rms == rms2, "{mode_name}: rms");
        assert_eq!(rank, rank2, "{mode_name}: rank");
        assert!(condition == condition2, "{mode_name}: condition");
        assert_eq!(covariance, covariance2, "{mode_name}: covariance");
    }
}

/// The buffer-reusing design fill reproduces the allocating design matrix
/// entry-for-entry, and rejects the same bad inputs with the same codes.
#[test]
fn design_matrix_into_matches_design_matrix() {
    let sample_rate = 1.0 / DT;
    let model = model();
    let (times, _) = window_fixture(1.0e-6);
    let parameters = 1 + 2 * model.fit_harmonics.len();
    let reference = design_matrix(&times, F_REF, PHASE, &model, sample_rate).unwrap();
    let mut reused = nalgebra::DMatrix::<f64>::zeros(N, parameters);
    design_matrix_into(&mut reused, &times, F_REF, PHASE, &model, sample_rate).unwrap();
    assert_eq!(reference, reused, "design entries differ");

    // Reuse across shapes: a 97-sample scratch must refuse a 33-sample
    // window with a shape error.
    let short_times: Vec<f64> = (0..33).map(|i| i as f64 * DT).collect();
    let short_reference = design_matrix(&short_times, F_REF, PHASE, &model, sample_rate).unwrap();
    let rejected = design_matrix_into(&mut reused, &short_times, F_REF, PHASE, &model, sample_rate);
    let error = rejected.expect_err("shape mismatch must fail");
    assert_eq!(error.code(), "dimension_mismatch");
    assert_eq!(short_reference.nrows(), 33);

    // Same Nyquist rejection with the same code on both paths.
    let bad_model = HarmonicSignalModel {
        fit_harmonics: vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16],
        output_harmonics: (1..=6).collect(),
        envelope_degree: 0,
    };
    // k=16 at 1.17MHz against the 28MHz Nyquist is fine; force aliasing
    // with a coarse sample rate instead.
    let coarse_rate = 1.0e6;
    let coarse_times: Vec<f64> = (0..N).map(|i| i as f64 / coarse_rate).collect();
    let alloc_err = design_matrix(&coarse_times, F_REF, PHASE, &bad_model, coarse_rate)
        .expect_err("aliased harmonic must fail");
    let mut wide = nalgebra::DMatrix::<f64>::zeros(N, 1 + 2 * bad_model.fit_harmonics.len());
    let into_err = design_matrix_into(
        &mut wide,
        &coarse_times,
        F_REF,
        PHASE,
        &bad_model,
        coarse_rate,
    )
    .expect_err("aliased harmonic must fail");
    assert_eq!(alloc_err.code(), into_err.code());
    assert_eq!(alloc_err.code(), "aliased_harmonic");
}

/// Generous-bound wall-time regression gate on the reference shape
/// (N=97, p=25, identity): 400 windows must complete far inside 60s.
/// Measured release cost is milliseconds; the bound only trips on a
/// pathological regression (per-window sleep, quadratic blowup, or a
/// reintroduced per-window factorization).
#[test]
fn identity_reference_shape_wall_time_bound() {
    let sample_rate = 1.0 / DT;
    let tolerances = JointSolverTolerances::default();
    let model = model();
    let noise = identity_noise();
    let plan = PreparedNoisePlan::prepare(&noise, N, tolerances).unwrap();
    let mut scratch = JointScratch::new();
    let windows = 400;
    let stride = 100;
    // Prebuild fixtures outside the timed region (steady-state loop only).
    let fixtures: Vec<(Vec<f64>, Vec<f64>)> = (0..windows)
        .map(|window| window_fixture(window as f64 * stride as f64 * DT))
        .collect();
    let t0 = Instant::now();
    for (times, signal) in &fixtures {
        let _ = estimate_joint_with_scratch(
            times,
            signal,
            F_REF,
            PHASE,
            sample_rate,
            &model,
            tolerances,
            &plan,
            &mut scratch,
        )
        .unwrap();
    }
    let elapsed = t0.elapsed();
    let per_window_us = elapsed.as_secs_f64() * 1e6 / windows as f64;
    println!(
        "identity reference shape: {windows} windows in {elapsed:.2?} ({per_window_us:.1}us/window)"
    );
    assert!(
        elapsed.as_secs() < 60,
        "identity wall-time regression: {windows} windows took {elapsed:?} (bound 60s)"
    );
}
