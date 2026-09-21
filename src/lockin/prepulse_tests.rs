//! Pre-pulse derivation tests (Issue #258, AT-01..03, AT-06).

use super::*;
use crate::config::{
    GlsCovarianceOutput, GlsFailurePolicy, GlsNoiseMode, JointHarmonicGlsConfig, Window,
};
use crate::lockin::joint::NoiseModelSource;
use crate::utils::time_axis::TimeAxisRef;
use std::f64::consts::PI;

const DT: f64 = 1.0e-5;
const F_REF: f64 = 1_000.0;
const PHASE: f64 = 0.3;

/// Deterministic xorshift64 RNG plus Box-Muller normals (test-only).
struct TestRng(u64);

impl TestRng {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn next_normal(&mut self) -> f64 {
        let u1 = (self.next_u64() as f64) / (u64::MAX as f64);
        let u2 = (self.next_u64() as f64) / (u64::MAX as f64);
        (-2.0 * u1.max(1e-12).ln()).sqrt() * (2.0 * PI * u2).cos()
    }
}

fn gls_config(noise_mode: GlsNoiseMode) -> JointHarmonicGlsConfig {
    JointHarmonicGlsConfig {
        fit_harmonics: (1..=12).collect(),
        output_harmonics: (1..=6).collect(),
        envelope_degree: 0,
        noise_mode,
        covariance_output: GlsCovarianceOutput::Diagonal,
        failure_policy: GlsFailurePolicy::Error,
        calibration_source: crate::config::GlsCalibrationSource::Prepulse,
        calibrations: Vec::new(),
        solver_tolerances: crate::config::GlsSolverTolerances::default(),
        scs_adequacy: crate::config::GlsScsAdequacy::default(),
    }
}

/// Stationary tone-plus-noise trace: the nuisance fit removes the tone and
/// leaves stationary residuals, so derivation succeeds.
fn stationary_trace(samples: usize, seed: u64, noise_scale: f64) -> (Vec<f64>, Vec<f64>) {
    let mut rng = TestRng(seed);
    let time: Vec<f64> = (0..samples).map(|index| index as f64 * DT).collect();
    let signal: Vec<f64> = time
        .iter()
        .map(|t| {
            (2.0 * PI * F_REF * t + PHASE).sin()
                + 0.2 * (2.0 * PI * 3.0 * F_REF * t + PHASE).sin()
                + noise_scale * rng.next_normal()
        })
        .collect();
    (time, signal)
}

/// Correlation-shifted noise: white in the head half, AR(1) in the tail
/// half. The SCS adequacy split must refuse derivation because the held-out
/// lag structure disagrees with training (AT-03 adequacy case).
fn correlation_shift_trace(samples: usize, seed: u64) -> (Vec<f64>, Vec<f64>) {
    let mut rng = TestRng(seed);
    let time: Vec<f64> = (0..samples).map(|index| index as f64 * DT).collect();
    let mut ar = 0.0;
    let signal: Vec<f64> = time
        .iter()
        .enumerate()
        .map(|(index, t)| {
            let tone = (2.0 * PI * F_REF * t + PHASE).sin();
            let noise = if index < samples / 2 {
                rng.next_normal()
            } else {
                ar = 0.9 * ar + rng.next_normal();
                ar
            };
            tone + 0.05 * noise
        })
        .collect();
    (time, signal)
}

fn full_window(samples: usize) -> Window {
    Window {
        start: 0.0,
        end: (samples - 1) as f64 * DT,
    }
}

