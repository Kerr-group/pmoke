//! Hoisted joint-GLS adequacy gate bound (LI 10s target): the per-window
//! TOL-07 exact eigendecomposition is replaced by an O(N) conservative
//! bound (`cond(S C S) <= cond(S)^2 * cond(C)`) with `cond(C)` measured
//! once per plan, falling back to the exact gate whenever the bound cannot
//! certify acceptance. Passing runs are bit-identical; the failure surface
//! is unchanged.
//!
//! Fixtures use the recorded-scale reference shape (N=257, fit 1..12,
//! phase_correlated AR(1) kernel, 8 variance bins); no private data.

use nalgebra::DMatrix;
use pmoke_analysis_core::{
    CorrelationKernel, HarmonicSignalModel, JointHarmonicSettings, JointSolverTolerances,
    NoiseMode, NoiseModel, PreparedNoisePlan, estimate_joint, estimate_joint_with_plan,
};
use std::f64::consts::TAU;
use std::time::Instant;

const SAMPLES: usize = 257;
const DT: f64 = 1.0e-6;
const F_REF: f64 = 10_000.0;
const SAMPLE_RATE: f64 = 1.0e6;

fn signal_model() -> HarmonicSignalModel {
    HarmonicSignalModel {
        fit_harmonics: (1..=12).collect(),
        output_harmonics: (1..=6).collect(),
        envelope_degree: 0,
    }
}

fn phase_correlated_noise() -> NoiseModel {
    NoiseModel {
        mode: NoiseMode::PhaseCorrelated,
        reference_variance_v2: 1.0,
        variance_bins: Some((0..8).map(|bin| 1.0 + 0.05 * bin as f64).collect()),
        correlation: Some(CorrelationKernel {
            lags: (0..64).map(|lag| 0.9_f64.powi(lag)).collect(),
            lag_step_s: DT,
        }),
    }
}

fn stationary_noise() -> NoiseModel {
    NoiseModel {
        mode: NoiseMode::StationaryCorrelated,
        reference_variance_v2: 4.0,
        variance_bins: None,
        correlation: Some(CorrelationKernel {
            lags: (0..64).map(|lag| 0.9_f64.powi(lag)).collect(),
            lag_step_s: DT,
        }),
    }
}

/// Deterministic two-tone window with an absolute-time shift per window, so
/// consecutive windows exercise genuinely different scale matrices `S`.
fn window(shift: f64) -> (Vec<f64>, Vec<f64>) {
    let times: Vec<f64> = (0..SAMPLES)
        .map(|index| shift + index as f64 * DT)
        .collect();
    let signal: Vec<f64> = times
        .iter()
        .map(|&time| {
            1.5 * (TAU * F_REF * time + 0.3).sin() + 0.4 * (TAU * 2.0 * F_REF * time).cos()
        })
        .collect();
    (times, signal)
}

fn assert_bit_identical(
    direct: &pmoke_analysis_core::JointEstimate,
    planned: &pmoke_analysis_core::JointEstimate,
    context: &str,
) {
    assert_eq!(direct.beta, planned.beta, "{context} beta");
    assert_eq!(direct.xy, planned.xy, "{context} xy");
    assert_eq!(
        direct.covariance_beta, planned.covariance_beta,
        "{context} covariance_beta"
    );
    assert_eq!(
        direct.covariance_xy, planned.covariance_xy,
        "{context} covariance_xy"
    );
    assert_eq!(direct.rank, planned.rank, "{context} rank");
    assert_eq!(direct.condition, planned.condition, "{context} condition");
    assert_eq!(
        direct.residual_rms, planned.residual_rms,
        "{context} residual_rms"
    );
    assert_eq!(
        direct.jitter_applied_v2, planned.jitter_applied_v2,
        "{context} jitter"
    );
}

/// AT: the hoisted bound path is bit-identical to the direct path on the
/// reference shape, in both correlated modes, across windows with
/// genuinely different scale matrices.
#[test]
fn hoisted_bound_matches_direct_bit_identical() {
    let model = signal_model();
    let tolerances = JointSolverTolerances::default();
    for noise in [phase_correlated_noise(), stationary_noise()] {
        let plan = PreparedNoisePlan::prepare(&noise, SAMPLES, tolerances).unwrap();
        let hoisted = plan.correlation_condition().unwrap();
        assert!(
            hoisted.is_finite() && hoisted > 1.0,
            "{:?} hoisted cond(C) = {hoisted}",
            noise.mode
        );
        for index in 0..3 {
            let (times, signal) = window(0.37 * DT * index as f64);
            let phase = 0.21 * index as f64;
            let direct = estimate_joint(
                &times,
                &signal,
                F_REF,
                phase,
                SAMPLE_RATE,
                &JointHarmonicSettings {
                    model: model.clone(),
                    noise: noise.clone(),
                    tolerances,
                },
            )
            .unwrap();
            let planned = estimate_joint_with_plan(
                &times,
                &signal,
                F_REF,
                phase,
                SAMPLE_RATE,
                &model,
                tolerances,
                &plan,
            )
            .unwrap();
            assert_bit_identical(
                &direct,
                &planned,
                &format!("{:?} window {index}", noise.mode),
            );
        }
    }
}

