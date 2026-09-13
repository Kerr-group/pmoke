//! WP-2 direct-solver evidence: AT-005 reproduction, AT-006 joint/target
//! equivalence, AT-009 rank and validation gates, AT-010 SciPy differential,
//! and AT-022 core covariance contracts. Correlated noise modes and
//! calibration builders belong to WP-3; they are asserted as explicit errors.

use nalgebra::{DMatrix, DVector};
use pmoke_analysis_core::{
    DEFAULT_MAX_CONDITION, HarmonicSignalModel, JointHarmonicSettings, JointSolverTolerances,
    NoiseMode, NoiseModel, TIMEBASE_RELATIVE_TOLERANCE, covariance_from_qr, design_matrix,
    estimate_joint, interpolate_variance, map_covariance_to_xy, map_to_xy, pack_upper_triangle,
    rotate_phase, rotate_xy_covariance, solve_direct, validate_noise_model, validate_signal_model,
    validate_timebase, whiten,
};
use serde::Deserialize;
use std::collections::BTreeMap;

const FIT_1_TO_12: [usize; 12] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
const OUTPUTS_1_TO_6: [usize; 6] = [1, 2, 3, 4, 5, 6];

fn model_1_to_12() -> HarmonicSignalModel {
    HarmonicSignalModel {
        fit_harmonics: FIT_1_TO_12.to_vec(),
        output_harmonics: OUTPUTS_1_TO_6.to_vec(),
        envelope_degree: 0,
    }
}

fn identity_noise() -> NoiseModel {
    NoiseModel {
        mode: NoiseMode::Identity,
        reference_variance_v2: 1.0,
        variance_bins: None,
    }
}

#[derive(Deserialize)]
struct Conventions {
    cases: Vec<ConventionCase>,
}

#[derive(Deserialize)]
struct ConventionCase {
    name: String,
    f_ref: f64,
    dt: f64,
    t_start: f64,
    samples: usize,
    phase_rad: f64,
    fit_harmonics: Vec<usize>,
    output_harmonics: Vec<usize>,
    signal: Vec<f64>,
    expected_beta: Vec<f64>,
    expected_xy: BTreeMap<String, Xy>,
}

#[derive(Deserialize)]
struct Xy {
    x: f64,
    y: f64,
}

#[derive(Deserialize)]
struct OracleExpected {
    cases: Vec<OracleCase>,
}

#[derive(Deserialize)]
struct OracleCase {
    name: String,
    beta: Vec<f64>,
    xy: BTreeMap<String, Xy>,
    rank: usize,
}

fn convention_case(name: &str) -> ConventionCase {
    let payload: Conventions =
        serde_json::from_str(include_str!("fixtures/joint-gls/conventions.json")).unwrap();
    payload
        .cases
        .into_iter()
        .find(|case| case.name == name)
        .unwrap()
}

fn case_times(case: &ConventionCase) -> Vec<f64> {
    (0..case.samples)
        .map(|index| case.t_start + index as f64 * case.dt)
        .collect()
}

fn settings(model: &HarmonicSignalModel, noise: &NoiseModel) -> JointHarmonicSettings {
    JointHarmonicSettings {
        model: model.clone(),
        noise: noise.clone(),
        tolerances: JointSolverTolerances::default(),
    }
}

fn close(a: f64, b: f64, tolerance: f64, context: &str) {
    assert!(
        (a - b).abs() <= tolerance * (1.0 + b.abs()),
        "{context}: {a} vs {b}"
    );
}

// AT-005: every basis column reproduces (A D = I column-wise) and the XY map
// gives E D = P; mixtures recover the recorded truth.
#[test]
fn basis_columns_reproduce_and_mixtures_match_truth() {
    let case = convention_case("mixed_phases_fractional_t0");
    let model = model_1_to_12();
    let times = case_times(&case);
    let design = design_matrix(&times, case.f_ref, case.phase_rad, &model, 1.0 / case.dt).unwrap();
    let (rows, columns) = (design.nrows(), design.ncols());
    assert_eq!(columns, 25);
    let noise = identity_noise();
    for column in 0..columns {
        let basis: Vec<f64> = (0..rows).map(|row| design[(row, column)]).collect();
        let estimate = estimate_joint(
            &times,
            &basis,
            case.f_ref,
            case.phase_rad,
            1.0 / case.dt,
            &settings(&model, &noise),
        )
        .unwrap();
        for (index, value) in estimate.beta.iter().enumerate() {
            let want = if index == column { 1.0 } else { 0.0 };
            close(
                *value,
                want,
                1.0e-9,
                &format!("basis {column} beta {index}"),
            );
        }
    }
    let estimate = estimate_joint(
        &times,
        &case.signal,
        case.f_ref,
        case.phase_rad,
        1.0 / case.dt,
        &settings(&model, &noise),
    )
    .unwrap();
    for (got, want) in estimate.beta.iter().zip(case.expected_beta.iter()) {
        close(*got, *want, 1.0e-9, "mixture beta");
    }
    for key in case.output_harmonics.iter().map(usize::to_string) {
        let (got, want) = (&estimate.xy, &case.expected_xy[&key]);
        let position = case
            .output_harmonics
            .iter()
            .position(|h| h.to_string() == key)
            .unwrap();
        close(got[2 * position], want.x, 1.0e-9, &format!("X{key}"));
        close(got[2 * position + 1], want.y, 1.0e-9, &format!("Y{key}"));
    }
}

