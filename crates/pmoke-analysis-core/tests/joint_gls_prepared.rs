//! PN-M3 accelerated prepared-plan evidence: the immutable noise plan
//! (`PreparedNoisePlan`) must produce exactly the direct solver's values on
//! noisy, off-grid windows in every mode, and the reused-factor path must
//! match the committed SciPy oracle within the PN-NFR-002 bound
//! `abs(error) <= 1e-8 V + 1e-8 * abs(reference)`.
//!
//! Fixtures: committed joint-GLS oracle cases (CC0-1.0, see
//! `fixtures/joint-gls/manifest.json`); no private data, no hardware.

use pmoke_analysis_core::{
    CorrelationKernel, HarmonicSignalModel, JointHarmonicSettings, JointSolverTolerances,
    MAX_WINDOW_SAMPLES, NoiseMode, NoiseModel, PreparedNoisePlan, estimate_joint,
    estimate_joint_with_plan,
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
    variance_bins: Vec<f64>,
    lags: Vec<f64>,
    lag_step_s: f64,
    signal: Vec<f64>,
    oracle: CorrelatedOracle,
}

#[derive(Deserialize)]
struct CorrelatedOracle {
    beta: Vec<f64>,
    xy: BTreeMap<String, Xy>,
    rank: usize,
    covariance_beta: Vec<Vec<f64>>,
    jitter_applied: f64,
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
}

fn golden() -> CorrelatedGolden {
    serde_json::from_str(include_str!("fixtures/joint-gls/correlated-golden.json")).unwrap()
}

fn correlated_case(name: &str) -> CorrelatedCase {
    golden()
        .cases
        .into_iter()
        .find(|case| case.name == name)
        .unwrap_or_else(|| panic!("missing case {name}"))
}

fn convention_case(name: &str) -> ConventionCase {
    let payload: Conventions =
        serde_json::from_str(include_str!("fixtures/joint-gls/conventions.json")).unwrap();
    payload
        .cases
        .into_iter()
        .find(|case| case.name == name)
        .unwrap_or_else(|| panic!("missing case {name}"))
}

fn correlated_noise(case: &CorrelatedCase) -> NoiseModel {
    let mode = match case.mode.as_str() {
        "stationary" => NoiseMode::StationaryCorrelated,
        "phase_correlated" => NoiseMode::PhaseCorrelated,
        other => panic!("unknown mode {other}"),
    };
    let variance_bins = match mode {
        NoiseMode::PhaseCorrelated => Some(case.variance_bins.clone()),
        _ => None,
    };
    NoiseModel {
        mode,
        reference_variance_v2: case.v0,
        variance_bins,
        correlation: Some(CorrelationKernel {
            lags: case.lags.clone(),
            lag_step_s: case.lag_step_s,
        }),
    }
}

fn harmonic_model(case: &CorrelatedCase) -> HarmonicSignalModel {
    HarmonicSignalModel {
        fit_harmonics: case.fit_harmonics.clone(),
        output_harmonics: case.output_harmonics.clone(),
        envelope_degree: 0,
    }
}

fn case_times(case: &CorrelatedCase) -> Vec<f64> {
    (0..case.samples)
        .map(|index| case.t_start + index as f64 * case.dt)
        .collect()
}

