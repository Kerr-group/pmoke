//! P1 blocked correlated triangular solve: the layout-blocked kernel
//! (transpose once, solve row-major, transpose back) must reproduce the
//! naive column-outer reference bit-for-bit (exact `assert_eq`, never
//! approximate), preserve every failure code, and keep the
//! phase-correlated whiten stage far inside a generous wall-time bound.
//!
//! Byte-identity uses exact `==`: both paths execute the same arithmetic
//! in the same per-element order, so any divergence is a defect. Timing
//! bounds are deliberately generous (see the release A/B note below) and
//! only trip on a pathological regression, never on machine noise.

use nalgebra::DMatrix;
use pmoke_analysis_core::{
    CorrelationKernel, HarmonicSignalModel, JointSolverTolerances, NoiseMode, NoiseModel,
    PreparedNoisePlan, design_matrix, forward_substitute,
};
use std::time::Instant;

/// Naive column-outer reference: the exact pre-P1 algorithm, kept here as
/// the bit-identity oracle for the blocked kernel.
fn naive_forward_substitute(lower: &DMatrix<f64>, rhs: &DMatrix<f64>) -> DMatrix<f64> {
    let dimension = lower.nrows();
    let mut solution = DMatrix::zeros(dimension, rhs.ncols());
    for column in 0..rhs.ncols() {
        for row in 0..dimension {
            let mut accumulator = rhs[(row, column)];
            for inner in 0..row {
                accumulator -= lower[(row, inner)] * solution[(inner, column)];
            }
            let diagonal = lower[(row, row)];
            assert!(diagonal != 0.0, "oracle needs a clean factor");
            solution[(row, column)] = accumulator / diagonal;
        }
    }
    solution
}

/// Deterministic xorshift64 stream for fixture generation (no RNG crate).
fn xorshift(state: &mut u64) -> f64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state as f64 / u64::MAX as f64) - 0.5
}

/// Deterministic SPD lower factor: unit diagonal with small strict
/// sub-diagonal entries, so the oracle never hits a zero diagonal.
fn lower_fixture(dimension: usize, seed: u64) -> DMatrix<f64> {
    let mut state = seed;
    let mut lower = DMatrix::zeros(dimension, dimension);
    for row in 0..dimension {
        lower[(row, row)] = 1.0 + 0.5 * (xorshift(&mut state) + 0.5).abs();
        for column in 0..row {
            lower[(row, column)] = 0.1 * xorshift(&mut state);
        }
    }
    lower
}

fn rhs_fixture(rows: usize, columns: usize, seed: u64) -> DMatrix<f64> {
    let mut state = seed;
    DMatrix::from_iterator(
        rows,
        columns,
        (0..rows * columns).map(|_| xorshift(&mut state)),
    )
}

/// The blocked solve reproduces the naive reference bit-for-bit across
/// shapes, including the tall N x (P+1) whitening shape and multi-column
/// panels.
#[test]
fn blocked_matches_naive_reference_bit_identical() {
    for (dimension, ncols, seed) in [
        (2, 2, 0x1234_5678_9ABC_DEF1),
        (5, 1, 0x0BAD_F00D_CAFE_1234),
        (17, 7, 0x9E37_79B9_7F4A_7C15),
        (97, 26, 0xDEAD_BEEF_1234_5678),
        (257, 26, 0xCAFE_BABE_8765_4321),
    ] {
        let lower = lower_fixture(dimension, seed);
        let rhs = rhs_fixture(dimension, ncols, !seed);
        let expected = naive_forward_substitute(&lower, &rhs);
        let got = forward_substitute(&lower, &rhs).unwrap();
        assert_eq!(expected, got, "shape {dimension}x{ncols} differs");
    }
}

/// Empty right-hand sides solve trivially with no diagonal checks, exactly
/// like the naive loop nest (whose column loop simply never runs).
#[test]
fn blocked_empty_rhs_matches_naive() {
    let lower = lower_fixture(4, 42);
    let rhs = DMatrix::zeros(4, 0);
    let expected = naive_forward_substitute(&lower, &rhs);
    let got = forward_substitute(&lower, &rhs).unwrap();
    assert_eq!(expected, got, "empty RHS differs");
    assert_eq!((got.nrows(), got.ncols()), (4, 0));
}

/// Failure surface is unchanged: same codes, same first-failing row for
/// zero diagonals, same validation precedence.
#[test]
fn blocked_preserves_failure_surface() {
    let lower = lower_fixture(4, 7);
    let rhs = rhs_fixture(4, 3, 8);
    // Dimension mismatch.
    let bad_rhs = rhs_fixture(3, 3, 9);
    assert_eq!(
        forward_substitute(&lower, &bad_rhs).unwrap_err().code(),
        "dimension_mismatch"
    );
    // Non-finite factor and right-hand side.
    let mut bad_lower = lower.clone();
    bad_lower[(1, 0)] = f64::NAN;
    assert_eq!(
        forward_substitute(&bad_lower, &rhs).unwrap_err().code(),
        "non_finite_input"
    );
    let mut bad_rhs = rhs.clone();
    bad_rhs[(2, 1)] = f64::INFINITY;
    assert_eq!(
        forward_substitute(&lower, &bad_rhs).unwrap_err().code(),
        "non_finite_input"
    );
    // Zero diagonal fails at the first such row with the same message.
    for zero_row in [0, 2] {
        let mut singular = lower.clone();
        singular[(zero_row, zero_row)] = 0.0;
        let error = forward_substitute(&singular, &rhs).unwrap_err();
        assert_eq!(error.code(), "covariance_not_spd");
        assert!(
            error.message().contains(&format!("row {zero_row}")),
            "message must name row {zero_row}: {}",
            error.message()
        );
    }
    // Zero diagonal with no right-hand sides is still a trivial Ok.
    let mut singular = lower.clone();
    singular[(1, 1)] = 0.0;
    let empty = DMatrix::zeros(4, 0);
    assert_eq!(
        forward_substitute(&singular, &empty).unwrap(),
        DMatrix::zeros(4, 0)
    );
}