// AT-006: joint-row combination equals the constrained target BLUE for the
// same D and R, including a rotated A3 target; nuisance expansion cannot
// reduce the theoretical target variance.
#[test]
fn joint_row_equals_constrained_target_blue() {
    let case = convention_case("large_even_weak_odd");
    let model = model_1_to_12();
    let times = case_times(&case);
    let design = design_matrix(&times, case.f_ref, case.phase_rad, &model, 1.0 / case.dt).unwrap();
    let gram = design.transpose() * &design;
    let gram_inverse = gram.clone().try_inverse().unwrap();
    let estimate = estimate_joint(
        &times,
        &case.signal,
        case.f_ref,
        case.phase_rad,
        1.0 / case.dt,
        &settings(&model, &identity_noise()),
    )
    .unwrap();
    // Raw X3 target: c selects b3 / 2 in beta order [DC, a1, b1, ...].
    let mut target = vec![0.0; gram.nrows()];
    target[2 + 2 * 2] = 0.5;
    let weights = &design * (&gram_inverse * DMatrix::from_vec(target.len(), 1, target.clone()));
    let joint_x3 = estimate.xy[4];
    let direct: f64 = weights
        .iter()
        .zip(case.signal.iter())
        .map(|(h, y)| h * y)
        .sum();
    close(direct, joint_x3, 1.0e-9, "X3 joint vs target");
    // Rotated A3 target: cos(d) X3 + sin(d) Y3.
    let delta = 0.7_f64;
    let mut rotated = vec![0.0; gram.nrows()];
    rotated[1 + 2 * 2] = delta.sin() / 2.0;
    rotated[2 + 2 * 2] = delta.cos() / 2.0;
    let rotated_weights = &design * (&gram_inverse * DMatrix::from_vec(rotated.len(), 1, rotated));
    let rotated_direct: f64 = rotated_weights
        .iter()
        .zip(case.signal.iter())
        .map(|(h, y)| h * y)
        .sum();
    let (rotated_x, _) = rotate_phase(&estimate.xy[4..5], &estimate.xy[5..6], delta).unwrap();
    close(
        rotated_direct,
        rotated_x[0],
        1.0e-9,
        "rotated A3 joint vs target",
    );
    // Nuisance expansion monotonicity: fitting 1..12 cannot reduce the X3
    // theoretical variance below fitting 1..6 on the same design rows.
    let small: Vec<usize> = (1..=6).collect();
    let small_model = HarmonicSignalModel {
        fit_harmonics: small,
        output_harmonics: OUTPUTS_1_TO_6.to_vec(),
        envelope_degree: 0,
    };
    let small_design = design_matrix(
        &times,
        case.f_ref,
        case.phase_rad,
        &small_model,
        1.0 / case.dt,
    )
    .unwrap();
    let small_gram_inverse = (&small_design.transpose() * &small_design)
        .try_inverse()
        .unwrap();
    let mut small_target = vec![0.0; small_gram_inverse.nrows()];
    small_target[2 + 2 * 2] = 0.5;
    let small_vector = DMatrix::from_vec(small_target.len(), 1, small_target);
    let small_variance = (small_vector.transpose() * &small_gram_inverse * &small_vector)[(0, 0)];
    let big_vector = DMatrix::from_vec(target.len(), 1, target);
    let big_variance = (big_vector.transpose() * &gram_inverse * &big_vector)[(0, 0)];
    assert!(
        big_variance + 1.0e-9 * small_variance >= small_variance,
        "nuisance expansion reduced target variance: {big_variance} < {small_variance}"
    );
}