/// AT: prepared-plan and direct XY must agree exactly (same arithmetic), for
/// every mode, on noisy off-grid windows.
#[test]
fn prepared_plan_matches_direct_on_noisy_windows() {
    // Synthetic AR(1)/heteroscedastic windows at debug-friendly size; the
    // committed oracle fixtures are covered by
    // `prepared_path_matches_committed_oracle_on_noisy_cases`.
    let tolerances = JointSolverTolerances::default();
    let samples = 256_usize;
    let dt = 1.0e-5;
    let f_ref = 1_000.0;
    let lags: Vec<f64> = (0..16).map(|lag| 0.65_f64.powi(lag)).collect();
    let noises = [
        NoiseModel {
            mode: NoiseMode::StationaryCorrelated,
            reference_variance_v2: 4.0,
            variance_bins: None,
            correlation: Some(CorrelationKernel {
                lags: lags.clone(),
                lag_step_s: dt,
            }),
        },
        NoiseModel {
            mode: NoiseMode::PhaseCorrelated,
            reference_variance_v2: 4.0,
            variance_bins: Some(vec![1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5]),
            correlation: Some(CorrelationKernel {
                lags: lags.clone(),
                lag_step_s: dt,
            }),
        },
    ];
    let model = HarmonicSignalModel {
        fit_harmonics: (1..=12).collect(),
        output_harmonics: (1..=6).collect(),
        envelope_degree: 0,
    };
    let mut noise_state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut random = move || {
        noise_state ^= noise_state << 13;
        noise_state ^= noise_state >> 7;
        noise_state ^= noise_state << 17;
        (noise_state as f64 / u64::MAX as f64) * 2.0 - 1.0
    };
    let signal: Vec<f64> = (0..samples)
        .map(|index| {
            let time = index as f64 * dt;
            1.5 * (std::f64::consts::TAU * f_ref * time + 0.3).sin()
                + 0.4 * (std::f64::consts::TAU * 2.0 * f_ref * time).cos()
                + 0.08 * random()
        })
        .collect();
    for noise in &noises {
        let plan = PreparedNoisePlan::prepare(noise, samples, tolerances).unwrap();
        for window in 0..3 {
            let shift = 0.37 * dt * window as f64;
            let times: Vec<f64> = (0..samples)
                .map(|index| shift + index as f64 * dt)
                .collect();
            let phase = 0.21 * window as f64;
            let direct = estimate_joint(
                &times,
                &signal,
                f_ref,
                phase,
                1.0 / dt,
                &JointHarmonicSettings {
                    model: model.clone(),
                    noise: noise.clone(),
                    tolerances,
                },
            )
            .unwrap();
            let accelerated = estimate_joint_with_plan(
                &times,
                &signal,
                f_ref,
                phase,
                1.0 / dt,
                &model,
                tolerances,
                &plan,
            )
            .unwrap();
            assert_eq!(
                direct.beta, accelerated.beta,
                "{:?} window {window} beta",
                noise.mode
            );
            assert_eq!(
                direct.xy, accelerated.xy,
                "{:?} window {window} xy",
                noise.mode
            );
            assert_eq!(
                direct.covariance_beta, accelerated.covariance_beta,
                "{:?} window {window} covariance",
                noise.mode
            );
            assert_eq!(direct.rank, accelerated.rank);
            assert_eq!(direct.condition, accelerated.condition);
            assert_eq!(direct.residual_rms, accelerated.residual_rms);
            assert_eq!(direct.jitter_applied_v2, accelerated.jitter_applied_v2);
            assert_eq!(plan.jitter_applied_v2(), direct.jitter_applied_v2);
        }
    }
}

/// AT: identity and phase-diagonal prepared paths agree exactly with the
/// direct path on a noisy fractional-offset convention case.
#[test]
fn prepared_plan_matches_direct_for_uncorrelated_modes() {
    let case = convention_case("mixed_phases_fractional_t0");
    let times: Vec<f64> = (0..case.samples)
        .map(|index| case.t_start + index as f64 * case.dt)
        .collect();
    let model = HarmonicSignalModel {
        fit_harmonics: case.fit_harmonics.clone(),
        output_harmonics: case.output_harmonics.clone(),
        envelope_degree: 0,
    };
    let tolerances = JointSolverTolerances::default();
    let noises = [
        NoiseModel {
            mode: NoiseMode::Identity,
            reference_variance_v2: 2.5,
            variance_bins: None,
            correlation: None,
        },
        NoiseModel {
            mode: NoiseMode::PhaseDiagonal,
            reference_variance_v2: 1.0,
            variance_bins: Some(vec![1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0]),
            correlation: None,
        },
    ];
    for noise in &noises {
        let direct = estimate_joint(
            &times,
            &case.signal,
            case.f_ref,
            case.phase_rad,
            1.0 / case.dt,
            &JointHarmonicSettings {
                model: model.clone(),
                noise: noise.clone(),
                tolerances,
            },
        )
        .unwrap();
        let plan = PreparedNoisePlan::prepare(noise, times.len(), tolerances).unwrap();
        let accelerated = estimate_joint_with_plan(
            &times,
            &case.signal,
            case.f_ref,
            case.phase_rad,
            1.0 / case.dt,
            &model,
            tolerances,
            &plan,
        )
        .unwrap();
        assert_eq!(direct.beta, accelerated.beta, "{:?}", noise.mode);
        assert_eq!(direct.xy, accelerated.xy, "{:?}", noise.mode);
        assert_eq!(plan.mode(), noise.mode);
    }
}

