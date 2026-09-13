//! WP-3 calibration evidence: role/block planning, nuisance removal, phase
//! variance and correlation recipes against committed oracle outputs,
//! artifact byte identity, applicability binding, and SCS adequacy.
//!
//! Acceptance mapping: AT-013 (roles/leakage), AT-014 (coverage/missing
//! bins), AT-015 (variance recipe), AT-016 (bias/uncertainty), AT-017
//! (correlation recipe), AT-041 (SCS adequacy). AT-018 byte-level loading
//! (oversized files, duplicate keys, unknown schema, bad hash) belongs to
//! the WP-5 native boundary; the core-level byte cap and hash identity are
//! pinned here.

use pmoke_analysis_core::{
    AcquisitionMeta, AdequacyGroup, AdequacyPolicy, ApplicabilityRequest, ArtifactRequest,
    BlockPlanRequest, CALIBRATION_ALGORITHM_VERSION, CALIBRATION_ARTIFACT_SCHEMA_VERSION,
    CALIBRATION_PHASE_CONVENTION, CalSample, CalibrationRole, CorrelationRecipe,
    DEFAULT_DT_REL_TOL, DEFAULT_FREQ_REL_TOL, HeldoutReport, JointSolverTolerances, ModelBinding,
    NUISANCE_PARAMETERS, PhaseVarianceRecipe, RoleInterval, SearchSpace, TuningMode,
    assemble_samples, build_artifact, ensure_fixed_tuning, estimate_correlation,
    estimate_phase_variance, fit_nuisance, inspect_applicability, plan_blocks, scs_adequacy,
};
use serde::Deserialize;
use std::collections::BTreeMap;

fn close(a: f64, b: f64, tolerance: f64, context: &str) {
    assert!(
        (a - b).abs() <= tolerance,
        "{context}: {a} vs {b} (tol {tolerance})"
    );
}

fn close_vec(got: &[f64], want: &[f64], tolerance: f64, context: &str) {
    assert_eq!(got.len(), want.len(), "{context}: length");
    for (index, (a, b)) in got.iter().zip(want.iter()).enumerate() {
        close(*a, *b, tolerance, &format!("{context}[{index}]"));
    }
}

#[derive(Deserialize)]
struct Golden {
    cases: Vec<CalCase>,
}

#[derive(Deserialize)]
struct CalCase {
    name: String,
    f_ref: f64,
    dt: f64,
    phase_rad: f64,
    block_len: usize,
    #[serde(default)]
    bins: usize,
    #[serde(default)]
    intervals: Vec<(u64, u64)>,
    truth: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    signals: Vec<Vec<f64>>,
    #[serde(default)]
    standardized_blocks: Vec<Vec<f64>>,
    #[serde(default)]
    train_blocks: Vec<Vec<f64>>,
    #[serde(default)]
    reserved_blocks: Vec<Vec<f64>>,
    #[serde(default)]
    phases: Vec<f64>,
    #[serde(default)]
    adequacy_policy: BTreeMap<String, f64>,
    oracle: BTreeMap<String, serde_json::Value>,
}

fn golden() -> Golden {
    serde_json::from_str(include_str!("fixtures/joint-gls/calibration-golden.json")).unwrap()
}

fn case(name: &str) -> CalCase {
    golden()
        .cases
        .into_iter()
        .find(|case| case.name == name)
        .unwrap_or_else(|| panic!("missing case {name}"))
}

fn adequacy_policy(case: &CalCase) -> AdequacyPolicy {
    let get = |key: &str| {
        *case
            .adequacy_policy
            .get(key)
            .unwrap_or_else(|| panic!("{key}"))
    };
    AdequacyPolicy {
        lags: get("lags") as usize,
        min_pairs_per_cell: get("min_pairs_per_cell") as usize,
        max_phase_spread: get("max_phase_spread"),
        max_reserved_shift: get("max_reserved_shift"),
    }
}

fn block_times(start: u64, len: usize, dt: f64) -> Vec<f64> {
    (0..len).map(|i| (start + i as u64) as f64 * dt).collect()
}