// AT-006 supplement: scalar response scaling and sign flip propagate exactly.
#[test]
fn response_scaling_propagates_exactly() {
    let case = convention_case("negative_fractional_t0");
    let model = model_1_to_12();
    let times = case_times(&case);
    let noise = identity_noise();
    let base = estimate_joint(
        &times,
        &case.signal,
        case.f_ref,
        case.phase_rad,
        1.0 / case.dt,
        &settings(&model, &noise),
    )
    .unwrap();
    let scaled_signal: Vec<f64> = case.signal.iter().map(|v| -2.5 * v).collect();
    let scaled = estimate_joint(
        &times,
        &scaled_signal,
        case.f_ref,
        case.phase_rad,
        1.0 / case.dt,
        &settings(&model, &noise),
    )
    .unwrap();
    for (got, want) in scaled.beta.iter().zip(base.beta.iter()) {
        close(*got, -2.5 * want, 1.0e-12, "scaled beta");
    }
}

// AT-009: rank, alias, conditioning, and validation gates.
#[test]
fn validation_and_rank_gates_reject_bad_models() {
    let good = model_1_to_12();
    validate_signal_model(&good, 10e6).unwrap();
    let bad_models = [
        HarmonicSignalModel {
            fit_harmonics: vec![1, 1, 2],
            output_harmonics: vec![1],
            envelope_degree: 0,
        },
        HarmonicSignalModel {
            fit_harmonics: vec![3, 1, 2],
            output_harmonics: vec![1],
            envelope_degree: 0,
        },
        HarmonicSignalModel {
            fit_harmonics: vec![0, 1],
            output_harmonics: vec![1],
            envelope_degree: 0,
        },
        HarmonicSignalModel {
            fit_harmonics: vec![1, 2],
            output_harmonics: vec![3],
            envelope_degree: 0,
        },
        HarmonicSignalModel {
            fit_harmonics: vec![1, 2],
            output_harmonics: vec![1, 2, 3],
            envelope_degree: 0,
        },
        HarmonicSignalModel {
            fit_harmonics: vec![],
            output_harmonics: vec![],
            envelope_degree: 0,
        },
        HarmonicSignalModel {
            fit_harmonics: (1..=13).collect(),
            output_harmonics: OUTPUTS_1_TO_6.to_vec(),
            envelope_degree: 0,
        },
        HarmonicSignalModel {
            fit_harmonics: vec![1, 2],
            output_harmonics: vec![1, 2],
            envelope_degree: 1,
        },
    ];
    for model in &bad_models {
        assert!(
            validate_signal_model(model, 10e6).is_err(),
            "accepted invalid model {model:?}"
        );
    }
    // Aliased harmonic (12th at 1.2 MHz past 1 MHz Nyquist here).
    let aliased_times: Vec<f64> = (0..512).map(|i| i as f64 * 1e-6).collect();
    assert_eq!(
        design_matrix(&aliased_times, 100_000.0, 0.0, &good, 1e6)
            .unwrap_err()
            .code(),
        "aliased_harmonic"
    );
    // Rank-deficient and ill-conditioned whitened designs.
    let tolerances = JointSolverTolerances::default();
    let rank_deficient = DMatrix::from_row_slice(3, 2, &[1.0, 2.0, 3.0, 6.0, 5.0, 10.0]);
    assert_eq!(
        solve_direct(
            &rank_deficient,
            &DVector::from_vec(vec![1.0, 2.0, 3.0]),
            tolerances
        )
        .unwrap_err()
        .code(),
        "rank_deficient_design"
    );
    // Full-rank but condition 1e9 above the 1e8 cap; 1e-13 would instead be
    // rank-deficient under the 1e-10 relative threshold (correctly).
    let ill = DMatrix::from_row_slice(2, 2, &[1.0, 0.0, 0.0, 1e-9]);
    assert_eq!(
        solve_direct(&ill, &DVector::from_vec(vec![1.0, 1.0]), tolerances)
            .unwrap_err()
            .code(),
        "ill_conditioned_design"
    );
    let borderline = DMatrix::from_row_slice(2, 2, &[1.0, 0.0, 0.0, 1e-7]);
    let (_, _, rank, condition) =
        solve_direct(&borderline, &DVector::from_vec(vec![1.0, 1.0]), tolerances).unwrap();
    assert_eq!(rank, 2);
    assert!(condition < DEFAULT_MAX_CONDITION);
    // Correlated modes are explicit errors, not silent OLS.
    let correlated = NoiseModel {
        mode: NoiseMode::StationaryCorrelated,
        reference_variance_v2: 1.0,
        variance_bins: None,
    };
    validate_noise_model(&correlated).unwrap();
    // Full v1 model on enough rows that design succeeds and whitening is
    // reached, where the correlated mode is explicitly rejected.
    let times: Vec<f64> = (0..32).map(|i| i as f64 * 0.01).collect();
    let signal: Vec<f64> = times
        .iter()
        .map(|t| (2.0 * std::f64::consts::PI * t).sin())
        .collect();
    assert_eq!(
        estimate_joint(
            &times,
            &signal,
            1.0,
            0.0,
            100.0,
            &settings(&model_1_to_12(), &correlated),
        )
        .unwrap_err()
        .code(),
        "unsupported_noise_mode"
    );
}