/// AT/NFR-002: the reused-factor path matches the committed SciPy oracle on
/// noisy signals, within the published accelerated-vs-direct bound.
#[test]
fn prepared_path_matches_committed_oracle_on_noisy_cases() {
    for name in ["stationary_ar1", "phase_correlated_hetero"] {
        let case = correlated_case(name);
        let noise = correlated_noise(&case);
        let model = harmonic_model(&case);
        let tolerances = JointSolverTolerances::default();
        let times = case_times(&case);
        let plan = PreparedNoisePlan::prepare(&noise, times.len(), tolerances).unwrap();
        let estimate = estimate_joint_with_plan(
            &times,
            &case.signal,
            case.f_ref,
            case.phase_rad,
            1.0 / case.dt,
            &model,
            tolerances,
            &plan,
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
        let bound = |reference: f64| 1.0e-8 + 1.0e-8 * reference.abs();
        for (got, want) in estimate.beta.iter().zip(case.oracle.beta.iter()) {
            let error = (got - want).abs();
            assert!(
                error <= 1.0e-10 + 1.0e-8 * scale,
                "{name} beta oracle bound: {got} vs {want}"
            );
            assert!(error <= bound(*want), "{name} beta PN-NFR-002 bound");
        }
        for (position, harmonic) in case.output_harmonics.iter().enumerate() {
            let want = case.oracle.xy.get(&harmonic.to_string()).unwrap();
            close(
                estimate.xy[2 * position],
                want.x,
                1.0e-10 + 1.0e-8 * scale,
                &format!("{name} X{harmonic}"),
            );
            close(
                estimate.xy[2 * position + 1],
                want.y,
                1.0e-10 + 1.0e-8 * scale,
                &format!("{name} Y{harmonic}"),
            );
        }
        // Covariance against the oracle within the committed Frobenius gate.
        let mut numerator = 0.0;
        let mut denominator = 0.0;
        let mut entries = 0;
        for (grow, wrow) in estimate
            .covariance_beta
            .iter()
            .zip(case.oracle.covariance_beta.iter())
        {
            for (g, w) in grow.iter().zip(wrow.iter()) {
                numerator += (g - w).powi(2);
                denominator += w.powi(2);
                entries += 1;
            }
        }
        let floor = 1e-24 * entries as f64;
        assert!(
            (numerator / (denominator + floor)).sqrt() <= 1.0e-7,
            "{name}: frob {numerator:.3e} / {denominator:.3e}"
        );
    }
}

/// Plan geometry and component preflight: wrong window length, missing
/// components, and impossible correlated windows fail before any window
/// allocation, exactly like the direct path.
#[test]
fn prepared_plan_preflight_fails_closed() {
    let tolerances = JointSolverTolerances::default();
    assert_eq!(
        PreparedNoisePlan::prepare(
            &NoiseModel {
                mode: NoiseMode::Identity,
                reference_variance_v2: 1.0,
                variance_bins: None,
                correlation: None,
            },
            0,
            tolerances,
        )
        .unwrap_err()
        .code(),
        "dimension_mismatch"
    );
    assert_eq!(
        PreparedNoisePlan::prepare(
            &NoiseModel {
                mode: NoiseMode::PhaseDiagonal,
                reference_variance_v2: 1.0,
                variance_bins: None,
                correlation: None,
            },
            64,
            tolerances,
        )
        .unwrap_err()
        .code(),
        "missing_noise_component"
    );
    let long_kernel = CorrelationKernel {
        lags: vec![1.0, 0.5],
        lag_step_s: 1.0e-5,
    };
    assert_eq!(
        PreparedNoisePlan::prepare(
            &NoiseModel {
                mode: NoiseMode::StationaryCorrelated,
                reference_variance_v2: 1.0,
                variance_bins: None,
                correlation: Some(long_kernel.clone()),
            },
            MAX_WINDOW_SAMPLES + 1,
            tolerances,
        )
        .unwrap_err()
        .code(),
        "resource_limit_exceeded"
    );
    // A plan for one window length refuses another geometry.
    let noise = NoiseModel {
        mode: NoiseMode::StationaryCorrelated,
        reference_variance_v2: 1.0,
        variance_bins: None,
        correlation: Some(long_kernel),
    };
    let plan = PreparedNoisePlan::prepare(&noise, 64, tolerances).unwrap();
    let times: Vec<f64> = (0..65).map(|index| index as f64 * 1.0e-5).collect();
    let signal = vec![0.0; 65];
    let model = HarmonicSignalModel {
        fit_harmonics: (1..=6).collect(),
        output_harmonics: (1..=6).collect(),
        envelope_degree: 0,
    };
    assert_eq!(
        estimate_joint_with_plan(
            &times, &signal, 1_000.0, 0.0, 1.0e5, &model, tolerances, &plan
        )
        .unwrap_err()
        .code(),
        "dimension_mismatch"
    );
    // Non-finite window data is rejected before factorization, as direct.
    let times_ok: Vec<f64> = (0..64).map(|index| index as f64 * 1.0e-5).collect();
    let mut bad = vec![0.0_f64; 64];
    bad[3] = f64::NAN;
    let nan_plan = PreparedNoisePlan::prepare(&noise, 64, tolerances).unwrap();
    assert_eq!(
        estimate_joint_with_plan(
            &times_ok, &bad, 1_000.0, 0.0, 1.0e5, &model, tolerances, &nan_plan
        )
        .unwrap_err()
        .code(),
        "non_finite_input"
    );
}
