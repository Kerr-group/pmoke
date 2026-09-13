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
    NoiseMode, NoiseModel, TIMEBASE_RELATIVE_TOLERANCE, cholesky_factor, estimate_joint,
    forward_substitute, interpolate_variance, toeplitz_from_lags, validate_correlation_kernel,
    validate_noise_model, validate_timebase,
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
    // Unneeded jitter is never applied: a PD kernel with a positive ceiling
    // records zero and reproduces the no-jitter estimate bit-for-bit.
    let mut generous = settings.clone();
    if let NoiseModel {
        correlation: Some(kernel),
        ..
    } = &mut generous.noise
    {
        *kernel = CorrelationKernel {
            lags: vec![1.0, 0.5],
            lag_step_s: 1e-5,
        };
    }
    generous.tolerances.max_jitter_v2 = 0.1;
    let unjittered = estimate_joint(&times, &signal, 1000.0, 0.0, 100_000.0, &generous).unwrap();
    assert_eq!(unjittered.jitter_applied_v2, 0.0);
    generous.tolerances.max_jitter_v2 = 0.0;
    let plain = estimate_joint(&times, &signal, 1000.0, 0.0, 100_000.0, &generous).unwrap();
    assert_eq!(unjittered.beta, plain.beta);
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

// TOL-07 on the executed covariance (M3 review F1): the same ill variance
// table must reject in both phase modes, and ill-conditioned realized
// covariances must reject in both correlated modes even when Cholesky
// succeeds and the whitened design is well conditioned.
#[test]
fn realized_noise_condition_gate() {
    // Same phase-variance table (condition 1e12) under a 1e10 cap: diagonal
    // rejects, and phase-correlated with an identity kernel must agree.
    let samples = 256;
    let rate = 64_000.0;
    let dt = 1.0 / rate;
    let times: Vec<f64> = (0..samples).map(|i| (i as f64 + 0.5) * dt).collect();
    let signal: Vec<f64> = times
        .iter()
        .map(|t| (std::f64::consts::TAU * 1000.0 * t).sin())
        .collect();
    let mut bins = vec![1.0; 64];
    bins[0] = 1e-12;
    for (mode, lags) in [
        (NoiseMode::PhaseDiagonal, None),
        (
            NoiseMode::PhaseCorrelated,
            Some(CorrelationKernel {
                lags: vec![1.0],
                lag_step_s: dt,
            }),
        ),
    ] {
        let settings = JointHarmonicSettings {
            model: HarmonicSignalModel {
                fit_harmonics: (1..=12).collect(),
                output_harmonics: (1..=6).collect(),
                envelope_degree: 0,
            },
            noise: NoiseModel {
                mode,
                reference_variance_v2: 1.0,
                variance_bins: Some(bins.clone()),
                correlation: lags.clone(),
            },
            tolerances: JointSolverTolerances::default(),
        };
        assert_eq!(
            estimate_joint(&times, &signal, 1000.0, 0.0, rate, &settings)
                .unwrap_err()
                .code(),
            "ill_conditioned_noise_model",
            "mode={mode:?}"
        );
    }
    // Finite SPD tridiagonal stationary covariance with condition ~2e12:
    // Cholesky succeeds but the TOL-07 cap (1e10) must refuse it.
    let samples = 32;
    let dt = 1e-5;
    let rho = (1.0 - 1e-12) / (2.0 * (std::f64::consts::PI / 33.0).cos());
    let times: Vec<f64> = (0..samples).map(|i| i as f64 * dt).collect();
    let f_ref = 1.0 / (samples as f64 * dt);
    let signal: Vec<f64> = times
        .iter()
        .map(|t| (std::f64::consts::TAU * f_ref * t).sin())
        .collect();
    let stationary = JointHarmonicSettings {
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
                lags: vec![1.0, rho],
                lag_step_s: dt,
            }),
        },
        tolerances: JointSolverTolerances::default(),
    };
    assert_eq!(
        estimate_joint(&times, &signal, f_ref, 0.0, 1.0 / dt, &stationary)
            .unwrap_err()
            .code(),
        "ill_conditioned_noise_model"
    );
    // Full-SCS isolation: a perfectly flat variance table (condition 1)
    // with the same ill correlation kernel must still refuse, proving the
    // gate measures R = S C S rather than re-checking the table.
    let scs = JointHarmonicSettings {
        model: stationary.model.clone(),
        noise: NoiseModel {
            mode: NoiseMode::PhaseCorrelated,
            reference_variance_v2: 1.0,
            variance_bins: Some(vec![1.0; 8]),
            correlation: stationary.noise.correlation.clone(),
        },
        tolerances: JointSolverTolerances::default(),
    };
    assert_eq!(
        estimate_joint(&times, &signal, f_ref, 0.0, 1.0 / dt, &scs)
            .unwrap_err()
            .code(),
        "ill_conditioned_noise_model"
    );
}