fn is_lower_hex_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[test]
fn identity_derivation_succeeds_and_loads() {
    let samples = 30_000usize;
    let (time, signal) = stationary_trace(samples, 7, 0.05);
    let gls = gls_config(GlsNoiseMode::Identity);
    let derivation = derive_prepulse(
        full_window(samples),
        TimeAxisRef::Explicit(&time),
        &[3],
        &[signal.as_slice()],
        F_REF,
        PHASE,
        DT,
        &gls,
        "acquisition-test",
    )
    .unwrap();
    assert!(
        is_lower_hex_digest(&derivation.digest),
        "digest must be SHA-256 lowercase hex, got {:?}",
        derivation.digest
    );
    let (model, binding) = derivation.source.load(3, GlsNoiseMode::Identity).unwrap();
    assert!(model.reference_variance_v2 > 0.0);
    assert!(model.variance_bins.is_none());
    assert!(model.correlation.is_none());
    assert_eq!(binding.channel, 3);
    assert_eq!(binding.path, "prepulse:ch3");
    assert!(is_lower_hex_digest(&binding.sha256));
    assert!(binding.bytes.is_none());
    assert!(derivation.source.load(9, GlsNoiseMode::Identity).is_err());
}

#[test]
fn derivation_is_deterministic_for_identical_inputs() {
    let samples = 30_000usize;
    let (time, signal) = stationary_trace(samples, 11, 0.05);
    let gls = gls_config(GlsNoiseMode::Identity);
    let run = || {
        derive_prepulse(
            full_window(samples),
            TimeAxisRef::Explicit(&time),
            &[3],
            &[signal.as_slice()],
            F_REF,
            PHASE,
            DT,
            &gls,
            "acquisition-test",
        )
        .unwrap()
    };
    let first = run();
    let second = run();
    assert_eq!(first.digest, second.digest);
    let (first_model, _) = first.source.load(3, GlsNoiseMode::Identity).unwrap();
    let (second_model, _) = second.source.load(3, GlsNoiseMode::Identity).unwrap();
    assert_eq!(
        first_model.reference_variance_v2,
        second_model.reference_variance_v2
    );
}

#[test]
fn digest_distinguishes_interval_and_estimator_inputs() {
    let samples = 30_000usize;
    let (time, signal) = stationary_trace(samples, 13, 0.05);
    let gls = gls_config(GlsNoiseMode::Identity);
    let base = derive_prepulse(
        full_window(samples),
        TimeAxisRef::Explicit(&time),
        &[3],
        &[signal.as_slice()],
        F_REF,
        PHASE,
        DT,
        &gls,
        "acquisition-test",
    )
    .unwrap()
    .digest;
    // Estimator params feed the digest.
    let phase_diagonal = derive_prepulse(
        full_window(samples),
        TimeAxisRef::Explicit(&time),
        &[3],
        &[signal.as_slice()],
        F_REF,
        PHASE,
        DT,
        &gls_config(GlsNoiseMode::PhaseDiagonal),
        "acquisition-test",
    )
    .unwrap()
    .digest;
    assert_ne!(base, phase_diagonal);
    // The interval spec feeds the digest.
    let shifted = derive_prepulse(
        Window {
            start: DT,
            end: (samples - 1) as f64 * DT,
        },
        TimeAxisRef::Explicit(&time),
        &[3],
        &[signal.as_slice()],
        F_REF,
        PHASE,
        DT,
        &gls,
        "acquisition-test",
    )
    .unwrap()
    .digest;
    assert_ne!(base, shifted);
    // The acquisition digest feeds the digest.
    let other_source = derive_prepulse(
        full_window(samples),
        TimeAxisRef::Explicit(&time),
        &[3],
        &[signal.as_slice()],
        F_REF,
        PHASE,
        DT,
        &gls,
        "other-acquisition",
    )
    .unwrap()
    .digest;
    assert_ne!(base, other_source);
}

