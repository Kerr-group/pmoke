//! WP-4 correlated-mode evidence: stationary (`v0 * C_N`) and
//! phase-correlated (`S_N C_N S_N`) estimation against committed oracle
//! outputs, mode/component preflight, regularization/SPD policy, and
//! analytical anchors.
//!
//! Acceptance mapping: AT-007 (mode semantics/capabilities), AT-008
//! (regularization/SPD), AT-009 (rank/conditioning via the shared
//! whitened-design gates), AT-010 (independent solver oracle).

use nalgebra::DMatrix;
use pmoke_analysis_core::{
    CorrelationKernel, HarmonicSignalModel, JointHarmonicSettings, JointSolverTolerances,
    NoiseMode, NoiseModel, cholesky_factor, estimate_joint, forward_substitute,
    interpolate_variance, toeplitz_from_lags, validate_correlation_kernel, validate_noise_model,
};
use serde::Deserialize;
use std::collections::BTreeMap;

fn close(a: f64, b: f64, tolerance: f64, context: &str) {
    assert!(
        (a - b).abs() <= tolerance,
        "{context}: {a} vs {b} (tol {tolerance})"
    );
}

#[derive(Deserialize)]
struct Xy {
    x: f64,
    y: f64,
}

#[derive(Deserialize)]
struct CorrelatedGolden {
    cases: Vec<CorrelatedCase>,
}

#[derive(Deserialize)]
struct CorrelatedCase {
    name: String,
    f_ref: f64,
    dt: f64,
    t_start: f64,
    samples: usize,
    phase_rad: f64,
    fit_harmonics: Vec<usize>,
    output_harmonics: Vec<usize>,
    mode: String,
    v0: f64,
    #[serde(default)]
    variances: Vec<f64>,
    #[serde(default)]
    variance_bins: Vec<f64>,
    lags: Vec<f64>,
    lag_step_s: f64,
    signal: Vec<f64>,
    true_beta: Vec<f64>,
    oracle: CorrelatedOracle,
}

#[derive(Deserialize)]
struct CorrelatedOracle {
    beta: Vec<f64>,
    xy: BTreeMap<String, Xy>,
    rank: usize,
    residual_norm: f64,
    covariance_beta: Vec<Vec<f64>>,
    jitter_applied: f64,
}

fn golden() -> CorrelatedGolden {
    serde_json::from_str(include_str!("fixtures/joint-gls/correlated-golden.json")).unwrap()
}

fn case(name: &str) -> CorrelatedCase {
    golden()
        .cases
        .into_iter()
        .find(|case| case.name == name)
        .unwrap_or_else(|| panic!("missing case {name}"))
}

fn settings_for(case: &CorrelatedCase) -> JointHarmonicSettings {
    let mode = match case.mode.as_str() {
        "stationary" => NoiseMode::StationaryCorrelated,
        "phase_correlated" => NoiseMode::PhaseCorrelated,
        other => panic!("unknown mode {other}"),
    };
    let (variance_bins, correlation) = match mode {
        NoiseMode::StationaryCorrelated => (
            None,
            Some(CorrelationKernel {
                lags: case.lags.clone(),
                lag_step_s: case.lag_step_s,
            }),
        ),
        // The committed absolute bins carry the true profile; per-sample
        // interpolation then reproduces the oracle variances.
        NoiseMode::PhaseCorrelated => (
            Some(case.variance_bins.clone()),
            Some(CorrelationKernel {
                lags: case.lags.clone(),
                lag_step_s: case.lag_step_s,
            }),
        ),
        _ => unreachable!(),
    };
    JointHarmonicSettings {
        model: HarmonicSignalModel {
            fit_harmonics: case.fit_harmonics.clone(),
            output_harmonics: case.output_harmonics.clone(),
            envelope_degree: 0,
        },
        noise: NoiseModel {
            mode,
            reference_variance_v2: case.v0,
            variance_bins,
            correlation,
        },
        tolerances: JointSolverTolerances::default(),
    }
}