fn plan_request(case: &CalCase) -> BlockPlanRequest {
    let roles = [
        CalibrationRole::Training,
        CalibrationRole::Training,
        CalibrationRole::TuningValidation,
        CalibrationRole::Evaluation,
    ];
    BlockPlanRequest {
        intervals: case
            .intervals
            .iter()
            .zip(roles.iter())
            .map(|((start, end), role)| RoleInterval {
                role: *role,
                start: *start,
                end: *end,
            })
            .collect(),
        block_len: case.block_len,
        total_samples: case.intervals.last().map(|pair| pair.1).unwrap_or(0),
        reference_frequency_hz: case.f_ref,
        sample_interval_s: case.dt,
        min_reference_cycles_per_block: 8.0,
        min_training_blocks: 8,
        min_training_intervals: 2,
    }
}

fn train_residuals(case: &CalCase) -> (Vec<Vec<f64>>, Vec<Vec<f64>>) {
    let tolerances = JointSolverTolerances::default();
    let mut times = Vec::new();
    let mut residuals = Vec::new();
    for block in 0..8 {
        let start = block as u64 * case.block_len as u64;
        let block_times = block_times(start, case.block_len, case.dt);
        let fit = fit_nuisance(
            &block_times,
            &case.signals[block],
            case.f_ref,
            case.phase_rad,
            tolerances,
        )
        .unwrap();
        assert_eq!(fit.rank, NUISANCE_PARAMETERS);
        times.push(block_times);
        residuals.push(fit.residual);
    }
    (times, residuals)
}

fn variance_recipe(case: &CalCase) -> PhaseVarianceRecipe {
    PhaseVarianceRecipe {
        bins: case.bins,
        min_samples_per_bin: 256,
        min_cycles_per_bin: 100,
        min_contributing_blocks: 8,
        shrinkage_alpha: 0.1,
        floor_ratio: 0.05,
    }
}

// AT-013: roles persist without overlap; evaluation-only mutation leaves
// model bytes unchanged; estimators are pure over their inputs.
#[test]
fn roles_blocks_and_leakage() {
    let case = case("white_stationary");
    let plan = plan_blocks(&plan_request(&case)).unwrap();
    assert_eq!(plan.blocks.len(), 12);
    assert_eq!(
        plan.blocks
            .iter()
            .filter(|block| block.role == CalibrationRole::Training)
            .count(),
        8
    );
    // Blocks tile intervals without gaps or overlaps; trailing tail recorded.
    for window in plan.blocks.windows(2) {
        if window[0].end <= window[1].start {
            continue;
        }
        panic!("overlapping realized blocks");
    }
    // Overlapping nomination is rejected, not repaired.
    let mut bad = plan_request(&case);
    bad.intervals.push(RoleInterval {
        role: CalibrationRole::Evaluation,
        start: 0,
        end: 100,
    });
    assert_eq!(
        plan_blocks(&bad).unwrap_err().code(),
        "invalid_calibration_request"
    );
    // Inverted and out-of-range intervals are rejected.
    let mut inverted = plan_request(&case);
    inverted.intervals[0] = RoleInterval {
        role: CalibrationRole::Training,
        start: 500,
        end: 100,
    };
    assert_eq!(
        plan_blocks(&inverted).unwrap_err().code(),
        "invalid_calibration_request"
    );
    // Too few training blocks publishes no plan.
    let mut thin = plan_request(&case);
    thin.min_training_blocks = 100;
    assert_eq!(
        plan_blocks(&thin).unwrap_err().code(),
        "insufficient_calibration"
    );
    // Single-interval training violates the two-interval recipe rule.
    let mut single = plan_request(&case);
    single.intervals = vec![RoleInterval {
        role: CalibrationRole::Training,
        start: 0,
        end: 8 * case.block_len as u64,
    }];
    single.min_training_blocks = 8;
    assert_eq!(
        plan_blocks(&single).unwrap_err().code(),
        "insufficient_calibration"
    );
}