// AT-010: Rust direct solver against committed SciPy oracle outputs (TOL-03).
#[test]
fn direct_solver_matches_committed_oracle() {
    let oracle: OracleExpected =
        serde_json::from_str(include_str!("fixtures/joint-gls/oracle-expected.json")).unwrap();
    let payload: Conventions =
        serde_json::from_str(include_str!("fixtures/joint-gls/conventions.json")).unwrap();
    assert!(!oracle.cases.is_empty());
    for expected in &oracle.cases {
        let case = payload
            .cases
            .iter()
            .find(|case| case.name == expected.name)
            .unwrap();
        let model = HarmonicSignalModel {
            fit_harmonics: case.fit_harmonics.clone(),
            output_harmonics: case.output_harmonics.clone(),
            envelope_degree: 0,
        };
        let estimate = estimate_joint(
            &case_times(case),
            &case.signal,
            case.f_ref,
            case.phase_rad,
            1.0 / case.dt,
            &settings(&model, &identity_noise()),
        )
        .unwrap();
        assert_eq!(estimate.rank, expected.rank, "{}", case.name);
        let scale = expected
            .beta
            .iter()
            .fold(0.0_f64, |max, value| max.max(value.abs()))
            .max(1.0);
        for (got, want) in estimate.beta.iter().zip(expected.beta.iter()) {
            assert!(
                (got - want).abs() <= 1.0e-10 + 1.0e-8 * scale,
                "{} beta: {got} vs {want}",
                case.name
            );
        }
        for key in case.output_harmonics.iter().map(usize::to_string) {
            let position = case
                .output_harmonics
                .iter()
                .position(|h| h.to_string() == key)
                .unwrap();
            let want = expected.xy.get(&key).unwrap();
            assert!(
                (estimate.xy[2 * position] - want.x).abs() <= 1.0e-10 + 1.0e-8 * scale,
                "{} X{key}",
                case.name
            );
            assert!(
                (estimate.xy[2 * position + 1] - want.y).abs() <= 1.0e-10 + 1.0e-8 * scale,
                "{} Y{key}",
                case.name
            );
        }
    }
}