fn assert_cov_close(got: &[Vec<f64>], want: &[Vec<f64>], context: &str) {
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

// AT-007/010: both correlated modes against committed oracle coefficients,
// quadratures, standardized residuals, and absolute covariances.
#[test]
fn correlated_modes_match_oracle() {
    for name in ["stationary_ar1", "phase_correlated_hetero"] {
        let case = case(name);
        let settings = settings_for(&case);
        let times: Vec<f64> = (0..case.samples)
            .map(|index| case.t_start + index as f64 * case.dt)
            .collect();
        let estimate = estimate_joint(
            &times,
            &case.signal,
            case.f_ref,
            case.phase_rad,
            1.0 / case.dt,
            &settings,
        )
        .unwrap();
        assert_eq!(estimate.rank, case.oracle.rank);
        assert_eq!(estimate.jitter_applied_v2, case.oracle.jitter_applied);
        let scale = case
            .oracle
            .beta
            .iter()
            .fold(0.0_f64, |max, value| max.max(value.abs()))
            .max(1.0);
        for (got, want) in estimate.beta.iter().zip(case.oracle.beta.iter()) {
            assert!(
                (got - want).abs() <= 1.0e-10 + 1.0e-8 * scale,
                "{name} beta: {got} vs {want}"
            );
        }
        for (position, harmonic) in case.output_harmonics.iter().enumerate() {
            let want = case.oracle.xy.get(&harmonic.to_string()).unwrap();
            assert!(
                (estimate.xy[2 * position] - want.x).abs() <= 1.0e-10 + 1.0e-8 * scale,
                "{name} X{harmonic}"
            );
            assert!(
                (estimate.xy[2 * position + 1] - want.y).abs() <= 1.0e-10 + 1.0e-8 * scale,
                "{name} Y{harmonic}"
            );
        }
        assert_cov_close(
            &estimate.covariance_beta,
            &case.oracle.covariance_beta,
            name,
        );
        // Fixture self-consistency: committed per-sample variances reproduce
        // the committed absolute bins through the same interpolation.
        if name == "phase_correlated_hetero" {
            let phases: Vec<f64> = times
                .iter()
                .map(|time| std::f64::consts::TAU * case.f_ref * time - case.phase_rad)
                .collect();
            let recomputed = interpolate_variance(&phases, &case.variance_bins).unwrap();
            for (index, (got, want)) in recomputed.iter().zip(case.variances.iter()).enumerate() {
                close(*got, *want, 1e-9, &format!("{name} variance[{index}]"));
            }
        }
        // Standardized RMS reproduces the oracle whitened norm per sample in
        // both correlated modes (unit variance scale by construction).
        close(
            estimate.residual_rms,
            case.oracle.residual_norm / (case.samples as f64).sqrt(),
            1e-9,
            &format!("{name} standardized residual RMS"),
        );
    }
}

// AT-007: correlated whitening is not an OLS stub. On correlated noise the
// correlated estimate differs from the identity estimate beyond arithmetic
// noise, and both match their own oracles.
#[test]
fn correlated_estimate_is_not_identity() {
    let case = case("stationary_ar1");
    let settings = settings_for(&case);
    let times: Vec<f64> = (0..case.samples)
        .map(|index| case.t_start + index as f64 * case.dt)
        .collect();
    let correlated = estimate_joint(
        &times,
        &case.signal,
        case.f_ref,
        case.phase_rad,
        1.0 / case.dt,
        &settings,
    )
    .unwrap();
    let identity = estimate_joint(
        &times,
        &case.signal,
        case.f_ref,
        case.phase_rad,
        1.0 / case.dt,
        &JointHarmonicSettings {
            model: settings.model.clone(),
            noise: NoiseModel {
                mode: NoiseMode::Identity,
                reference_variance_v2: 1.0,
                variance_bins: None,
                correlation: None,
            },
            tolerances: JointSolverTolerances::default(),
        },
    )
    .unwrap();
    let max_diff = correlated
        .beta
        .iter()
        .zip(identity.beta.iter())
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f64::max);
    assert!(
        max_diff > 1e-6,
        "correlated whitening must move the estimate, got {max_diff:.3e}"
    );
}

// Truth recovery: committed GLS estimates land within 5 sigma of the known
// synthesis coefficients under the committed design covariance.
#[test]
fn correlated_truth_recovery_within_covariance() {
    for name in ["stationary_ar1", "phase_correlated_hetero"] {
        let case = case(name);
        let settings = settings_for(&case);
        let times: Vec<f64> = (0..case.samples)
            .map(|index| case.t_start + index as f64 * case.dt)
            .collect();
        let estimate = estimate_joint(
            &times,
            &case.signal,
            case.f_ref,
            case.phase_rad,
            1.0 / case.dt,
            &settings,
        )
        .unwrap();
        for (index, ((got, truth), row)) in estimate
            .beta
            .iter()
            .zip(case.true_beta.iter())
            .zip(estimate.covariance_beta.iter())
            .enumerate()
        {
            let sigma = row[index].max(0.0).sqrt();
            assert!(
                (got - truth).abs() <= 5.0 * sigma,
                "{name} beta[{index}]: {got} vs truth {truth} (5-sigma {sigma:.3e})"
            );
        }
    }
}