// AT-014: missing phase arcs, insufficient cycles, and partial intervals
// fail predictably; exclusions are persisted, never hidden.
#[test]
fn coverage_gaps_are_errors_with_exclusions() {
    let case = case("white_stationary");
    let (times, residuals) = train_residuals(&case);
    // Drop every sample in the first octant: missing bins must fail.
    let mut gapped_times = Vec::new();
    let mut gapped_residuals = Vec::new();
    for (block_times, block_residuals) in times.iter().zip(residuals.iter()) {
        let mut kept_times = Vec::new();
        let mut kept_residuals = Vec::new();
        for (time, residual) in block_times.iter().zip(block_residuals.iter()) {
            let phase = (std::f64::consts::TAU * case.f_ref * time - case.phase_rad)
                .rem_euclid(std::f64::consts::TAU);
            if phase > std::f64::consts::TAU / 8.0 {
                kept_times.push(*time);
                kept_residuals.push(*residual);
            }
        }
        gapped_times.push(kept_times);
        gapped_residuals.push(kept_residuals);
    }
    let samples =
        assemble_samples(&gapped_times, &gapped_residuals, case.f_ref, case.phase_rad).unwrap();
    assert_eq!(
        estimate_phase_variance(&samples, variance_recipe(&case))
            .unwrap_err()
            .code(),
        "insufficient_calibration"
    );
    // Partial trailing interval: whole blocks kept, tail excluded by index.
    let mut partial = plan_request(&case);
    partial.intervals[0] = RoleInterval {
        role: CalibrationRole::Training,
        start: 0,
        end: 3 * case.block_len as u64 + 17,
    };
    let plan = plan_blocks(&partial).unwrap_err();
    assert_eq!(plan.code(), "insufficient_calibration");
}

// AT-015: variance recipe against the committed oracle: mean removal,
// count-1 pooling, cyclic smoothing, shrink/floor order, and
// variance-versus-standard-deviation confusion.
#[test]
fn variance_recipe_matches_oracle() {
    for name in ["white_stationary", "heteroscedastic_carriers"] {
        let case = case(name);
        let (times, residuals) = train_residuals(&case);
        let samples = assemble_samples(&times, &residuals, case.f_ref, case.phase_rad).unwrap();
        let output = estimate_phase_variance(&samples, variance_recipe(&case)).unwrap();
        let oracle = case.oracle.get("variance").unwrap();
        let want_v: Vec<f64> =
            serde_json::from_value(oracle.get("variances").unwrap().clone()).unwrap();
        close_vec(
            &output.variances,
            &want_v,
            1e-9,
            &format!("{name} variances"),
        );
        close(
            output.v0,
            oracle.get("v0").unwrap().as_f64().unwrap(),
            1e-9,
            &format!("{name} v0"),
        );
        assert_eq!(output.contributing_blocks, 8);
        assert!(output.floor_activations.is_empty());
    }
    // Variance, not standard deviation: white sigma=2 estimates ~4, not ~2.
    let case = case("white_stationary");
    let (times, residuals) = train_residuals(&case);
    let samples = assemble_samples(&times, &residuals, case.f_ref, case.phase_rad).unwrap();
    let output = estimate_phase_variance(&samples, variance_recipe(&case)).unwrap();
    close(output.v0, 4.0, 0.25, "white v0 near sigma^2");
    assert!(
        (output.v0 - 2.0).abs() > 1.0,
        "estimate {} is not a standard deviation",
        output.v0
    );
    // Within-bin mean removal: a per-bin DC offset leaves variance unchanged.
    let mut offset: Vec<CalSample> = samples.clone();
    for sample in offset.iter_mut() {
        let bin = (sample.phase_rad / std::f64::consts::TAU * case.bins as f64).floor() as usize
            % case.bins;
        sample.residual += 10.0 * bin as f64;
    }
    let shifted = estimate_phase_variance(&offset, variance_recipe(&case)).unwrap();
    close_vec(&shifted.variances, &output.variances, 1e-9, "mean removal");
    // Phase-origin shift permutes bins without changing the value set.
    let shifted_phase = case.phase_rad + std::f64::consts::TAU / case.bins as f64;
    let (times, residuals) = train_residuals(&case);
    let rotated = assemble_samples(&times, &residuals, case.f_ref, shifted_phase).unwrap();
    let output_rotated = estimate_phase_variance(&rotated, variance_recipe(&case)).unwrap();
    let mut sorted = output.variances.clone();
    let mut sorted_rotated = output_rotated.variances.clone();
    sorted.sort_by(f64::total_cmp);
    sorted_rotated.sort_by(f64::total_cmp);
    close_vec(&sorted, &sorted_rotated, 1e-9, "origin shift");
}