// AT-022 core: 1/4 covariance scaling, upper-triangle packing, rotation.
#[test]
fn covariance_contracts_hold() {
    let case = convention_case("large_even_weak_odd");
    let model = model_1_to_12();
    let times = case_times(&case);
    let design = design_matrix(&times, case.f_ref, case.phase_rad, &model, 1.0 / case.dt).unwrap();
    let (whitened, _, scale) = whiten(
        &design,
        &case.signal,
        case.phase_rad,
        case.f_ref,
        &times,
        &identity_noise(),
        JointSolverTolerances::default(),
    )
    .unwrap();
    assert_eq!(scale, 1.0);
    let covariance = covariance_from_qr(&whitened, scale).unwrap();
    // Symmetry and 1/4 XY scaling against the mapped blocks.
    for i in 0..covariance.nrows() {
        for j in 0..covariance.ncols() {
            close(covariance[(i, j)], covariance[(j, i)], 1.0e-12, "symmetry");
        }
    }
    let mapped = map_covariance_to_xy(&covariance, &model).unwrap();
    assert_eq!((mapped.nrows(), mapped.ncols()), (12, 12));
    // X3 block: beta indices a3 = 1 + 2*2 = 5, b3 = 6; XY positions 4, 5.
    close(mapped[(4, 4)], covariance[(6, 6)] / 4.0, 1.0e-12, "X3 var");
    close(mapped[(5, 5)], covariance[(5, 5)] / 4.0, 1.0e-12, "Y3 var");
    close(
        mapped[(4, 5)],
        covariance[(6, 5)] / 4.0,
        1.0e-12,
        "X3Y3 cov",
    );
    let packed = pack_upper_triangle(&mapped).unwrap();
    assert_eq!(packed.len(), 78);
    assert_eq!(packed[0], mapped[(0, 0)]);
    assert_eq!(packed[1], mapped[(0, 1)]);
    assert_eq!(packed[12], mapped[(1, 1)]);
    // Rotation by zeros is identity; composition holds: R(a)R(b) = R(a+b).
    let deltas = [0.1, -0.3, 0.7, 0.0, 1.2, -0.9];
    let rotated = rotate_xy_covariance(&mapped, &deltas).unwrap();
    let identity = rotate_xy_covariance(&mapped, &[0.0; 6]).unwrap();
    for i in 0..12 {
        for j in 0..12 {
            close(identity[(i, j)], mapped[(i, j)], 1.0e-12, "zero rotation");
        }
    }
    let twice = rotate_xy_covariance(&rotated, &deltas).unwrap();
    let doubled: [f64; 6] = std::array::from_fn(|i| deltas[i] * 2.0);
    let direct = rotate_xy_covariance(&mapped, &doubled).unwrap();
    for i in 0..12 {
        for j in 0..12 {
            close(twice[(i, j)], direct[(i, j)], 1.0e-9, "composition");
        }
    }
    // Rotated means agree with the established phase routine, and rotated
    // variances stay nonnegative.
    let estimate = estimate_joint(
        &times,
        &case.signal,
        case.f_ref,
        case.phase_rad,
        1.0 / case.dt,
        &settings(&model, &identity_noise()),
    )
    .unwrap();
    for (k, delta) in deltas.iter().enumerate() {
        let (xs, _) = rotate_phase(
            &estimate.xy[2 * k..2 * k + 1],
            &estimate.xy[2 * k + 1..2 * k + 2],
            *delta,
        )
        .unwrap();
        let (sa, ca) = delta.sin_cos();
        close(
            xs[0],
            estimate.xy[2 * k] * ca + estimate.xy[2 * k + 1] * sa,
            1.0e-12,
            &format!("rotated mean {k}"),
        );
        assert!(rotated[(2 * k, 2 * k)] >= 0.0, "rotated var {k}");
        assert!(rotated[(2 * k + 1, 2 * k + 1)] >= 0.0, "rotated var {k}");
    }
    // Trace per 2x2 block is rotation-invariant.
    for k in 0..6 {
        let before = mapped[(2 * k, 2 * k)] + mapped[(2 * k + 1, 2 * k + 1)];
        let after = rotated[(2 * k, 2 * k)] + rotated[(2 * k + 1, 2 * k + 1)];
        close(after, before, 1.0e-9, &format!("trace {k}"));
    }
}

// Scalar reference variance scales uncertainty, never the estimate.
#[test]
fn reference_variance_scales_covariance_only() {
    let case = convention_case("negative_fractional_t0");
    let model = model_1_to_12();
    let times = case_times(&case);
    let unit = estimate_joint(
        &times,
        &case.signal,
        case.f_ref,
        case.phase_rad,
        1.0 / case.dt,
        &settings(&model, &identity_noise()),
    )
    .unwrap();
    let scaled_noise = NoiseModel {
        mode: NoiseMode::Identity,
        reference_variance_v2: 4.0,
        variance_bins: None,
    };
    let scaled = estimate_joint(
        &times,
        &case.signal,
        case.f_ref,
        case.phase_rad,
        1.0 / case.dt,
        &settings(&model, &scaled_noise),
    )
    .unwrap();
    assert_eq!(unit.beta, scaled.beta);
    for (row_u, row_s) in unit
        .covariance_beta
        .iter()
        .zip(scaled.covariance_beta.iter())
    {
        for (a, b) in row_u.iter().zip(row_s.iter()) {
            close(*b, 4.0 * a, 1.0e-12, "variance scale");
        }
    }
}