const SMALL_N: usize = 257;
const DT: f64 = 1.0e-6;
const F_REF: f64 = 10_000.0;
const PHASE: f64 = 0.3;

fn signal_model() -> HarmonicSignalModel {
    HarmonicSignalModel {
        fit_harmonics: (1..=12).collect(),
        output_harmonics: (1..=6).collect(),
        envelope_degree: 0,
    }
}

fn phase_correlated_noise(dt: f64) -> NoiseModel {
    NoiseModel {
        mode: NoiseMode::PhaseCorrelated,
        reference_variance_v2: 1.0,
        variance_bins: Some(vec![1.0, 1.1, 0.9, 1.2, 1.0, 1.05, 0.95, 1.15]),
        correlation: Some(CorrelationKernel {
            lags: (0..64).map(|lag| 0.6_f64.powi(lag)).collect(),
            lag_step_s: dt,
        }),
    }
}

/// Generous-bound wall-time regression gate on the phase-correlated
/// whiten stage (N=257, P=25): two windows must complete far inside 60s.
/// Measured release cost is milliseconds; the bound only trips on a
/// pathological regression (per-window sleep, cubic blowup, or a
/// reintroduced per-column factor streaming).
///
/// Release A/B for the acceptance fixture (N=2003/P=25, same AR(1)
/// kernel family, interleaved naive vs blocked in one process, Apple M4
/// Pro): triangular solve 62.3ms -> 25.3ms per window (2.47x, 59%
/// faster); full whiten pack+solve+split 62.1ms -> 21.0ms (2.95x, 66%
/// faster), bit-identical. The N=2003 timing itself lives behind
/// `n2003_whiten_stage_timing` (`#[ignore]`, release-only) so the default
/// `cargo test` lane stays fast in both debug and release.
#[test]
fn phase_correlated_whiten_stage_wall_time_bound() {
    let sample_rate = 1.0 / DT;
    let tolerances = JointSolverTolerances::default();
    let model = signal_model();
    let noise = phase_correlated_noise(DT);
    let times: Vec<f64> = (0..SMALL_N).map(|i| i as f64 * DT).collect();
    let signal: Vec<f64> = times
        .iter()
        .map(|&t| (std::f64::consts::TAU * F_REF * t + 0.3).sin())
        .collect();
    let design = design_matrix(&times, F_REF, PHASE, &model, sample_rate).unwrap();
    let plan = PreparedNoisePlan::prepare(&noise, SMALL_N, tolerances).unwrap();
    let windows = 2;
    let t0 = Instant::now();
    for _ in 0..windows {
        let system = plan.whiten(&design, &signal, &times, F_REF, PHASE).unwrap();
        assert_eq!(
            (system.design.nrows(), system.design.ncols()),
            (SMALL_N, 25)
        );
        assert!(system.design.iter().all(|v| v.is_finite()));
        std::hint::black_box(system.design[(0, 0)]);
    }
    let elapsed = t0.elapsed();
    println!("phase-correlated whiten N=257: {windows} windows in {elapsed:.2?}");
    assert!(
        elapsed.as_secs() < 60,
        "whiten wall-time regression: {windows} windows took {elapsed:?} (bound 60s)"
    );
}

/// Release-only timing record for the acceptance fixture (N=2003/P=25).
/// Ignored by default; run explicitly with
/// `cargo test --release -p pmoke-analysis-core --test joint_gls_blocked_solve -- --ignored --nocapture`.
/// Prints the whiten-stage wall time for the PR before/after record. The
/// generous bound only guards against pathological blowup.
#[test]
#[ignore]
fn n2003_whiten_stage_timing() {
    const N: usize = 2003;
    const NDT: f64 = 1e-7;
    let sample_rate = 1.0 / NDT;
    let tolerances = JointSolverTolerances::default();
    let model = signal_model();
    let noise = phase_correlated_noise(NDT);
    let times: Vec<f64> = (0..N).map(|i| i as f64 * NDT).collect();
    let signal: Vec<f64> = times
        .iter()
        .map(|&t| (std::f64::consts::TAU * F_REF * t + 0.3).sin())
        .collect();
    let design = design_matrix(&times, F_REF, PHASE, &model, sample_rate).unwrap();
    assert_eq!((design.nrows(), design.ncols()), (N, 25));
    let plan = PreparedNoisePlan::prepare(&noise, N, tolerances).unwrap();
    let system = plan.whiten(&design, &signal, &times, F_REF, PHASE).unwrap();
    std::hint::black_box(system.design[(0, 0)]);
    let reps = 5;
    let mut times_ms = Vec::new();
    for _ in 0..reps {
        let t0 = Instant::now();
        let system = plan.whiten(&design, &signal, &times, F_REF, PHASE).unwrap();
        std::hint::black_box(system.design[(0, 0)]);
        times_ms.push(t0.elapsed().as_secs_f64() * 1e3);
    }
    times_ms.sort_by(|a, b| a.partial_cmp(b).unwrap());
    println!("N=2003/P=25 phase-correlated whiten ms sorted: {times_ms:?}");
    println!("N=2003/P=25 whiten median ms: {:.2}", times_ms[reps / 2]);
    assert!(
        times_ms.iter().sum::<f64>() < 60_000.0,
        "N=2003 whiten wall-time regression: {times_ms:?} (bound 60s total)"
    );
}