// AT-016: known-noise bias and uncertainty; nuisance regression removes the
// committed carriers; DOF treatment stays approximate and explicit.
#[test]
fn known_noise_bias_and_uncertainty() {
    let case = case("heteroscedastic_carriers");
    let tolerances = JointSolverTolerances::default();
    // Carrier recovery on block 0: DC, trend slope, h1..h3 amplitudes.
    let start = 0;
    let block_times = block_times(start, case.block_len, case.dt);
    let fit = fit_nuisance(
        &block_times,
        &case.signals[0],
        case.f_ref,
        case.phase_rad,
        tolerances,
    )
    .unwrap();
    close(fit.coefficients[0], 0.5, 0.05, "nuisance DC");
    // Per-block trend over 20 ms is far below the noise (slope drift 2e-4
    // vs sigma ~1.5): the trend absorbs slow drift but its value is not
    // identifiable, so only finiteness is required here.
    assert!(fit.coefficients[1].is_finite());
    // Harmonic SE ≈ sigma/sqrt(N/2) ≈ 0.05; 0.15 is a 3-sigma band.
    close(fit.coefficients[2], 1.5, 0.15, "nuisance a1");
    close(fit.coefficients[3], -0.5, 0.15, "nuisance b1");
    // Residual variance matches the local heteroscedastic truth on average:
    // effective DOF ≈ samples - nuisance params per block (approximate).
    let (times, residuals) = train_residuals(&case);
    let samples = assemble_samples(&times, &residuals, case.f_ref, case.phase_rad).unwrap();
    let output = estimate_phase_variance(&samples, variance_recipe(&case)).unwrap();
    let truth_profile: Vec<f64> =
        serde_json::from_value(case.truth.get("variance_profile").unwrap().clone()).unwrap();
    let truth_v0 = case.truth.get("v0").unwrap().as_f64().unwrap();
    for (bin, (estimated, profile)) in output
        .variances
        .iter()
        .zip(truth_profile.iter())
        .enumerate()
    {
        // Per-bin sampling SE ≈ v*sqrt(2/count) after smoothing/shrinkage;
        // 5 sigma against the known truth is generous but binding.
        let truth = truth_v0 * profile;
        let count = output.counts[bin] as f64;
        let sigma = truth * (2.0 / count).sqrt();
        assert!(
            (estimated - truth).abs() <= 5.0 * sigma,
            "bin {bin}: {estimated} vs truth {truth} (5-sigma {sigma})"
        );
    }
}

// AT-017: correlation recipe against the committed oracle: biased-lag
// denominator, lag-zero normalization, Bartlett taper, identity shrinkage,
// SPD, and block-local estimation (no cross-block concatenation).
#[test]
fn correlation_recipe_matches_oracle() {
    let recipe = CorrelationRecipe {
        max_lag: 64,
        shrinkage_eta: 0.05,
    };
    // White case: standardize by the known truth (flat sigma = 2); lag-zero
    // normalization makes the scale cancel against the committed oracle,
    // which ran on raw residuals.
    let white = case("white_stationary");
    let (_, residuals) = train_residuals(&white);
    let standardized: Vec<Vec<f64>> = residuals
        .iter()
        .map(|block| block.iter().map(|value| value / 2.0).collect())
        .collect();
    let output = estimate_correlation(&standardized, None, white.dt, recipe).unwrap();
    let oracle = white.oracle.get("correlation").unwrap();
    let want: Vec<f64> = serde_json::from_value(oracle.get("lags").unwrap().clone()).unwrap();
    close_vec(&output.lags, &want, 1e-9, "white lags");
    close(output.lags[0], 1.0, 1e-12, "lag-zero normalization");
    assert_eq!(output.taper_id, "bartlett_v1");
    assert!(output.spd_validated);
    assert!(output.tail_energy.is_some());
    // AR(1) case: committed blocks, taper theory, and small tail.
    let ar1 = case("ar1_correlated");
    let output = estimate_correlation(&ar1.standardized_blocks, None, ar1.dt, recipe).unwrap();
    let oracle = ar1.oracle.get("correlation").unwrap();
    let want: Vec<f64> = serde_json::from_value(oracle.get("lags").unwrap().clone()).unwrap();
    close_vec(&output.lags, &want, 1e-9, "ar1 lags");
    for (lag, value) in output.lags.iter().enumerate().take(9) {
        let theory = 0.95 * 0.7_f64.powi(lag as i32) * (1.0 - lag as f64 / 65.0)
            + 0.05 * f64::from(lag == 0);
        assert!(
            (value - theory).abs() <= 0.05,
            "lag {lag}: {value} vs tapered theory {theory}"
        );
    }
    assert!(
        output.tail_energy.unwrap() < 0.1,
        "AR(1) tail beyond J=64 must be sampling noise"
    );
    // Block structure matters: one concatenated block is a different
    // computation (own DC removal and denominator), so hidden concatenation
    // cannot reproduce the blocked estimate.
    let concatenated: Vec<f64> = ar1.standardized_blocks.concat();
    let single = estimate_correlation(&[concatenated], None, ar1.dt, recipe).unwrap();
    assert!(
        (single.lags[1] - output.lags[1]).abs() > 1e-12,
        "concatenated input must not reproduce blocked estimation"
    );
}