// Single effective-interval convention (M3 review F2): the validator
// returns the first sample interval, correlated inference binds the kernel
// against that same value, so a kernel built from the returned interval
// always binds on an accepted, slightly nonuniform grid.
#[test]
fn validator_returned_interval_binds_correlated_inference() {
    let samples = 64;
    let dt = 1e-5;
    let rate = 1.0 / dt;
    let clean: Vec<f64> = (0..samples).map(|i| i as f64 * dt).collect();
    let settings_for = |lag_step_s: f64| JointHarmonicSettings {
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
                lags: vec![1.0],
                lag_step_s,
            }),
        },
        tolerances: JointSolverTolerances::default(),
    };
    let f_ref = rate / samples as f64;
    let signal = vec![0.0; samples];
    // Unperturbed control binds at the nominal interval.
    assert_eq!(validate_timebase(&clean, rate).unwrap(), dt);
    estimate_joint(&clean, &signal, f_ref, 0.0, rate, &settings_for(dt)).unwrap();
    // Accepted wobble on one sample: the validator output round-trips into
    // the kernel binding instead of raising model_binding_mismatch.
    let mut wobbled = clean.clone();
    wobbled[1] += dt * TIMEBASE_RELATIVE_TOLERANCE / 4.0;
    let effective = validate_timebase(&wobbled, rate).unwrap();
    assert_ne!(effective, dt);
    estimate_joint(
        &wobbled,
        &signal,
        f_ref,
        0.0,
        rate,
        &settings_for(effective),
    )
    .unwrap();
}

// Strict monotonicity independent of the uniformity allowance (M3 review
// F8): at large time origins the roundoff allowance can exceed a sample
// step, so non-increasing steps fail in both validation and inference.
#[test]
fn timebase_strict_monotonicity_at_large_origin() {
    let dt = 2.0f64.powi(-30);
    let rate = 2.0f64.powi(30);
    let clean: Vec<f64> = (0..64).map(|i| 1e6 + i as f64 * dt).collect();
    assert!(validate_timebase(&clean, rate).is_ok());
    let mut duplicated = clean.clone();
    duplicated[32] = duplicated[31];
    assert_eq!(
        validate_timebase(&duplicated, rate).unwrap_err().code(),
        "invalid_timebase"
    );
    let mut descending = clean.clone();
    descending[40] = descending[39] - dt;
    assert_eq!(
        validate_timebase(&descending, rate).unwrap_err().code(),
        "invalid_timebase"
    );
    let settings = JointHarmonicSettings {
        model: HarmonicSignalModel {
            fit_harmonics: (1..=6).collect(),
            output_harmonics: (1..=6).collect(),
            envelope_degree: 0,
        },
        noise: NoiseModel {
            mode: NoiseMode::PhaseCorrelated,
            reference_variance_v2: 1.0,
            variance_bins: Some(vec![1.0; 2]),
            correlation: Some(CorrelationKernel {
                lags: vec![1.0],
                lag_step_s: dt,
            }),
        },
        tolerances: JointSolverTolerances::default(),
    };
    assert_eq!(
        estimate_joint(
            &duplicated,
            &vec![0.0; 64],
            rate / 64.0,
            0.0,
            rate,
            &settings
        )
        .unwrap_err()
        .code(),
        "invalid_timebase"
    );
}