// AT-007: missing components fail preflight; extraneous components fail
// validation; malformed kernels fail before factorization.
#[test]
fn correlated_component_preflight() {
    let kernel = CorrelationKernel {
        lags: vec![1.0, 0.5],
        lag_step_s: 1e-5,
    };
    let base = NoiseModel {
        mode: NoiseMode::StationaryCorrelated,
        reference_variance_v2: 1.0,
        variance_bins: None,
        correlation: Some(kernel.clone()),
    };
    validate_noise_model(&base).unwrap();
    // Missing kernel.
    let missing = NoiseModel {
        correlation: None,
        ..base.clone()
    };
    assert_eq!(
        validate_noise_model(&missing).unwrap_err().code(),
        "missing_noise_component"
    );
    // Extraneous bins on the stationary mode.
    let extraneous = NoiseModel {
        variance_bins: Some(vec![1.0, 2.0]),
        ..base.clone()
    };
    assert_eq!(
        validate_noise_model(&extraneous).unwrap_err().code(),
        "invalid_noise_model"
    );
    // Kernel on a diagonal mode.
    let misplaced = NoiseModel {
        mode: NoiseMode::PhaseDiagonal,
        variance_bins: Some(vec![1.0, 2.0]),
        correlation: Some(kernel.clone()),
        ..base.clone()
    };
    assert_eq!(
        validate_noise_model(&misplaced).unwrap_err().code(),
        "invalid_noise_model"
    );
    // Malformed kernels.
    for (lags, step, code) in [
        (vec![], 1e-5, "invalid_noise_model"),
        (vec![0.9, 0.5], 1e-5, "invalid_noise_model"),
        (vec![1.0, f64::NAN], 1e-5, "non_finite_input"),
        (vec![1.0, 0.5], 0.0, "invalid_noise_model"),
        (vec![1.0, 0.5], f64::INFINITY, "invalid_noise_model"),
    ] {
        let bad = CorrelationKernel {
            lags,
            lag_step_s: step,
        };
        assert_eq!(validate_correlation_kernel(&bad).unwrap_err().code(), code);
    }
    // Phase-correlated without bins.
    let no_bins = NoiseModel {
        mode: NoiseMode::PhaseCorrelated,
        variance_bins: None,
        correlation: Some(kernel),
        ..base.clone()
    };
    assert_eq!(
        validate_noise_model(&no_bins).unwrap_err().code(),
        "missing_noise_component"
    );
}

// AT-008: regularization policy. Indefinite factors fail with zero jitter;
// bounded jitter succeeds once and records the applied amount; a ceiling
// below the definiteness gap still fails; invalid ceilings are rejected.
#[test]
fn jitter_policy_is_bounded_and_recorded() {
    // Singular 2x2 ones factor: fails plain, succeeds with jitter 0.1.
    let singular = DMatrix::from_row_slice(2, 2, &[1.0, 1.0, 1.0, 1.0]);
    assert_eq!(
        cholesky_factor(&singular, 0.0).unwrap_err().code(),
        "covariance_not_spd"
    );
    let (_, applied) = cholesky_factor(&singular, 0.1).unwrap();
    assert_eq!(applied, 0.1);
    // Genuinely indefinite tridiagonal ([1,1] lags): jitter 0.1 cannot
    // close a definiteness gap near 1.0.
    let indefinite = toeplitz_from_lags(&[1.0, 1.0], 32);
    assert_eq!(
        cholesky_factor(&indefinite, 0.1).unwrap_err().code(),
        "covariance_not_spd"
    );
    assert_eq!(
        cholesky_factor(&singular, -1.0).unwrap_err().code(),
        "non_finite_input"
    );
    assert_eq!(
        cholesky_factor(&singular, f64::NAN).unwrap_err().code(),
        "non_finite_input"
    );
    // End to end: marginally indefinite kernel ([1, 0.5007] at N=64 needs
    // 2.4e-4 of jitter) through estimate_joint.
    let times: Vec<f64> = (0..64).map(|i| i as f64 * 1e-5).collect();
    let signal: Vec<f64> = times
        .iter()
        .map(|t| (2.0 * std::f64::consts::PI * 1000.0 * t).sin())
        .collect();
    let mut settings = JointHarmonicSettings {
        model: HarmonicSignalModel {
            fit_harmonics: (1..=6).collect(),
            output_harmonics: (1..=6).collect(),
            envelope_degree: 0,
        },
        noise: NoiseModel {
            mode: NoiseMode::StationaryCorrelated,
            reference_variance_v2: 1.0,
            variance_bins: None,
            correlation: Some(CorrelationKernel {
                lags: vec![1.0, 0.5007],
                lag_step_s: 1e-5,
            }),
        },
        tolerances: JointSolverTolerances::default(),
    };
    assert_eq!(
        estimate_joint(&times, &signal, 1000.0, 0.0, 100_000.0, &settings)
            .unwrap_err()
            .code(),
        "covariance_not_spd"
    );
    settings.tolerances.max_jitter_v2 = 0.01;
    let estimate = estimate_joint(&times, &signal, 1000.0, 0.0, 100_000.0, &settings).unwrap();
    assert_eq!(estimate.jitter_applied_v2, 0.01);
    assert_eq!(estimate.rank, 13);
}