// F1: finite inputs that overflow arithmetic report non_finite_output;
// factorization inputs are validated before use.
#[test]
fn non_finite_results_are_rejected() {
    let case = convention_case("large_even_weak_odd");
    let model = model_1_to_12();
    let times = case_times(&case);
    let rate = 1.0 / case.dt;
    let settings = settings(&model, &identity_noise());
    // All response samples at finite 1e308 overflow the coefficients.
    let huge = vec![1.0e308; case.samples];
    assert_eq!(
        estimate_joint(&times, &huge, case.f_ref, case.phase_rad, rate, &settings)
            .unwrap_err()
            .code(),
        "non_finite_output"
    );
    // Finite 1e155-scale response stays finite through scale-safe RMS.
    let big: Vec<f64> = (0..case.samples)
        .map(|i| 1.0e155 * (1.0 + 0.001 * (i as f64).sin()))
        .collect();
    let estimate =
        estimate_joint(&times, &big, case.f_ref, case.phase_rad, rate, &settings).unwrap();
    assert!(estimate.beta.iter().all(|v| v.is_finite()));
    assert!(estimate.residual_rms.is_finite() && estimate.residual_rms > 0.0);
    // NaN design or response never reaches factorization.
    let design = design_matrix(&times, case.f_ref, case.phase_rad, &model, rate).unwrap();
    let mut nan_design = design.clone();
    nan_design[(0, 0)] = f64::NAN;
    assert_eq!(
        whiten(
            &nan_design,
            &case.signal,
            case.phase_rad,
            case.f_ref,
            &times,
            &identity_noise(),
            JointSolverTolerances::default()
        )
        .unwrap_err()
        .code(),
        "non_finite_input"
    );
    let mut nan_signal = case.signal.clone();
    nan_signal[7] = f64::INFINITY;
    assert_eq!(
        estimate_joint(
            &times,
            &nan_signal,
            case.f_ref,
            case.phase_rad,
            rate,
            &settings
        )
        .unwrap_err()
        .code(),
        "non_finite_input"
    );
    // Helper preconditions return errors, never panics or silent garbage.
    assert_eq!(
        covariance_from_qr(&DMatrix::from_row_slice(2, 3, &[1.0; 6]), 1.0)
            .unwrap_err()
            .code(),
        "dimension_mismatch"
    );
    assert_eq!(
        covariance_from_qr(&DMatrix::identity(2, 2), -1.0)
            .unwrap_err()
            .code(),
        "invalid_noise_model"
    );
    assert_eq!(
        interpolate_variance(&[0.1], &[1.0, -2.0])
            .unwrap_err()
            .code(),
        "invalid_noise_model"
    );
    assert_eq!(
        rotate_xy_covariance(&DMatrix::identity(12, 12), &[f64::NAN; 6])
            .unwrap_err()
            .code(),
        "non_finite_input"
    );
    assert_eq!(
        map_to_xy(&[f64::NAN; 25], &model).unwrap_err().code(),
        "non_finite_input"
    );
}

// F2: incoherent timebases are rejected before design construction.
#[test]
fn timebase_validation_rejects_incoherent_grids() {
    let rate = 100_000.0;
    let dt = 1.0 / rate;
    let clean: Vec<f64> = (0..64).map(|i| i as f64 * dt).collect();
    assert_eq!(validate_timebase(&clean, rate).unwrap(), dt);
    // Duplicate timestamp.
    let mut duplicated = clean.clone();
    duplicated[10] = duplicated[9];
    // Adjacent swap.
    let mut swapped = clean.clone();
    swapped.swap(20, 21);
    // Quarter-step jitter on one sample.
    let mut jittered = clean.clone();
    jittered[30] += 0.25 * dt;
    for grid in [&duplicated, &swapped, &jittered] {
        assert_eq!(
            validate_timebase(grid, rate).unwrap_err().code(),
            "invalid_timebase"
        );
    }
    // Declared rate disagrees with coordinates by orders of magnitude.
    assert_eq!(
        validate_timebase(&clean, 1.0).unwrap_err().code(),
        "invalid_timebase"
    );
    // Serialization roundoff within the established allowance still passes,
    // including a wobble at half the published tolerance.
    let wobbled: Vec<f64> = clean
        .iter()
        .map(|t| t + t.abs() * (f64::EPSILON * 4.0 + TIMEBASE_RELATIVE_TOLERANCE * 0.5))
        .collect();
    assert!(validate_timebase(&wobbled, rate).is_ok());
}

// F3: identical covariances report identical residual RMS in every mode.
#[test]
fn residual_rms_matches_across_equivalent_modes() {
    let case = convention_case("mixed_phases_fractional_t0");
    let model = model_1_to_12();
    let times = case_times(&case);
    let rate = 1.0 / case.dt;
    let perturbed: Vec<f64> = case
        .signal
        .iter()
        .zip(times.iter())
        .map(|(y, t)| y + 1.0e-3 * (2.0 * std::f64::consts::PI * 12345.0 * t).sin())
        .collect();
    let identity = estimate_joint(
        &times,
        &perturbed,
        case.f_ref,
        case.phase_rad,
        rate,
        &settings(
            &model,
            &NoiseModel {
                mode: NoiseMode::Identity,
                reference_variance_v2: 4.0,
                variance_bins: None,
            },
        ),
    )
    .unwrap();
    let diagonal = estimate_joint(
        &times,
        &perturbed,
        case.f_ref,
        case.phase_rad,
        rate,
        &settings(
            &model,
            &NoiseModel {
                mode: NoiseMode::PhaseDiagonal,
                reference_variance_v2: 1.0,
                variance_bins: Some(vec![4.0; 8]),
            },
        ),
    )
    .unwrap();
    for (a, b) in identity.beta.iter().zip(diagonal.beta.iter()) {
        close(*a, *b, 1.0e-12, "R=4I beta");
    }
    for (row_a, row_b) in identity
        .covariance_beta
        .iter()
        .zip(diagonal.covariance_beta.iter())
    {
        for (a, b) in row_a.iter().zip(row_b.iter()) {
            close(*a, *b, 1.0e-12, "R=4I covariance");
        }
    }
    close(
        identity.residual_rms,
        diagonal.residual_rms,
        1.0e-12,
        "R=4I residual RMS",
    );
}