#[test]
fn explicit_frozen_tolerances_match_implicit_defaults_end_to_end() {
    let samples = 30_000usize;
    let (time, signal) = stationary_trace(samples, 7, 0.05);
    let default_gls = gls_config(GlsNoiseMode::Identity);
    let mut frozen_gls = default_gls.clone();
    frozen_gls.solver_tolerances = crate::config::GlsSolverTolerances {
        rank_tol: 1e-10,
        max_condition: 1e8,
        max_noise_condition: 1e10,
        max_jitter_v2: 0.0,
    };
    frozen_gls.scs_adequacy = crate::config::GlsScsAdequacy {
        lags: 4,
        min_pairs_per_cell: 10,
        max_phase_spread: 0.2,
        max_reserved_shift: 0.2,
    };
    let derive = |gls: &JointHarmonicGlsConfig| {
        derive_prepulse(
            full_window(samples),
            TimeAxisRef::Explicit(&time),
            &[3],
            &[signal.as_slice()],
            F_REF,
            PHASE,
            DT,
            gls,
            "acquisition-test",
        )
        .unwrap()
    };
    let from_default = derive(&default_gls);
    let from_frozen = derive(&frozen_gls);
    assert_eq!(from_default.digest, from_frozen.digest);
    let (default_model, default_binding) =
        from_default.source.load(3, GlsNoiseMode::Identity).unwrap();
    let (frozen_model, frozen_binding) =
        from_frozen.source.load(3, GlsNoiseMode::Identity).unwrap();
    assert_eq!(default_model, frozen_model);
    assert_eq!(default_binding, frozen_binding);
}

#[test]
fn nondefault_tolerances_feed_the_derivation_digest() {
    let samples = 30_000usize;
    let (time, signal) = stationary_trace(samples, 13, 0.05);
    let base_gls = gls_config(GlsNoiseMode::Identity);
    let derive = |gls: &JointHarmonicGlsConfig| {
        derive_prepulse(
            full_window(samples),
            TimeAxisRef::Explicit(&time),
            &[3],
            &[signal.as_slice()],
            F_REF,
            PHASE,
            DT,
            gls,
            "acquisition-test",
        )
        .unwrap()
        .digest
    };
    let base = derive(&base_gls);
    // Solver tolerances steer the nuisance fit, so they feed the digest.
    let mut solver_override = base_gls.clone();
    solver_override.solver_tolerances.rank_tol = 1e-9;
    assert_ne!(base, derive(&solver_override));
    // The SCS adequacy gate feeds the digest.
    let mut adequacy_override = base_gls.clone();
    adequacy_override.scs_adequacy.max_phase_spread = 0.5;
    assert_ne!(base, derive(&adequacy_override));
}

#[test]
fn empty_interval_is_a_named_error() {
    let samples = 30_000usize;
    let (time, signal) = stationary_trace(samples, 17, 0.05);
    let gls = gls_config(GlsNoiseMode::Identity);
    let error = derive_prepulse(
        Window {
            start: 5.0,
            end: 6.0,
        },
        TimeAxisRef::Explicit(&time),
        &[3],
        &[signal.as_slice()],
        F_REF,
        PHASE,
        DT,
        &gls,
        "acquisition-test",
    )
    .err()
    .unwrap();
    assert!(
        format!("{error:#}").contains("holds no samples"),
        "{error:#}"
    );
}

#[test]
fn short_interval_trips_the_training_block_gate() {
    let samples = 30_000usize;
    let (time, signal) = stationary_trace(samples, 19, 0.05);
    let gls = gls_config(GlsNoiseMode::Identity);
    // 100 samples cannot supply two 64-sample training blocks.
    let error = derive_prepulse(
        Window {
            start: 0.0,
            end: 99.0 * DT,
        },
        TimeAxisRef::Explicit(&time),
        &[3],
        &[signal.as_slice()],
        F_REF,
        PHASE,
        DT,
        &gls,
        "acquisition-test",
    )
    .err()
    .unwrap();
    assert!(
        format!("{error:#}").contains("insufficient_calibration"),
        "{error:#}"
    );
}

#[test]
fn non_finite_signal_is_a_named_error() {
    let samples = 30_000usize;
    let (time, mut signal) = stationary_trace(samples, 23, 0.05);
    signal[500] = f64::NAN;
    let gls = gls_config(GlsNoiseMode::Identity);
    let error = derive_prepulse(
        full_window(samples),
        TimeAxisRef::Explicit(&time),
        &[3],
        &[signal.as_slice()],
        F_REF,
        PHASE,
        DT,
        &gls,
        "acquisition-test",
    )
    .err()
    .unwrap();
    assert!(
        format!("{error:#}").contains("non_finite_input"),
        "{error:#}"
    );
}