// Constant-profile phase-correlated equals scaled stationary inference:
// with flat S the order of variance scaling and the C solve is immaterial,
// pinning the NUMERICS 4.2 operation order for the non-constant path.
#[test]
fn constant_profile_matches_stationary() {
    let case = case("stationary_ar1");
    let times: Vec<f64> = (0..case.samples)
        .map(|index| case.t_start + index as f64 * case.dt)
        .collect();
    let model = HarmonicSignalModel {
        fit_harmonics: case.fit_harmonics.clone(),
        output_harmonics: case.output_harmonics.clone(),
        envelope_degree: 0,
    };
    let stationary = estimate_joint(
        &times,
        &case.signal,
        case.f_ref,
        case.phase_rad,
        1.0 / case.dt,
        &JointHarmonicSettings {
            model: model.clone(),
            noise: NoiseModel {
                mode: NoiseMode::StationaryCorrelated,
                reference_variance_v2: 2.5,
                variance_bins: None,
                correlation: Some(CorrelationKernel {
                    lags: case.lags.clone(),
                    lag_step_s: case.lag_step_s,
                }),
            },
            tolerances: JointSolverTolerances::default(),
        },
    )
    .unwrap();
    let phase = estimate_joint(
        &times,
        &case.signal,
        case.f_ref,
        case.phase_rad,
        1.0 / case.dt,
        &JointHarmonicSettings {
            model,
            noise: NoiseModel {
                mode: NoiseMode::PhaseCorrelated,
                reference_variance_v2: 1.0,
                variance_bins: Some(vec![2.5; 4]),
                correlation: Some(CorrelationKernel {
                    lags: case.lags.clone(),
                    lag_step_s: case.lag_step_s,
                }),
            },
            tolerances: JointSolverTolerances::default(),
        },
    )
    .unwrap();
    for (a, b) in stationary.beta.iter().zip(phase.beta.iter()) {
        close(*a, *b, 1e-9, "constant-profile beta");
    }
    assert_cov_close(
        &phase.covariance_beta,
        &stationary.covariance_beta,
        "profile",
    );
    close(
        stationary.residual_rms,
        phase.residual_rms,
        1e-9,
        "profile RMS",
    );
    assert_eq!(phase.jitter_applied_v2, 0.0);
}

// Degenerate kernel ([1.0] makes C_N the identity): stationary must
// reproduce the identity mode in beta and covariance. This is the sharpest
// check that the v0 scale is absorbed exactly once.
#[test]
fn degenerate_stationary_matches_identity() {
    let case = case("stationary_ar1");
    let times: Vec<f64> = (0..case.samples)
        .map(|index| case.t_start + index as f64 * case.dt)
        .collect();
    let model = HarmonicSignalModel {
        fit_harmonics: case.fit_harmonics.clone(),
        output_harmonics: case.output_harmonics.clone(),
        envelope_degree: 0,
    };
    let kernel = CorrelationKernel {
        lags: vec![1.0],
        lag_step_s: case.dt,
    };
    let stationary = estimate_joint(
        &times,
        &case.signal,
        case.f_ref,
        case.phase_rad,
        1.0 / case.dt,
        &JointHarmonicSettings {
            model: model.clone(),
            noise: NoiseModel {
                mode: NoiseMode::StationaryCorrelated,
                reference_variance_v2: 2.0,
                variance_bins: None,
                correlation: Some(kernel),
            },
            tolerances: JointSolverTolerances::default(),
        },
    )
    .unwrap();
    let identity = estimate_joint(
        &times,
        &case.signal,
        case.f_ref,
        case.phase_rad,
        1.0 / case.dt,
        &JointHarmonicSettings {
            model,
            noise: NoiseModel {
                mode: NoiseMode::Identity,
                reference_variance_v2: 2.0,
                variance_bins: None,
                correlation: None,
            },
            tolerances: JointSolverTolerances::default(),
        },
    )
    .unwrap();
    for (a, b) in stationary.beta.iter().zip(identity.beta.iter()) {
        close(*a, *b, 1e-9, "degenerate beta");
    }
    assert_cov_close(
        &stationary.covariance_beta,
        &identity.covariance_beta,
        "degenerate",
    );
}