/// Exact realized condition of `R = S C S` with the gate's own method
/// (explicit realization + symmetric eigendecomposition).
fn exact_realized_condition(toeplitz: &DMatrix<f64>, sqrt_variance: &[f64]) -> f64 {
    let rows = sqrt_variance.len();
    let mut realized = DMatrix::zeros(rows, rows);
    for row in 0..rows {
        for column in 0..rows {
            realized[(row, column)] =
                sqrt_variance[row] * toeplitz[(row, column)] * sqrt_variance[column];
        }
    }
    let eigenvalues = realized.symmetric_eigen().eigenvalues;
    let mut smallest = f64::INFINITY;
    let mut largest = f64::NEG_INFINITY;
    for value in eigenvalues.iter() {
        smallest = smallest.min(*value);
        largest = largest.max(*value);
    }
    largest / smallest
}

/// AT: bound fallback preserves the exact verdict. A cap between the exact
/// condition and the conservative bound still passes identically (the bound
/// cannot certify, so the exact gate decides); a cap below the exact
/// condition rejects with the same code on both paths.
#[test]
fn hoisted_bound_fallback_preserves_exact_verdict() {
    let model = signal_model();
    let noise = phase_correlated_noise();
    let tolerances = JointSolverTolerances::default();
    let (times, signal) = window(0.0);
    let plan = PreparedNoisePlan::prepare(&noise, SAMPLES, tolerances).unwrap();
    let cond_c = plan.correlation_condition().unwrap();

    // Per-window scales through the same interpolation the estimator uses.
    let bins = noise.variance_bins.as_ref().unwrap();
    let phases: Vec<f64> = times.iter().map(|time| TAU * F_REF * time).collect();
    let variances = pmoke_analysis_core::interpolate_variance(&phases, bins).unwrap();
    let sqrt_variance: Vec<f64> = variances.iter().map(|value| value.sqrt()).collect();
    let (mut smallest, mut largest) = (f64::INFINITY, 0.0_f64);
    for scale in sqrt_variance.iter() {
        smallest = smallest.min(*scale);
        largest = largest.max(*scale);
    }
    let bound = (largest / smallest).powi(2) * cond_c;

    let kernel = noise.correlation.as_ref().unwrap();
    let mut toeplitz = DMatrix::zeros(SAMPLES, SAMPLES);
    for row in 0..SAMPLES {
        for column in 0..SAMPLES {
            let lag = row.abs_diff(column);
            if lag < kernel.lags.len() {
                toeplitz[(row, column)] = kernel.lags[lag];
            }
        }
    }
    let exact = exact_realized_condition(&toeplitz, &sqrt_variance);
    assert!(
        bound >= exact,
        "bound {bound:.6e} must dominate exact {exact:.6e}"
    );
    // The reference fixture must discriminate: bound/exact ~1.17, so a cap
    // at exact * 1.05 sits strictly between them.
    assert!(
        bound > exact * 1.05,
        "fixture does not discriminate: bound {bound:.6e} vs exact {exact:.6e}"
    );

    // Cap between exact and bound: the bound cannot certify, the exact gate
    // passes, and both paths agree bit-identically.
    let between = JointSolverTolerances {
        max_noise_condition: exact * 1.05,
        ..tolerances
    };
    let between_plan = PreparedNoisePlan::prepare(&noise, SAMPLES, between).unwrap();
    let direct = estimate_joint(
        &times,
        &signal,
        F_REF,
        0.0,
        SAMPLE_RATE,
        &JointHarmonicSettings {
            model: model.clone(),
            noise: noise.clone(),
            tolerances: between,
        },
    )
    .unwrap();
    let planned = estimate_joint_with_plan(
        &times,
        &signal,
        F_REF,
        0.0,
        SAMPLE_RATE,
        &model,
        between,
        &between_plan,
    )
    .unwrap();
    assert_bit_identical(&direct, &planned, "between-cap window");

    // Cap below exact: both paths reject with the same TOL-07 code.
    let below = JointSolverTolerances {
        max_noise_condition: exact * 0.5,
        ..tolerances
    };
    let below_plan = PreparedNoisePlan::prepare(&noise, SAMPLES, below).unwrap();
    assert_eq!(
        estimate_joint(
            &times,
            &signal,
            F_REF,
            0.0,
            SAMPLE_RATE,
            &JointHarmonicSettings {
                model: model.clone(),
                noise: noise.clone(),
                tolerances: below,
            },
        )
        .unwrap_err()
        .code(),
        "ill_conditioned_noise_model"
    );
    assert_eq!(
        estimate_joint_with_plan(
            &times,
            &signal,
            F_REF,
            0.0,
            SAMPLE_RATE,
            &model,
            below,
            &below_plan
        )
        .unwrap_err()
        .code(),
        "ill_conditioned_noise_model"
    );
}