// Explicit full-R unequal-variance regression (M3 review positive
// evidence): phase-correlated inference with a heteroscedastic profile and
// a non-identity kernel against an independent SciPy reference that
// factors the full R = S C S and solves by SVD least squares (not the
// Rust QR path). cond(R) ~ 5.2, rank 13.
#[test]
fn explicit_full_r_unequal_variance_matches_scipy() {
    let times: Vec<f64> = vec![
        0.0001237,
        0.0001337,
        0.0001437,
        0.0001537,
        0.0001637,
        0.0001737,
        0.00018370000000000002,
        0.0001937,
        0.00020370000000000002,
        0.0002137,
        0.00022370000000000002,
        0.0002337,
        0.00024370000000000001,
        0.00025370000000000004,
        0.0002637,
        0.0002737,
        0.0002837,
        0.00029370000000000004,
        0.0003037,
        0.0003137,
        0.0003237,
        0.00033370000000000003,
        0.0003437,
        0.0003537,
        0.0003637,
        0.00037370000000000003,
        0.00038370000000000006,
        0.0003937,
        0.0004037,
        0.0004137,
        0.00042370000000000005,
        0.00043369999999999997,
        0.0004437,
        0.0004537,
        0.00046370000000000005,
        0.0004737000000000001,
        0.0004837,
        0.0004937,
        0.0005037,
        0.0005137000000000001,
        0.0005237,
        0.0005337,
        0.0005437,
        0.0005537000000000001,
        0.0005637,
        0.0005737,
        0.0005837,
        0.0005937000000000001,
        0.0006037000000000001,
        0.0006137000000000001,
        0.0006237,
        0.0006337000000000001,
        0.0006437000000000001,
        0.0006537000000000001,
        0.0006637,
        0.0006737000000000001,
        0.0006837000000000001,
        0.0006937000000000001,
        0.0007037,
        0.0007137000000000001,
        0.0007237000000000001,
        0.0007337000000000001,
        0.0007437,
        0.0007537,
        0.0007637000000000001,
        0.0007737000000000001,
        0.0007837000000000001,
        0.0007937,
        0.0008037000000000001,
        0.0008137000000000001,
        0.0008237000000000001,
        0.0008337,
        0.0008437000000000001,
        0.0008537000000000001,
        0.0008637000000000001,
        0.0008737,
        0.0008837000000000001,
        0.0008937000000000001,
        0.0009037000000000001,
        0.0009137,
        0.0009237000000000001,
        0.0009337000000000001,
        0.0009437000000000001,
        0.0009537000000000001,
        0.0009637000000000001,
        0.0009737000000000001,
        0.0009837000000000001,
        0.0009937000000000001,
        0.0010037,
        0.0010137,
        0.0010237,
        0.0010337,
        0.0010437,
        0.0010537,
        0.0010637,
        0.0010737000000000001,
    ];
    let signal: Vec<f64> = vec![
        -1.0466948693971636,
        -0.7394278673543037,
        -0.2762654119071396,
        -0.14147736640867542,
        -1.556184769082292,
        -0.19808972537293643,
        1.2192261283191153,
        0.5653166331298098,
        -0.9367530232042223,
        -1.2147638291330658,
        -0.1925746100878748,
        0.7401175638106116,
        1.2953620821390632,
        -0.4739754248585596,
        -0.3402995923041745,
        -1.3571035158283409,
        -1.9432493464257443,
        -1.7857642505497289,
        -0.7021895931409995,
        -0.4500295373770623,
        -0.6449672010334859,
        0.2660747564097425,
        1.2416334452676443,
        0.9629407646897004,
        -0.8966069905982671,
        -1.4314968750408106,
        -0.8827094731160152,
        0.27855175967915,
        0.4038211621575528,
        0.18188110087151105,
        -0.07498213514838045,
        1.0563714883615827,
        0.9831742397824519,
        -0.8555526139645148,
        -0.4924961932400831,
        1.2377046881662586,
        1.2364661311081042,
        0.08221295212119983,
        -0.7578892555152685,
        -0.9731217642177405,
        0.6335457232852579,
        0.9779668867925589,
        -1.810947873836739,
        1.027890316630814,
        0.8331446472304431,
        -1.20616089699233,
        -0.45524899839672966,
        1.1555202582554818,
        1.398572516806525,
        0.8058037348280813,
        1.3157246274412702,
        -0.9882871103875676,
        -0.9198224386169549,
        -0.7419811472523592,
        -0.39316820355975923,
        -0.16970435541996176,
        -0.9102237269712676,
        -0.06133091614314165,
        -0.13526079047128706,
        -0.4560556727677948,
        0.33532711204180643,
        -0.9507871697108541,
        -1.7918655094326825,
        -1.974770741941264,
        0.11335638805307269,
        -0.6808151058391178,
        -0.5713372839942907,
        -0.17170462522418728,
        -3.3342177133266406,
        -1.7260844933569333,
        0.12055707395449948,
        -2.4332323961730076,
        -1.433009885110794,
        -0.5871596383566261,
        -0.5288793059472499,
        -0.5765775927602841,
        0.46709922068960846,
        -0.014082894355737285,
        -0.0814406759744864,
        -0.6325559694613315,
        -0.07632503009952737,
        -0.3787092129099501,
        -1.1364417621659513,
        0.8215737816595279,
        -0.8438444614478022,
        0.5319836987865516,
        -0.498108112190787,
        -1.2315481018583354,
        -0.5065900880661822,
        0.7482863275976179,
        1.4710045420572646,
        0.7380246423137433,
        -0.3667626123170086,
        -0.22875198825798582,
        -2.0412741147438345,
        -0.1717132169498351,
    ];
    let profile: Vec<f64> = vec![
        0.7294051420988681,
        0.7870854031763387,
        0.8414190210477992,
        0.8903179852490936,
        0.931903136008821,
        0.9645763793045063,
        0.9870821007196626,
        0.9985554180016589,
        0.998555418001659,
        0.9870821007196626,
        0.9645763793045065,
        0.931903136008821,
        0.8903179852490936,
        0.8414190210477993,
        0.7870854031763387,
        0.7294051420988682,
        0.6705948579011318,
        0.6129145968236613,
        0.5585809789522007,
        0.5096820147509064,
        0.468096863991179,
        0.4354236206954935,
        0.41291789928033734,
        0.4014445819983409,
        0.4014445819983409,
        0.4129178992803373,
        0.43542362069549345,
        0.4680968639911789,
        0.5096820147509062,
        0.5585809789522006,
        0.6129145968236612,
        0.6705948579011318,
    ];
    let want_beta: Vec<f64> = vec![
        -0.2807397491946526,
        -0.11990393372976368,
        -0.15058565005559815,
        -0.04981677512351433,
        -0.18041802003018287,
        0.23598715471636067,
        0.04407720071045395,
        -0.07969151725534111,
        0.17684961909680982,
        -0.060437235072655755,
        0.1147613139983737,
        0.2658335535785232,
        0.2699357292724455,
    ];
    let want_cov: Vec<Vec<f64>> = vec![
        vec![
            0.010434980927061703,
            -0.0005789441867298277,
            0.005253618905342094,
            -0.0006174473412959946,
            -0.0009162234065476494,
            0.0009062423362844238,
            -9.429542980899198e-05,
            -0.0002490310352734586,
            0.0006276048193824884,
            -0.00028127188179745313,
            -0.0003185024579963222,
            0.00017224740347419803,
            -5.555252539683234e-05,
        ],
        vec![
            -0.0005789441867298277,
            0.02005944944026951,
            -0.0009249180302029038,
            0.00035622539701948787,
            0.005063790041586193,
            -0.0008576861057373885,
            -0.00024110273327876517,
            0.0005724938213381097,
            -0.00042760780138586645,
            -3.762283958878496e-05,
            0.0005296258605399397,
            -0.00024212086069570676,
            -0.0002257794421629285,
        ],
        vec![
            0.005253618905342094,
            -0.0009249180302029038,
            0.021306070311533816,
            -0.005255805794031947,
            -0.0014929467736017074,
            0.0015388673154397982,
            -0.0003419178114769439,
            -0.0002460401520661428,
            0.0011653913156872918,
            -0.0006526773153514079,
            -0.00043008579866155933,
            0.00028429102022680487,
            -0.0002501847010627054,
        ],
        vec![
            -0.0006174473412959946,
            0.00035622539701948787,
            -0.005255805794031947,
            0.019870870536757415,
            0.0006557188431669836,
            -0.0008503904138780023,
            0.004630926488790597,
            -0.00036329903927297764,
            -0.0009095706442885304,
            0.0008332690836535033,
            -6.640765005850261e-05,
            -8.848666191172203e-05,
            0.0005656283945212679,
        ],
        vec![
            -0.0009162234065476494,
            0.005063790041586193,
            -0.0014929467736017074,
            0.0006557188431669836,
            0.020392397844857704,
            -0.005321840001262488,
            -0.00024329055760503036,
            0.0007816684915480045,
            -0.0007769405892633983,
            0.00010489092711167829,
            0.0007912166286601454,
            -0.00041106917009530526,
            -0.00022983306118908934,
        ],
        vec![
            0.0009062423362844238,
            -0.0008576861057373885,
            0.0015388673154397982,
            -0.0008503904138780023,
            -0.005321840001262488,
            0.019459136439012842,
            -6.471620539305641e-05,
            -0.0004939768800318722,
            0.0047347385043268915,
            -0.0004462022894360868,
            -0.0006637439092649621,
            0.0004551815352488072,
            -6.070590636272807e-05,
        ],
        vec![
            -9.429542980899198e-05,
            -0.00024110273327876517,
            -0.0003419178114769439,
            0.004630926488790597,
            -0.00024329055760503036,
            -6.471620539305641e-05,
            0.019038927793051927,
            -0.004690918472002261,
            -0.0005354154745508896,
            0.000904845661284338,
            -0.0006122420453051885,
            0.0001639888356649796,
            0.0008172472197242939,
        ],
        vec![
            -0.0002490310352734586,
            0.0005724938213381097,
            -0.0002460401520661428,
            -0.00036329903927297764,
            0.0007816684915480045,
            -0.0004939768800318722,
            -0.004690918472002261,
            0.018171811827206515,
            0.00012154673696220822,
            -0.0007099176500217886,
            0.0044264575144142716,
            -0.00040984280480710507,
            -0.0007971290308868952,
        ],
        vec![
            0.0006276048193824884,
            -0.00042760780138586645,
            0.0011653913156872918,
            -0.0009095706442885304,
            -0.0007769405892633983,
            0.0047347385043268915,
            -0.0005354154745508896,
            0.00012154673696220822,
            0.018004978598087532,
            -0.004342872937702774,
            -0.00023657193417198,
            0.0003549196505123625,
            -0.0005020226729436066,
        ],
        vec![
            -0.00028127188179745313,
            -3.762283958878496e-05,
            -0.0006526773153514079,
            0.0008332690836535033,
            0.00010489092711167829,
            -0.0004462022894360868,
            0.000904845661284338,
            -0.0007099176500217886,
            -0.004342872937702774,
            0.016800943564592035,
            -0.0002630399220541903,
            -0.00013900019763729348,
            0.004053392265479464,
        ],
        vec![
            -0.0003185024579963222,
            0.0005296258605399397,
            -0.00043008579866155933,
            -6.640765005850261e-05,
            0.0007912166286601454,
            -0.0006637439092649621,
            -0.0006122420453051885,
            0.0044264575144142716,
            -0.00023657193417198,
            -0.0002630399220541903,
            0.01664409956874972,
            -0.003782030571792279,
            -0.0004704392928648474,
        ],
        vec![
            0.00017224740347419803,
            -0.00024212086069570676,
            0.00028429102022680487,
            -8.848666191172203e-05,
            -0.00041106917009530526,
            0.0004551815352488072,
            0.0001639888356649796,
            -0.00040984280480710507,
            0.0003549196505123625,
            -0.00013900019763729348,
            -0.003782030571792279,
            0.014154072895438963,
            6.464591925230474e-05,
        ],
        vec![
            -5.555252539683234e-05,
            -0.0002257794421629285,
            -0.0002501847010627054,
            0.0005656283945212679,
            -0.00022983306118908934,
            -6.070590636272807e-05,
            0.0008172472197242939,
            -0.0007971290308868952,
            -0.0005020226729436066,
            0.004053392265479464,
            -0.0004704392928648474,
            6.464591925230474e-05,
            0.014415959827644478,
        ],
    ];
    let want_rms = 1.0349270142270763;
    let settings = JointHarmonicSettings {
        model: HarmonicSignalModel {
            fit_harmonics: (1..=6).collect(),
            output_harmonics: (1..=6).collect(),
            envelope_degree: 0,
        },
        noise: NoiseModel {
            mode: NoiseMode::PhaseCorrelated,
            reference_variance_v2: 1.0,
            variance_bins: Some(profile.clone()),
            correlation: Some(CorrelationKernel {
                lags: vec![1.0, 0.2],
                lag_step_s: 1e-5,
            }),
        },
        tolerances: JointSolverTolerances::default(),
    };
    let estimate = estimate_joint(&times, &signal, 4000.0, 0.41, 100_000.0, &settings).unwrap();
    assert_eq!(estimate.rank, 13);
    for (index, (got, want)) in estimate.beta.iter().zip(want_beta.iter()).enumerate() {
        assert!(
            (got - want).abs() <= 1e-12,
            "beta[{index}]: got {got:.6e}, want {want:.6e}"
        );
    }
    assert!(
        (estimate.residual_rms - want_rms).abs() <= 1e-12,
        "residual_rms: got {:.6e}, want {want_rms:.6e}",
        estimate.residual_rms
    );
    let mut numerator = 0.0;
    let mut denominator = 0.0;
    for (grow, wrow) in estimate.covariance_beta.iter().zip(want_cov.iter()) {
        for (got, want) in grow.iter().zip(wrow.iter()) {
            numerator += (got - want).powi(2);
            denominator += want.powi(2);
        }
    }
    assert!(
        (numerator / denominator).sqrt() <= 1e-12,
        "covariance_beta relative error: {numerator:.3e} / {denominator:.3e}"
    );
}