// Geometry binding: a kernel recorded at another sample interval is
// rejected rather than silently re-tapered; oversized windows fail before
// the quadratic factor is allocated.
#[test]
fn kernel_geometry_binding() {
    let case = case("stationary_ar1");
    let mut settings = settings_for(&case);
    let times: Vec<f64> = (0..case.samples)
        .map(|index| case.t_start + index as f64 * case.dt)
        .collect();
    if let NoiseModel {
        correlation: Some(kernel),
        ..
    } = &mut settings.noise
    {
        kernel.lag_step_s = 2.0 * case.dt;
    }
    assert_eq!(
        estimate_joint(
            &times,
            &case.signal,
            case.f_ref,
            case.phase_rad,
            1.0 / case.dt,
            &settings
        )
        .unwrap_err()
        .code(),
        "model_binding_mismatch"
    );
    // Oversized window: the cap fires before Toeplitz allocation, so a bare
    // 4096-row design (cheap) suffices to exercise it.
    let big_times: Vec<f64> = (0..4096).map(|i| i as f64 * 1e-5).collect();
    let big_signal = vec![0.0; 4096];
    let big_settings = JointHarmonicSettings {
        model: HarmonicSignalModel {
            fit_harmonics: (1..=12).collect(),
            output_harmonics: (1..=6).collect(),
            envelope_degree: 0,
        },
        noise: NoiseModel {
            mode: NoiseMode::StationaryCorrelated,
            reference_variance_v2: 1.0,
            variance_bins: None,
            correlation: Some(CorrelationKernel {
                lags: vec![1.0, 0.5],
                lag_step_s: 1e-5,
            }),
        },
        tolerances: JointSolverTolerances::default(),
    };
    assert_eq!(
        estimate_joint(
            &big_times,
            &big_signal,
            1000.0,
            0.0,
            100_000.0,
            &big_settings
        )
        .unwrap_err()
        .code(),
        "resource_limit_exceeded"
    );
}

// Triangular-solve and Toeplitz anchors with hand-computed values.
#[test]
fn triangular_and_toeplitz_anchors() {
    // L = [[2,0],[1,3]], B = [[4,5],[7,8]]: X = [[2,2.5],[2,1.8333]].
    let lower = DMatrix::from_row_slice(2, 2, &[2.0, 0.0, 1.0, 3.0]);
    let rhs = DMatrix::from_row_slice(2, 2, &[4.0, 5.0, 7.0, 8.0]);
    let solved = forward_substitute(&lower, &rhs).unwrap();
    close(solved[(0, 0)], 2.0, 1e-12, "forward (0,0)");
    close(solved[(0, 1)], 2.5, 1e-12, "forward (0,1)");
    close(solved[(1, 0)], (7.0 - 2.0) / 3.0, 1e-12, "forward (1,0)");
    close(solved[(1, 1)], (8.0 - 2.5) / 3.0, 1e-12, "forward (1,1)");
    // Zero diagonal fails instead of dividing.
    let singular = DMatrix::from_row_slice(2, 2, &[0.0, 0.0, 1.0, 1.0]);
    assert_eq!(
        forward_substitute(&singular, &rhs).unwrap_err().code(),
        "covariance_not_spd"
    );
    // Toeplitz pads zeros beyond the recorded support.
    let toeplitz = toeplitz_from_lags(&[1.0, 0.5], 4);
    assert_eq!(toeplitz[(0, 2)], 0.0);
    assert_eq!(toeplitz[(0, 1)], 0.5);
    assert_eq!(toeplitz[(3, 3)], 1.0);
    // Identity factors cleanly with no jitter recorded.
    let (factor, applied) = cholesky_factor(&DMatrix::identity(3, 3), 0.0).unwrap();
    assert_eq!(applied, 0.0);
    for i in 0..3 {
        close(factor[(i, i)], 1.0, 1e-12, "cholesky diag");
    }
}