#[test]
fn inverted_window_is_rejected() {
    let samples = 30_000usize;
    let (time, signal) = stationary_trace(samples, 29, 0.05);
    let gls = gls_config(GlsNoiseMode::Identity);
    let error = derive_prepulse(
        Window {
            start: 0.2,
            end: 0.1,
        },
        TimeAxisRef::Explicit(&time),
        &[3],
        &[signal.as_slice()],
        F_REF,
        PHASE,
        DT,
        &gls,
        "acquisition-test",
    )
    .err()
    .unwrap();
    assert!(format!("{error:#}").contains("non-empty"), "{error:#}");
}

#[test]
fn correlation_shift_between_halves_fails_adequacy() {
    let samples = 30_000usize;
    let (time, signal) = correlation_shift_trace(samples, 31);
    let gls = gls_config(GlsNoiseMode::Identity);
    let error = derive_prepulse(
        full_window(samples),
        TimeAxisRef::Explicit(&time),
        &[3],
        &[signal.as_slice()],
        F_REF,
        PHASE,
        DT,
        &gls,
        "acquisition-test",
    )
    .err()
    .unwrap();
    assert!(format!("{error:#}").contains("adequacy"), "{error:#}");
}

#[test]
fn correlated_modes_derive_tables_identity_does_not() {
    let samples = 30_000usize;
    let (time, signal) = stationary_trace(samples, 37, 0.05);
    for (mode, want_bins, want_correlation) in [
        (GlsNoiseMode::Identity, false, false),
        (GlsNoiseMode::PhaseDiagonal, true, false),
        (GlsNoiseMode::StationaryCorrelated, false, true),
        (GlsNoiseMode::PhaseCorrelated, true, true),
    ] {
        let gls = gls_config(mode);
        let derivation = derive_prepulse(
            full_window(samples),
            TimeAxisRef::Explicit(&time),
            &[3],
            &[signal.as_slice()],
            F_REF,
            PHASE,
            DT,
            &gls,
            "acquisition-test",
        )
        .unwrap();
        let (model, _) = derivation.source.load(3, mode).unwrap();
        assert_eq!(model.variance_bins.is_some(), want_bins, "{mode:?}");
        assert_eq!(model.correlation.is_some(), want_correlation, "{mode:?}");
    }
}

#[test]
fn loaded_model_rejects_a_foreign_noise_mode() {
    let samples = 30_000usize;
    let (time, signal) = stationary_trace(samples, 41, 0.05);
    let derivation = derive_prepulse(
        full_window(samples),
        TimeAxisRef::Explicit(&time),
        &[3],
        &[signal.as_slice()],
        F_REF,
        PHASE,
        DT,
        &gls_config(GlsNoiseMode::Identity),
        "acquisition-test",
    )
    .unwrap();
    assert!(
        derivation
            .source
            .load(3, GlsNoiseMode::PhaseDiagonal)
            .is_err()
    );
}