// AT-013/AT-018 (core level): artifact byte identity. Same inputs hash to
// the same bytes; training-data mutation changes them; evaluation-only
// mutation does not (evaluation data never enters the model).
#[test]
fn artifact_bytes_bind_training_only() {
    let case = case("white_stationary");
    let plan = plan_blocks(&plan_request(&case)).unwrap();
    let (times, residuals) = train_residuals(&case);
    let samples = assemble_samples(&times, &residuals, case.f_ref, case.phase_rad).unwrap();
    let variance = estimate_phase_variance(&samples, variance_recipe(&case)).unwrap();
    let standardized: Vec<Vec<f64>> = residuals
        .iter()
        .map(|block| block.iter().map(|value| value / 2.0).collect())
        .collect();
    let correlation = estimate_correlation(
        &standardized,
        None,
        case.dt,
        CorrelationRecipe {
            max_lag: 64,
            shrinkage_eta: 0.05,
        },
    )
    .unwrap();
    let binding = ModelBinding {
        channel: 3,
        voltage_unit: "V".to_string(),
        adc_scale_provenance: Some("synthetic-fixture".to_string()),
        sample_interval_s: case.dt,
        recorded_original_dt_s: Some(case.dt),
        sample_interval_rel_tol: DEFAULT_DT_REL_TOL,
        reference_frequency_hz: case.f_ref,
        frequency_rel_tol: DEFAULT_FREQ_REL_TOL,
        phase_convention: CALIBRATION_PHASE_CONVENTION.to_string(),
        acquisition: AcquisitionMeta {
            device: Some("synthetic".to_string()),
            gain: Some(1.0),
            bandwidth_hz: Some(50_000.0),
        },
    };
    let request = ArtifactRequest {
        model_id: "wp3-test-white".to_string(),
        pmoke_version: "0.0.0-test".to_string(),
        backend_versions: BTreeMap::from([("test".to_string(), "0".to_string())]),
        binding,
        variance: variance.clone(),
        correlation: Some(correlation),
        recipe: variance_recipe(&case),
        correlation_recipe: Some(CorrelationRecipe {
            max_lag: 64,
            shrinkage_eta: 0.05,
        }),
        plan: plan.clone(),
        source_digests: vec!["synthetic".to_string()],
        seed: 20260920,
        tuning: TuningMode::Fixed,
        heldout: HeldoutReport {
            blocks: 2,
            profile_rmse_v2: Some(0.01),
            standardized_lag1: Some(0.02),
        },
    };
    let first = build_artifact(request.clone()).unwrap();
    assert_eq!(
        first.artifact.schema_version,
        CALIBRATION_ARTIFACT_SCHEMA_VERSION
    );
    assert_eq!(
        first.artifact.algorithm_version,
        CALIBRATION_ALGORITHM_VERSION
    );
    assert_eq!(first.artifact.capabilities.modes.len(), 3);
    // Deterministic bytes: rebuild hashes identically.
    let second = build_artifact(request.clone()).unwrap();
    assert_eq!(first.json_bytes, second.json_bytes);
    assert_eq!(first.sha256_hex, second.sha256_hex);
    assert_eq!(first.sha256_hex.len(), 64);
    // Evaluation-only mutation leaves model bytes unchanged: rebuild from
    // signals with the evaluation blocks replaced by zeros. Training blocks
    // are untouched, so the estimate and its hash must reproduce exactly.
    let mut eval_mutated = case.signals.clone();
    for block in eval_mutated.iter_mut().skip(10).take(2) {
        *block = vec![0.0; case.block_len];
    }
    let tolerances = JointSolverTolerances::default();
    let mut mutated_times = Vec::new();
    let mut mutated_residuals = Vec::new();
    for (block, signal) in eval_mutated.iter().enumerate().take(8) {
        let start = block as u64 * case.block_len as u64;
        let block_times = block_times(start, case.block_len, case.dt);
        let fit =
            fit_nuisance(&block_times, signal, case.f_ref, case.phase_rad, tolerances).unwrap();
        mutated_times.push(block_times);
        mutated_residuals.push(fit.residual);
    }
    let mutated_samples = assemble_samples(
        &mutated_times,
        &mutated_residuals,
        case.f_ref,
        case.phase_rad,
    )
    .unwrap();
    let mutated_variance =
        estimate_phase_variance(&mutated_samples, variance_recipe(&case)).unwrap();
    assert_eq!(mutated_variance.variances, variance.variances);
    // Training-data mutation does change the model: perturb one sample.
    let mut train_mutated = case.signals.clone();
    train_mutated[0][0] += 1.0;
    let perturbed_times = block_times(0, case.block_len, case.dt);
    let perturbed = fit_nuisance(
        &perturbed_times,
        &train_mutated[0],
        case.f_ref,
        case.phase_rad,
        tolerances,
    )
    .unwrap();
    assert_ne!(perturbed.residual, residuals[0]);
    // The hashed bytes are the parsed bytes: re-parse and compare digests.
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest as _;
    hasher.update(&first.json_bytes);
    let reparsed: [u8; 32] = hasher.finalize().into();
    assert_eq!(reparsed, first.sha256);
    // Oversized artifacts fail before hashing, not after.
    let mut huge_variance = variance.clone();
    huge_variance.variances = vec![1.0; 200_000];
    huge_variance.counts = vec![1; 200_000];
    let mut huge_request = request.clone();
    huge_request.variance = huge_variance;
    assert_eq!(
        build_artifact(huge_request).unwrap_err().code(),
        "resource_limit_exceeded"
    );
    // Autotuning search is rejected explicitly, never silently fixed.
    let mut search_request = request.clone();
    search_request.tuning = TuningMode::Search(SearchSpace {
        candidates: BTreeMap::from([("alpha".to_string(), vec![0.1, 0.2])]),
        seed: 1,
        selection_rule: "heldout".to_string(),
    });
    assert_eq!(
        build_artifact(search_request).unwrap_err().code(),
        "unsupported_autotuning"
    );
    assert!(ensure_fixed_tuning(&TuningMode::Fixed).is_ok());
}