/// Perf regression: twelve hoisted windows sharing one plan must finish in
/// less wall time than three direct windows. The old code spends a second
/// O(N^3) eigendecomposition per window on both paths (~4x the work), so
/// this fails before the hoist and passes after (~0.3x) with margin for
/// ~2x runner variance on either side. Self-calibrating: no absolute wall
/// bound, so debug and release profiles both gate the structural cost.
#[test]
fn hoisted_plan_beats_direct_wall() {
    let model = signal_model();
    let noise = phase_correlated_noise();
    let tolerances = JointSolverTolerances::default();
    let settings = JointHarmonicSettings {
        model: model.clone(),
        noise: noise.clone(),
        tolerances,
    };
    let plan = PreparedNoisePlan::prepare(&noise, SAMPLES, tolerances).unwrap();

    // Warmup so the clock measures steady-state estimation, not setup.
    let (times, signal) = window(0.0);
    let _ = estimate_joint(&times, &signal, F_REF, 0.0, SAMPLE_RATE, &settings).unwrap();
    let _ = estimate_joint_with_plan(
        &times,
        &signal,
        F_REF,
        0.0,
        SAMPLE_RATE,
        &model,
        tolerances,
        &plan,
    )
    .unwrap();

    let windows: Vec<(Vec<f64>, Vec<f64>)> = (0..12)
        .map(|index| window(0.37 * DT * index as f64))
        .collect();

    let start = Instant::now();
    for (times, signal) in windows.iter().take(3) {
        let _ = estimate_joint(times, signal, F_REF, 0.0, SAMPLE_RATE, &settings).unwrap();
    }
    let direct_elapsed = start.elapsed();

    let start = Instant::now();
    for (times, signal) in windows.iter() {
        let _ = estimate_joint_with_plan(
            times,
            signal,
            F_REF,
            0.0,
            SAMPLE_RATE,
            &model,
            tolerances,
            &plan,
        )
        .unwrap();
    }
    let planned_elapsed = start.elapsed();

    eprintln!(
        "hoisted perf: direct 3 windows in {direct_elapsed:?}, planned 12 windows in {planned_elapsed:?}"
    );
    assert!(
        planned_elapsed < direct_elapsed,
        "hoisted 12 windows ({planned_elapsed:?}) must beat direct 3 ({direct_elapsed:?})"
    );
}

/// Perf regression: the hoist covers BOTH correlated modes through the
/// shared `whiten_correlated` gate, so twelve stationary planned windows
/// sharing one plan must finish in less wall time than three direct
/// stationary windows. Same self-calibrating structure as
/// `hoisted_plan_beats_direct_wall`: fails before the hoist, passes after
/// with margin for ~2x runner variance on either side.
#[test]
fn stationary_planned_beats_direct_wall() {
    let model = signal_model();
    let noise = stationary_noise();
    let tolerances = JointSolverTolerances::default();
    let settings = JointHarmonicSettings {
        model: model.clone(),
        noise: noise.clone(),
        tolerances,
    };
    let plan = PreparedNoisePlan::prepare(&noise, SAMPLES, tolerances).unwrap();

    // Warmup so the clock measures steady-state estimation, not setup.
    let (times, signal) = window(0.0);
    let _ = estimate_joint(&times, &signal, F_REF, 0.0, SAMPLE_RATE, &settings).unwrap();
    let _ = estimate_joint_with_plan(
        &times,
        &signal,
        F_REF,
        0.0,
        SAMPLE_RATE,
        &model,
        tolerances,
        &plan,
    )
    .unwrap();

    let windows: Vec<(Vec<f64>, Vec<f64>)> = (0..12)
        .map(|index| window(0.37 * DT * index as f64))
        .collect();

    let start = Instant::now();
    for (times, signal) in windows.iter().take(3) {
        let _ = estimate_joint(times, signal, F_REF, 0.0, SAMPLE_RATE, &settings).unwrap();
    }
    let direct_elapsed = start.elapsed();

    let start = Instant::now();
    for (times, signal) in windows.iter() {
        let _ = estimate_joint_with_plan(
            times,
            signal,
            F_REF,
            0.0,
            SAMPLE_RATE,
            &model,
            tolerances,
            &plan,
        )
        .unwrap();
    }
    let planned_elapsed = start.elapsed();

    eprintln!(
        "stationary hoisted perf: direct 3 windows in {direct_elapsed:?}, planned 12 windows in {planned_elapsed:?}"
    );
    assert!(
        planned_elapsed < direct_elapsed,
        "stationary hoisted 12 windows ({planned_elapsed:?}) must beat direct 3 ({direct_elapsed:?})"
    );
}