#[test]
fn acquisition_digest_falls_back_to_the_absent_sentinel() {
    use crate::test_support::test_config;
    let dir = std::env::temp_dir().join(format!("pmoke-prepulse-absent-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut cfg = test_config(vec![1], vec![3]);
    cfg.set_artifact_root(dir.clone());
    assert_eq!(acquisition_digest_for(&cfg), PREPULSE_ABSENT_ACQUISITION);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn acquisition_digest_hashes_the_manifest_when_present() {
    use crate::test_support::test_config;
    let dir = std::env::temp_dir().join(format!("pmoke-prepulse-present-{}", std::process::id()));
    let mut cfg = test_config(vec![1], vec![3]);
    cfg.set_artifact_root(dir.clone());
    let manifest = cfg.resolver().acquisition_manifest();
    std::fs::create_dir_all(manifest.parent().unwrap()).unwrap();
    std::fs::write(&manifest, "version = 7\n").unwrap();
    let digest = acquisition_digest_for(&cfg);
    assert_eq!(
        digest,
        crate::utils::checksum::file_sha256(&manifest).unwrap()
    );
    assert!(is_lower_hex_digest(&digest));
    std::fs::remove_dir_all(&dir).unwrap();
}

/// End-to-end pre-pulse LI fixture (AT-01): a 12 000-sample trace with the
/// derivation window ahead of the analysis region.
fn prepulse_run_fixture(seed: u64) -> (Vec<f64>, Vec<Vec<f64>>) {
    let samples = 12_000usize;
    let time: Vec<f64> = (0..samples)
        .map(|index| -0.11 + index as f64 * DT)
        .collect();
    let mut rng = TestRng(seed);
    let sensor: Vec<f64> = time.iter().map(|t| 0.001 * t).collect();
    let reference: Vec<f64> = time.iter().map(|t| (2.0 * PI * F_REF * t).sin()).collect();
    let signal: Vec<f64> = time
        .iter()
        .map(|t| {
            (2.0 * PI * F_REF * t + PHASE).sin()
                + 0.2 * (2.0 * PI * 3.0 * F_REF * t + PHASE).sin()
                + 0.05 * rng.next_normal()
        })
        .collect();
    (time, vec![sensor, reference, signal])
}

fn prepulse_run_config(dir: &std::path::Path) -> crate::config::Config {
    use crate::config::{LockinEstimator, LockinWindow};
    use crate::test_support::test_config;
    let mut cfg = test_config(vec![1], vec![3]);
    cfg.roles.reference_ch = 2;
    cfg.source_path = dir.join("config.toml");
    cfg.set_artifact_root(dir.to_path_buf());
    cfg.lockin.workers = 1;
    cfg.lockin.stride_samples = 10;
    cfg.lockin.lpf_half_window_cycles = 1.0;
    cfg.lockin.window = LockinWindow::legacy_boxcar(1.0);
    cfg.lockin.estimator = LockinEstimator::JointHarmonicGls(gls_config(GlsNoiseMode::Identity));
    cfg.pulse.bg_window_before = Window {
        start: -0.10,
        end: -0.01,
    };
    cfg
}

#[test]
fn prepulse_end_to_end_through_run_li() {
    use crate::lockin::run_li;
    // AT-01: prepulse with empty calibrations starts and yields LI output,
    // with the derivation digest on the provenance and no file artifact.
    let dir = std::env::temp_dir().join(format!("pmoke-prepulse-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = prepulse_run_config(&dir);
    let (time, data) = prepulse_run_fixture(101);
    let (_, _, _, li_results, _, provenance) =
        run_li(&cfg, TimeAxisRef::Explicit(&time), &data).unwrap();
    assert_eq!(li_results.len(), 1);
    assert_eq!(li_results[0].len(), 12);
    let encoded = serde_json::to_value(&provenance).unwrap();
    assert_eq!(encoded["kind"], "joint_harmonic_gls");
    let digest = encoded["prepulse_calibration_digest"]
        .as_str()
        .expect("provenance must record the pre-pulse derivation digest");
    assert!(is_lower_hex_digest(digest));
    // No file artifact is retained for derived models (bytes-free binding).
    assert!(
        !cfg.paths().lockin_calibration_json(3).is_file(),
        "pre-pulse derivation must not retain a calibration file"
    );
    let snapshot = std::fs::read_to_string(cfg.paths().lockin_estimator_json(3)).unwrap();
    let snapshot_value: serde_json::Value = serde_json::from_str(&snapshot).unwrap();
    assert_eq!(snapshot_value["model"]["path"], "prepulse:ch3");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn prepulse_reruns_agree_on_digest_and_output() {
    use crate::lockin::run_li;
    // AT-02: two runs over identical inputs agree on the digest and the LI
    // output content (provenance carries no paths or timestamps).
    let fixture = prepulse_run_fixture(103);
    let mut digests = Vec::new();
    let mut outputs = Vec::new();
    for tag in ["rerun-a", "rerun-b"] {
        let dir = std::env::temp_dir().join(format!("pmoke-prepulse-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = prepulse_run_config(&dir);
        let (_, _, _, li_results, _, provenance) =
            run_li(&cfg, TimeAxisRef::Explicit(&fixture.0), &fixture.1).unwrap();
        let encoded = serde_json::to_value(&provenance).unwrap();
        digests.push(
            encoded["prepulse_calibration_digest"]
                .as_str()
                .expect("digest must be recorded")
                .to_string(),
        );
        outputs.push(li_results);
        std::fs::remove_dir_all(&dir).unwrap();
    }
    assert_eq!(digests[0], digests[1]);
    assert_eq!(outputs[0], outputs[1]);
}

#[test]
fn prepulse_derivation_cost_is_a_fraction_of_the_li_run() {
    use crate::lockin::run_li;
    // NFR-03 probe (provisional +10% target): the one-shot derivation must
    // stay far below the LI run it feeds. The loose bound below guards
    // against pathological regressions; the exact release ratio is measured
    // and recorded at validation time.
    let dir = std::env::temp_dir().join(format!("pmoke-prepulse-perf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = prepulse_run_config(&dir);
    let (time, data) = prepulse_run_fixture(107);
    let gls = gls_config(GlsNoiseMode::Identity);
    let signal: &[&[f64]] = &[data[2].as_slice()];
    let start = std::time::Instant::now();
    let derivation = derive_prepulse(
        cfg.pulse.bg_window_before,
        TimeAxisRef::Explicit(&time),
        &[3],
        signal,
        F_REF,
        PHASE,
        DT,
        &gls,
        &acquisition_digest_for(&cfg),
    )
    .unwrap();
    let derivation_elapsed = start.elapsed();
    drop(derivation);
    let start = std::time::Instant::now();
    run_li(&cfg, TimeAxisRef::Explicit(&time), &data).unwrap();
    let run_elapsed = start.elapsed();
    eprintln!("pre-pulse derivation {derivation_elapsed:?} vs LI run {run_elapsed:?}");
    assert!(
        derivation_elapsed < run_elapsed,
        "derivation {derivation_elapsed:?} must cost less than the LI run {run_elapsed:?}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn truncation_warns_and_records_requested_and_effective() {
    // Issue #269 FR-01/FR-02: a requested window extending 5 ms before the
    // recorded timebase truncates to the recorded samples (with a warning),
    // and both the requested window and the effective range are recorded.
    let samples = 30_000usize;
    let (time, signal) = stationary_trace(samples, 43, 0.05);
    let gls = gls_config(GlsNoiseMode::Identity);
    let requested = Window {
        start: -0.005,
        end: (samples - 1) as f64 * DT,
    };
    let derivation = derive_prepulse(
        requested,
        TimeAxisRef::Explicit(&time),
        &[3],
        &[signal.as_slice()],
        F_REF,
        PHASE,
        DT,
        &gls,
        "acquisition-test",
    )
    .unwrap();
    assert_eq!(derivation.requested_window.start, -0.005);
    assert_eq!(derivation.requested_window.end, (samples - 1) as f64 * DT);
    assert_eq!(derivation.effective_start, 0);
    assert_eq!(derivation.effective_end, samples);
}

#[test]
fn phase_correlated_scs_refusal_is_mode_independent() {
    // Issue #269 FR-04: the SCS adequacy gate is mandatory even with
    // noise_mode = "phase_correlated" (shot13 ch2 spread 0.506 analogue:
    // refusal is the correct behavior, never a relaxed threshold).
    let samples = 30_000usize;
    let (time, signal) = correlation_shift_trace(samples, 47);
    let gls = gls_config(GlsNoiseMode::PhaseCorrelated);
    let error = derive_prepulse(
        full_window(samples),
        TimeAxisRef::Explicit(&time),
        &[3],
        &[signal.as_slice()],
        F_REF,
        PHASE,
        DT,
        &gls,
        "acquisition-test",
    )
    .err()
    .unwrap();
    assert!(format!("{error:#}").contains("adequacy"), "{error:#}");
}