// F5: realized noise-covariance condition is gated independently (TOL-07).
#[test]
fn noise_condition_gate_rejects_extreme_heteroscedasticity() {
    let case = convention_case("mixed_phases_fractional_t0");
    let model = model_1_to_12();
    let times = case_times(&case);
    let rate = 1.0 / case.dt;
    let extreme = NoiseModel {
        mode: NoiseMode::PhaseDiagonal,
        reference_variance_v2: 1.0,
        variance_bins: Some(vec![1e-12, 1.0]),
    };
    assert_eq!(
        estimate_joint(
            &times,
            &case.signal,
            case.f_ref,
            case.phase_rad,
            rate,
            &settings(&model, &extreme),
        )
        .unwrap_err()
        .code(),
        "ill_conditioned_noise_model"
    );
    let healthy = NoiseModel {
        mode: NoiseMode::PhaseDiagonal,
        reference_variance_v2: 1.0,
        variance_bins: Some(vec![1.0, 4.0]),
    };
    estimate_joint(
        &times,
        &case.signal,
        case.f_ref,
        case.phase_rad,
        rate,
        &settings(&model, &healthy),
    )
    .unwrap();
}

#[derive(Deserialize)]
struct WeightedGolden {
    cases: Vec<WeightedCase>,
}

#[derive(Deserialize)]
struct WeightedCase {
    name: String,
    f_ref: f64,
    dt: f64,
    t_start: f64,
    samples: usize,
    phase_rad: f64,
    fit_harmonics: Vec<usize>,
    output_harmonics: Vec<usize>,
    bins: usize,
    v0: f64,
    variance_profile: Vec<f64>,
    signal: Vec<f64>,
    oracle: WeightedOracle,
}

#[derive(Deserialize)]
struct WeightedOracle {
    beta: Vec<f64>,
    xy: BTreeMap<String, Xy>,
    rank: usize,
    residual_norm: f64,
    covariance_beta: Vec<Vec<f64>>,
}

fn assert_cov_close(got: &[Vec<f64>], want: &[Vec<f64>], context: &str) {
    // TOL-04: Frobenius relative error within 1e-7 with a per-entry 1e-12
    // absolute floor near zero.
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    let mut entries = 0;
    for (grow, wrow) in got.iter().zip(want.iter()) {
        for (g, w) in grow.iter().zip(wrow.iter()) {
            numerator += (g - w).powi(2);
            denominator += w.powi(2);
            entries += 1;
        }
    }
    let floor = 1e-24 * entries as f64;
    assert!(
        (numerator / (denominator + floor)).sqrt() <= 1.0e-7,
        "{context}: frob {numerator:.3e} / {denominator:.3e}"
    );
}