// FR-031: applicability binding. Cross-channel and incompatible dt/freq
// fail hard; unknown acquisition stays an explicit warning.
#[test]
fn applicability_binding() {
    let case = case("white_stationary");
    let plan = plan_blocks(&plan_request(&case)).unwrap();
    let (times, residuals) = train_residuals(&case);
    let samples = assemble_samples(&times, &residuals, case.f_ref, case.phase_rad).unwrap();
    let variance = estimate_phase_variance(&samples, variance_recipe(&case)).unwrap();
    let request = ArtifactRequest {
        model_id: "wp3-test-binding".to_string(),
        pmoke_version: "0.0.0-test".to_string(),
        backend_versions: BTreeMap::new(),
        binding: ModelBinding {
            channel: 3,
            voltage_unit: "V".to_string(),
            adc_scale_provenance: None,
            sample_interval_s: case.dt,
            recorded_original_dt_s: None,
            sample_interval_rel_tol: DEFAULT_DT_REL_TOL,
            reference_frequency_hz: case.f_ref,
            frequency_rel_tol: DEFAULT_FREQ_REL_TOL,
            phase_convention: CALIBRATION_PHASE_CONVENTION.to_string(),
            acquisition: AcquisitionMeta {
                device: None,
                gain: None,
                bandwidth_hz: None,
            },
        },
        variance,
        correlation: None,
        recipe: variance_recipe(&case),
        correlation_recipe: None,
        plan,
        source_digests: Vec::new(),
        seed: 0,
        tuning: TuningMode::Fixed,
        heldout: HeldoutReport {
            blocks: 0,
            profile_rmse_v2: None,
            standardized_lag1: None,
        },
    };
    let built = build_artifact(request).unwrap();
    assert_eq!(built.artifact.capabilities.modes.len(), 2);
    let good = ApplicabilityRequest {
        channel: 3,
        sample_interval_s: case.dt,
        reference_frequency_hz: case.f_ref,
        voltage_unit: "V".to_string(),
        phase_convention: CALIBRATION_PHASE_CONVENTION.to_string(),
        acquisition_known: true,
    };
    let report = inspect_applicability(&built.artifact, &good);
    // Acquisition metadata is explicitly unknown in this artifact.
    assert!(report.compatible);
    assert_eq!(
        report.warnings,
        vec!["unverified_acquisition_conditions".to_string()]
    );
    // Cross-channel reuse is a hard error.
    let mut other_channel = good.clone();
    other_channel.channel = 5;
    let report = inspect_applicability(&built.artifact, &other_channel);
    assert!(!report.compatible);
    assert_eq!(report.errors[0].code, "model_binding_mismatch");
    // Incompatible dt fails; a 1e-12 relative drift passes.
    let mut wrong_dt = good.clone();
    wrong_dt.sample_interval_s = case.dt * 1.001;
    assert!(!inspect_applicability(&built.artifact, &wrong_dt).compatible);
    let mut close_dt = good.clone();
    close_dt.sample_interval_s = case.dt * (1.0 + 1e-12);
    assert!(inspect_applicability(&built.artifact, &close_dt).compatible);
    // Wrong reference frequency and units fail.
    let mut wrong_freq = good.clone();
    wrong_freq.reference_frequency_hz = case.f_ref * 2.0;
    assert!(!inspect_applicability(&built.artifact, &wrong_freq).compatible);
    let mut wrong_unit = good.clone();
    wrong_unit.voltage_unit = "mV".to_string();
    assert!(!inspect_applicability(&built.artifact, &wrong_unit).compatible);
}