// F6/F7: nonzero-residual diagonal estimation against committed oracle
// coefficients, residuals, and absolute covariance.
#[test]
fn diagonal_mode_matches_oracle_with_nonzero_residual() {
    let payload: WeightedGolden =
        serde_json::from_str(include_str!("fixtures/joint-gls/weighted-golden.json")).unwrap();
    assert_eq!(payload.cases.len(), 1);
    let case = &payload.cases[0];
    let model = HarmonicSignalModel {
        fit_harmonics: case.fit_harmonics.clone(),
        output_harmonics: case.output_harmonics.clone(),
        envelope_degree: 0,
    };
    assert_eq!(case.bins, case.variance_profile.len());
    let absolute_bins: Vec<f64> = case.variance_profile.iter().map(|v| case.v0 * v).collect();
    let noise = NoiseModel {
        mode: NoiseMode::PhaseDiagonal,
        reference_variance_v2: case.v0,
        variance_bins: Some(absolute_bins),
    };
    let times: Vec<f64> = (0..case.samples)
        .map(|index| case.t_start + index as f64 * case.dt)
        .collect();
    let estimate = estimate_joint(
        &times,
        &case.signal,
        case.f_ref,
        case.phase_rad,
        1.0 / case.dt,
        &settings(&model, &noise),
    )
    .unwrap();
    assert_eq!(estimate.rank, case.oracle.rank);
    assert!(estimate.residual_rms > 0.0);
    let scale = case
        .oracle
        .beta
        .iter()
        .fold(0.0_f64, |max, value| max.max(value.abs()))
        .max(1.0);
    for (got, want) in estimate.beta.iter().zip(case.oracle.beta.iter()) {
        assert!(
            (got - want).abs() <= 1.0e-10 + 1.0e-8 * scale,
            "{} beta: {got} vs {want}",
            case.name
        );
    }
    for key in case.output_harmonics.iter().map(usize::to_string) {
        let position = case
            .output_harmonics
            .iter()
            .position(|h| h.to_string() == key)
            .unwrap();
        let want = case.oracle.xy.get(&key).unwrap();
        assert!(
            (estimate.xy[2 * position] - want.x).abs() <= 1.0e-10 + 1.0e-8 * scale,
            "{} X{key}",
            case.name
        );
        assert!(
            (estimate.xy[2 * position + 1] - want.y).abs() <= 1.0e-10 + 1.0e-8 * scale,
            "{} Y{key}",
            case.name
        );
    }
    assert_cov_close(
        &estimate.covariance_beta,
        &case.oracle.covariance_beta,
        &case.name,
    );
    // Standardized RMS reproduces the oracle whitened norm per sample.
    close(
        estimate.residual_rms,
        case.oracle.residual_norm / (case.samples as f64).sqrt(),
        1.0e-9,
        "standardized residual RMS",
    );
}

// F6: constant-profile diagonal mode equals scaled identity inference.
#[test]
fn constant_profile_matches_identity() {
    let case = convention_case("large_even_weak_odd");
    let model = model_1_to_12();
    let times = case_times(&case);
    let rate = 1.0 / case.dt;
    let perturbed: Vec<f64> = case
        .signal
        .iter()
        .zip(times.iter())
        .map(|(y, t)| y + 2.0e-4 * (2.0 * std::f64::consts::PI * 7777.0 * t).cos())
        .collect();
    let identity = estimate_joint(
        &times,
        &perturbed,
        case.f_ref,
        case.phase_rad,
        rate,
        &settings(
            &model,
            &NoiseModel {
                mode: NoiseMode::Identity,
                reference_variance_v2: 2.5,
                variance_bins: None,
            },
        ),
    )
    .unwrap();
    let diagonal = estimate_joint(
        &times,
        &perturbed,
        case.f_ref,
        case.phase_rad,
        rate,
        &settings(
            &model,
            &NoiseModel {
                mode: NoiseMode::PhaseDiagonal,
                reference_variance_v2: 1.0,
                variance_bins: Some(vec![2.5; 4]),
            },
        ),
    )
    .unwrap();
    for (a, b) in identity.beta.iter().zip(diagonal.beta.iter()) {
        close(*a, *b, 1.0e-12, "constant-profile beta");
    }
    assert_cov_close(
        &diagonal.covariance_beta,
        &identity.covariance_beta,
        "profile",
    );
    close(
        identity.residual_rms,
        diagonal.residual_rms,
        1.0e-12,
        "profile RMS",
    );
}

// F7: reported target variance reproduces the theoretical prediction.
#[test]
fn reported_target_variance_matches_theory() {
    let case = convention_case("large_even_weak_odd");
    let model = model_1_to_12();
    let times = case_times(&case);
    let design = design_matrix(&times, case.f_ref, case.phase_rad, &model, 1.0 / case.dt).unwrap();
    let gram_inverse = (&design.transpose() * &design).try_inverse().unwrap();
    let estimate = estimate_joint(
        &times,
        &case.signal,
        case.f_ref,
        case.phase_rad,
        1.0 / case.dt,
        &settings(&model, &identity_noise()),
    )
    .unwrap();
    // X3 selector in beta order with the 1/2 amplitude mapping.
    let parameters = gram_inverse.nrows();
    let mut selector = vec![0.0; parameters];
    selector[2 + 2 * 2] = 0.5;
    let mut theory = 0.0;
    for (i, &ci) in selector.iter().enumerate() {
        for (j, &cj) in selector.iter().enumerate() {
            theory += ci * gram_inverse[(i, j)] * cj;
        }
    }
    let reported = estimate.covariance_xy[4][4];
    close(reported, theory, 1.0e-9, "X3 target variance");
}