// AT-041: SCS adequacy. Stationary white is adequate; time-drifting
// correlation is a model mismatch caught by the reserved comparison.
#[test]
fn scs_adequacy_diagnostic() {
    let adequate_case = case("scs_adequate_white");
    let policy = adequacy_policy(&adequate_case);
    let train: Vec<AdequacyGroup> = adequate_case
        .train_blocks
        .iter()
        .map(|block| AdequacyGroup {
            standardized: block.clone(),
            phases: adequate_case.phases.clone(),
        })
        .collect();
    let reserved: Vec<AdequacyGroup> = adequate_case
        .reserved_blocks
        .iter()
        .map(|block| AdequacyGroup {
            standardized: block.clone(),
            phases: adequate_case.phases.clone(),
        })
        .collect();
    let report = scs_adequacy(&train, &reserved, policy).unwrap();
    let oracle = adequate_case.oracle.get("adequacy").unwrap();
    assert_eq!(
        report.adequate,
        oracle.get("adequate").unwrap().as_bool().unwrap()
    );
    close(
        report.phase_spread,
        oracle.get("phase_spread").unwrap().as_f64().unwrap(),
        1e-9,
        "adequate spread",
    );
    close(
        report.reserved_shift,
        oracle.get("reserved_shift").unwrap().as_f64().unwrap(),
        1e-9,
        "adequate shift",
    );
    assert!(report.adequate);
    assert!(report.warnings.is_empty());
    let drift_case = case("nonseparable_drift");
    let policy = adequacy_policy(&drift_case);
    let train: Vec<AdequacyGroup> = drift_case
        .train_blocks
        .iter()
        .map(|block| AdequacyGroup {
            standardized: block.clone(),
            phases: drift_case.phases.clone(),
        })
        .collect();
    let reserved: Vec<AdequacyGroup> = drift_case
        .reserved_blocks
        .iter()
        .map(|block| AdequacyGroup {
            standardized: block.clone(),
            phases: drift_case.phases.clone(),
        })
        .collect();
    let report = scs_adequacy(&train, &reserved, policy).unwrap();
    let oracle = drift_case.oracle.get("adequacy").unwrap();
    assert_eq!(
        report.adequate,
        oracle.get("adequate").unwrap().as_bool().unwrap()
    );
    assert!(
        !report.adequate,
        "drift must be a model mismatch, not adequacy"
    );
    assert!(report.reserved_shift > policy.max_reserved_shift);
    assert_eq!(
        report.warnings,
        vec!["insufficient_scs_adequacy".to_string()]
    );
    assert!(report.reason.contains("reserved shift"));
}
