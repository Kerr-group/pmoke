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
    CALIBRATION_PHASE_CONVENTION, CalSample, CalibrationArtifact, CalibrationRole,
    CorrelationRecipe, DEFAULT_CORRELATION_ETA, DEFAULT_DT_REL_TOL, DEFAULT_FREQ_REL_TOL,
    HeldoutReport, JointSolverTolerances, ModelBinding, NUISANCE_PARAMETERS, PhaseVarianceRecipe,
    RoleInterval, SearchSpace, TuningMode, assemble_samples, build_artifact, ensure_fixed_tuning,
    estimate_correlation, estimate_phase_variance, fit_nuisance, inspect_applicability,
    plan_blocks, scs_adequacy,
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
            1.0 / case.dt,
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
        1.0 / case.dt,
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
        reference_frequency_rel_uncertainty: None,
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
        frequency_tol_overridden: false,
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
        let fit = fit_nuisance(
            &block_times,
            signal,
            case.f_ref,
            case.phase_rad,
            1.0 / case.dt,
            tolerances,
        )
        .unwrap();
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
        1.0 / case.dt,
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
    huge_variance.distinct_cycles = vec![1; 200_000];
    huge_variance.raw = vec![1.0; 200_000];
    huge_variance.smoothed = vec![1.0; 200_000];
    let mut huge_recipe = variance_recipe(&case);
    huge_recipe.bins = 200_000;
    let mut huge_request = request.clone();
    huge_request.variance = huge_variance;
    huge_request.recipe = huge_recipe;
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
            adc_scale_provenance: Some("synthetic-scale".to_string()),
            sample_interval_s: case.dt,
            recorded_original_dt_s: None,
            sample_interval_rel_tol: DEFAULT_DT_REL_TOL,
            reference_frequency_hz: case.f_ref,
            frequency_rel_tol: DEFAULT_FREQ_REL_TOL,
            reference_frequency_rel_uncertainty: None,
            phase_convention: CALIBRATION_PHASE_CONVENTION.to_string(),
            acquisition: AcquisitionMeta {
                device: Some("synthetic".to_string()),
                gain: Some(2.0),
                bandwidth_hz: Some(50_000.0),
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
        frequency_tol_overridden: false,
    };
    let built = build_artifact(request).unwrap();
    assert_eq!(built.artifact.capabilities.modes.len(), 2);
    let good = ApplicabilityRequest {
        channel: 3,
        sample_interval_s: case.dt,
        reference_frequency_hz: case.f_ref,
        voltage_unit: "V".to_string(),
        phase_convention: CALIBRATION_PHASE_CONVENTION.to_string(),
        adc_scale_provenance: Some("synthetic-scale".to_string()),
        acquisition: AcquisitionMeta {
            device: Some("synthetic".to_string()),
            gain: Some(2.0),
            bandwidth_hz: Some(50_000.0),
        },
        reference_frequency_rel_uncertainty: None,
    };
    let report = inspect_applicability(&built.artifact, &good);
    // Fully matching acquisition: compatible with no warnings.
    assert!(report.compatible);
    assert!(report.warnings.is_empty());
    // A changed gain is a hard mismatch even though every other field holds.
    let mut changed_gain = good.clone();
    changed_gain.acquisition.gain = Some(4.0);
    let report = inspect_applicability(&built.artifact, &changed_gain);
    assert!(!report.compatible);
    assert_eq!(report.errors[0].code, "model_binding_mismatch");
    // Unknown request-side gain stays explicitly unverified, not an error.
    let mut unknown_gain = good.clone();
    unknown_gain.acquisition.gain = None;
    let report = inspect_applicability(&built.artifact, &unknown_gain);
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
    for key in [
        "phase_spread",
        "reserved_spread",
        "reserved_shift",
        "pattern_shift",
    ] {
        close(
            match key {
                "phase_spread" => report.phase_spread,
                "reserved_spread" => report.reserved_spread,
                "reserved_shift" => report.reserved_shift,
                _ => report.pattern_shift,
            },
            oracle.get(key).unwrap().as_f64().unwrap(),
            1e-9,
            &format!("adequate {key}"),
        );
    }
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
    assert!(report.reason.contains("pattern"));
}

// F1: held-out phase-dependent correlation with a canceling pooled average.
// White training plus sign-flipping reserved pairs: pooled shift stays near
// zero while reserved octants swing, so only reserved phase statistics can
// reject. A pooled-only diagnostic would affirm adequacy here.
#[test]
fn reserved_periodic_structure_is_rejected() {
    let periodic = case("periodic_reserved_only");
    let policy = adequacy_policy(&periodic);
    let train: Vec<AdequacyGroup> = periodic
        .train_blocks
        .iter()
        .map(|block| AdequacyGroup {
            standardized: block.clone(),
            phases: periodic.phases.clone(),
        })
        .collect();
    let reserved: Vec<AdequacyGroup> = periodic
        .reserved_blocks
        .iter()
        .map(|block| AdequacyGroup {
            standardized: block.clone(),
            phases: periodic.phases.clone(),
        })
        .collect();
    let report = scs_adequacy(&train, &reserved, policy).unwrap();
    let oracle = periodic.oracle.get("adequacy").unwrap();
    assert_eq!(
        report.adequate,
        oracle.get("adequate").unwrap().as_bool().unwrap()
    );
    assert!(!report.adequate);
    // The pooled averages agree: this is the blind spot of pooled-only checks.
    assert!(report.reserved_shift < policy.max_reserved_shift);
    assert!(report.reserved_spread > policy.max_phase_spread);
    close(
        report.reserved_spread,
        oracle.get("reserved_spread").unwrap().as_f64().unwrap(),
        1e-9,
        "reserved spread",
    );
    assert!(report.reason.contains("reserved phase spread"));
}

// F2: missing power, missing coverage, and non-finite policy never affirm.
#[test]
fn adequacy_gates_reject_unverifiable_inputs() {
    let policy = AdequacyPolicy {
        lags: 4,
        min_pairs_per_cell: 10,
        max_phase_spread: 0.5,
        max_reserved_shift: 0.5,
    };
    let phases: Vec<f64> = (0..256)
        .map(|i| i as f64 / 256.0 * std::f64::consts::TAU)
        .collect();
    let white = AdequacyGroup {
        standardized: (0..256).map(|i| ((i * 37) as f64 / 256.0).sin()).collect(),
        phases: phases.clone(),
    };
    // All-zero residuals carry no lag-zero power.
    let zero = AdequacyGroup {
        standardized: vec![0.0; 256],
        phases: phases.clone(),
    };
    assert_eq!(
        scs_adequacy(
            std::slice::from_ref(&zero),
            std::slice::from_ref(&white),
            policy
        )
        .unwrap_err()
        .code(),
        "insufficient_calibration"
    );
    // Reserved phases at a single label miss every other octant.
    let single_phase = AdequacyGroup {
        standardized: white.standardized.clone(),
        phases: vec![0.0; 256],
    };
    assert_eq!(
        scs_adequacy(std::slice::from_ref(&white), &[single_phase], policy)
            .unwrap_err()
            .code(),
        "insufficient_calibration"
    );
    // NaN thresholds cannot compare; zero lags/pairs are malformed policy.
    let nan_policy = AdequacyPolicy {
        max_phase_spread: f64::NAN,
        ..policy
    };
    assert_eq!(
        scs_adequacy(
            std::slice::from_ref(&white),
            std::slice::from_ref(&white),
            nan_policy
        )
        .unwrap_err()
        .code(),
        "invalid_calibration_request"
    );
    let zero_lag = AdequacyPolicy { lags: 0, ..policy };
    assert_eq!(
        scs_adequacy(
            std::slice::from_ref(&white),
            std::slice::from_ref(&white),
            zero_lag
        )
        .unwrap_err()
        .code(),
        "invalid_calibration_request"
    );
}

// Issue #301 follow-up: a validated but unsatisfiable lag count must fail
// with the typed insufficient_calibration error before any lag-sized
// allocation (no capacity-overflow panic, no huge allocation). The
// boundary lag == shortest group length fails the same way; a small
// satisfiable lag on the same groups still reaches the verdict path.
#[test]
fn adequacy_rejects_impossible_lag_support_before_allocation() {
    let samples = 64usize;
    let phases: Vec<f64> = (0..samples)
        .map(|i| i as f64 / samples as f64 * std::f64::consts::TAU)
        .collect();
    let group = || AdequacyGroup {
        standardized: (0..samples)
            .map(|i| ((i * 37) as f64 / samples as f64).sin())
            .collect(),
        phases: phases.clone(),
    };
    let train = group();
    let reserved = group();
    let policy = AdequacyPolicy {
        lags: 4,
        min_pairs_per_cell: 1,
        max_phase_spread: 0.5,
        max_reserved_shift: 0.5,
    };
    // Control: a small satisfiable lag count reaches a verdict, not an error.
    assert!(
        scs_adequacy(
            std::slice::from_ref(&train),
            std::slice::from_ref(&reserved),
            policy
        )
        .is_ok()
    );
    // Boundary: lag support equal to the shortest group length has no pairs.
    let boundary = AdequacyPolicy {
        lags: samples,
        ..policy
    };
    assert_eq!(
        scs_adequacy(
            std::slice::from_ref(&train),
            std::slice::from_ref(&reserved),
            boundary
        )
        .unwrap_err()
        .code(),
        "insufficient_calibration"
    );
    // The validated TOML-representable reproducer (i64::MAX) must surface
    // the same typed error instead of panicking on allocation.
    let oversized = AdequacyPolicy {
        lags: i64::MAX as usize,
        ..policy
    };
    assert_eq!(
        scs_adequacy(
            std::slice::from_ref(&train),
            std::slice::from_ref(&reserved),
            oversized
        )
        .unwrap_err()
        .code(),
        "insufficient_calibration"
    );
}

// F3: pool-then-normalize analytical anchor with unequal block power.
#[test]
fn correlation_pool_order_anchor() {
    let recipe = CorrelationRecipe {
        max_lag: 1,
        shrinkage_eta: 0.0,
    };
    let output = estimate_correlation(
        &[vec![1.0, 1.0, -1.0, -1.0], vec![3.0, -3.0, 3.0, -3.0]],
        None,
        1e-5,
        recipe,
    )
    .unwrap();
    // Pooled auto0 = 5, pooled auto1 = -3.25, normalized -0.65, taper 1/2.
    close(output.lags[0], 1.0, 1e-12, "anchor lag0");
    close(output.lags[1], -0.325, 1e-12, "anchor lag1");
}

// F4: the accepted A-005 correlation shrinkage default is 0.01.
#[test]
fn correlation_default_eta_matches_accepted_recipe() {
    assert_eq!(DEFAULT_CORRELATION_ETA, 0.01);
    assert_eq!(CorrelationRecipe::default().shrinkage_eta, 0.01);
}

// F5: the constructor rejects internally invalid models before hashing.
#[test]
fn artifact_constructor_rejects_invalid_models() {
    use pmoke_analysis_core::{BlockPlan, PhaseVarianceOutput};
    let case = case("white_stationary");
    let plan = plan_blocks(&plan_request(&case)).unwrap();
    let (times, residuals) = train_residuals(&case);
    let samples = assemble_samples(&times, &residuals, case.f_ref, case.phase_rad).unwrap();
    let variance = estimate_phase_variance(&samples, variance_recipe(&case)).unwrap();
    let recipe = CorrelationRecipe {
        max_lag: 64,
        shrinkage_eta: 0.05,
    };
    let standardized: Vec<Vec<f64>> = residuals
        .iter()
        .map(|block| block.iter().map(|value| value / 2.0).collect())
        .collect();
    let correlation = estimate_correlation(&standardized, None, case.dt, recipe).unwrap();
    let binding = ModelBinding {
        channel: 3,
        voltage_unit: "V".to_string(),
        adc_scale_provenance: None,
        sample_interval_s: case.dt,
        recorded_original_dt_s: None,
        sample_interval_rel_tol: DEFAULT_DT_REL_TOL,
        reference_frequency_hz: case.f_ref,
        frequency_rel_tol: DEFAULT_FREQ_REL_TOL,
        reference_frequency_rel_uncertainty: None,
        phase_convention: CALIBRATION_PHASE_CONVENTION.to_string(),
        acquisition: AcquisitionMeta {
            device: None,
            gain: None,
            bandwidth_hz: None,
        },
    };
    let base = ArtifactRequest {
        model_id: "wp3-test-forge".to_string(),
        pmoke_version: "0.0.0-test".to_string(),
        backend_versions: BTreeMap::new(),
        binding,
        variance: variance.clone(),
        correlation: Some(correlation.clone()),
        recipe: variance_recipe(&case),
        correlation_recipe: Some(recipe),
        plan: BlockPlan {
            intervals: plan.intervals.clone(),
            blocks: plan.blocks.clone(),
            exclusions: plan.exclusions.clone(),
        },
        source_digests: Vec::new(),
        seed: 0,
        tuning: TuningMode::Fixed,
        heldout: HeldoutReport {
            blocks: 0,
            profile_rmse_v2: None,
            standardized_lag1: None,
        },
        frequency_tol_overridden: false,
    };
    // Empty phase table advertises no bins: rejected, not hashed.
    let mut empty = base.clone();
    empty.variance = PhaseVarianceOutput {
        variances: Vec::new(),
        v0: 1.0,
        counts: Vec::new(),
        distinct_cycles: Vec::new(),
        contributing_blocks: 8,
        raw: Vec::new(),
        smoothed: Vec::new(),
        floor_activations: Vec::new(),
    };
    assert_eq!(
        build_artifact(empty).unwrap_err().code(),
        "insufficient_calibration"
    );
    // Forged SPD flag with an indefinite leading block: refactorization fails.
    let mut forged = correlation.clone();
    forged.lags = vec![1.0, 2.0].into_iter().chain(vec![0.0; 63]).collect();
    let mut forged_request = base.clone();
    forged_request.correlation = Some(forged);
    assert_eq!(
        build_artifact(forged_request).unwrap_err().code(),
        "covariance_not_spd"
    );
    // NaN lag: rejected before serialization (serde_json would emit null,
    // which cannot deserialize back into the model type).
    let mut nonfinite = correlation.clone();
    nonfinite.lags[3] = f64::NAN;
    let mut nonfinite_request = base.clone();
    nonfinite_request.correlation = Some(nonfinite);
    assert_eq!(
        build_artifact(nonfinite_request).unwrap_err().code(),
        "non_finite_input"
    );
    // Conflicting eta between output and recipe: one authoritative record.
    let mut conflict_recipe = recipe;
    conflict_recipe.shrinkage_eta = 0.25;
    let mut conflict_request = base.clone();
    conflict_request.correlation_recipe = Some(conflict_recipe);
    assert_eq!(
        build_artifact(conflict_request).unwrap_err().code(),
        "invalid_calibration_request"
    );
}

// F7: hash tests that rebuild the artifact after each mutation and compare
// digests, plus a real-type round trip of the generated bytes.
#[test]
fn rebuilt_artifact_hashes_track_data_mutations() {
    let case = case("white_stationary");
    let plan = plan_blocks(&plan_request(&case)).unwrap();
    let binding = ModelBinding {
        channel: 3,
        voltage_unit: "V".to_string(),
        adc_scale_provenance: None,
        sample_interval_s: case.dt,
        recorded_original_dt_s: None,
        sample_interval_rel_tol: DEFAULT_DT_REL_TOL,
        reference_frequency_hz: case.f_ref,
        frequency_rel_tol: DEFAULT_FREQ_REL_TOL,
        reference_frequency_rel_uncertainty: None,
        phase_convention: CALIBRATION_PHASE_CONVENTION.to_string(),
        acquisition: AcquisitionMeta {
            device: None,
            gain: None,
            bandwidth_hz: None,
        },
    };
    let recipe = CorrelationRecipe {
        max_lag: 64,
        shrinkage_eta: 0.05,
    };
    let build_from_signals = |signals: &[Vec<f64>]| {
        let tolerances = JointSolverTolerances::default();
        let mut times = Vec::new();
        let mut residuals = Vec::new();
        for (block, signal) in signals.iter().enumerate().take(8) {
            let block_times = block_times(
                block as u64 * case.block_len as u64,
                case.block_len,
                case.dt,
            );
            let fit = fit_nuisance(
                &block_times,
                signal,
                case.f_ref,
                case.phase_rad,
                1.0 / case.dt,
                tolerances,
            )
            .unwrap();
            times.push(block_times);
            residuals.push(fit.residual);
        }
        let samples = assemble_samples(&times, &residuals, case.f_ref, case.phase_rad).unwrap();
        let variance = estimate_phase_variance(&samples, variance_recipe(&case)).unwrap();
        let standardized: Vec<Vec<f64>> = residuals
            .iter()
            .map(|block| block.iter().map(|value| value / 2.0).collect())
            .collect();
        let correlation = estimate_correlation(&standardized, None, case.dt, recipe).unwrap();
        build_artifact(ArtifactRequest {
            model_id: "wp3-test-rebuild".to_string(),
            pmoke_version: "0.0.0-test".to_string(),
            backend_versions: BTreeMap::new(),
            binding: binding.clone(),
            variance,
            correlation: Some(correlation),
            recipe: variance_recipe(&case),
            correlation_recipe: Some(recipe),
            plan: plan.clone(),
            source_digests: vec!["synthetic".to_string()],
            seed: 20260920,
            tuning: TuningMode::Fixed,
            heldout: HeldoutReport {
                blocks: 2,
                profile_rmse_v2: None,
                standardized_lag1: None,
            },
            frequency_tol_overridden: false,
        })
        .unwrap()
    };
    let baseline = build_from_signals(&case.signals);
    // Generated bytes deserialize into the real model type unchanged.
    let round_trip: CalibrationArtifact = serde_json::from_slice(&baseline.json_bytes).unwrap();
    assert_eq!(round_trip, baseline.artifact);
    // Evaluation-only mutation rebuilds to identical bytes and digest.
    let mut eval_mutated = case.signals.clone();
    for block in eval_mutated.iter_mut().skip(10).take(2) {
        *block = vec![0.0; case.block_len];
    }
    let rebuilt_eval = build_from_signals(&eval_mutated);
    assert_eq!(rebuilt_eval.json_bytes, baseline.json_bytes);
    assert_eq!(rebuilt_eval.sha256_hex, baseline.sha256_hex);
    // Training-data mutation rebuilds to a different digest and content.
    let mut train_mutated = case.signals.clone();
    train_mutated[0][0] += 1.0;
    let rebuilt_train = build_from_signals(&train_mutated);
    assert_ne!(rebuilt_train.sha256_hex, baseline.sha256_hex);
    assert_ne!(
        rebuilt_train.artifact.phase.variances_v2,
        baseline.artifact.phase.variances_v2
    );
}

// F8: nuisance regression follows the shared timebase convention and the
// Nyquist bound; plus a noiseless coefficient anchor on the Rust path.
#[test]
fn nuisance_timebase_and_nyquist_gates() {
    let tolerances = JointSolverTolerances::default();
    // Nonmonotonic times (swapped adjacent pair, endpoints kept): rejected.
    let mut times: Vec<f64> = (0..64).map(|i| i as f64 * 1e-5).collect();
    times.swap(20, 21);
    let signal = vec![0.0; 64];
    assert_eq!(
        fit_nuisance(&times, &signal, 1000.0, 0.0, 100_000.0, tolerances)
            .unwrap_err()
            .code(),
        "invalid_timebase"
    );
    // 12th harmonic past Nyquist (6 kHz reference at 100 kHz sampling):
    // full rank would still succeed, so the bound is explicit.
    let nyquist_times: Vec<f64> = (0..256).map(|i| i as f64 * 1e-5).collect();
    assert_eq!(
        fit_nuisance(
            &nyquist_times,
            &vec![0.0; 256],
            6000.0,
            0.0,
            100_000.0,
            tolerances
        )
        .unwrap_err()
        .code(),
        "aliased_harmonic"
    );
    // Noiseless anchor: DC, scaled trend, and h1 recover exactly.
    let anchor_times: Vec<f64> = (0..512).map(|i| i as f64 * 1e-5).collect();
    let phi: Vec<f64> = anchor_times
        .iter()
        .map(|time| std::f64::consts::TAU * 1000.0 * time - 0.7)
        .collect();
    let first = anchor_times[0];
    let last = anchor_times[511];
    let signal: Vec<f64> = anchor_times
        .iter()
        .zip(phi.iter())
        .map(|(time, phase)| {
            0.5 + 0.25 * (2.0 * (time - first) / (last - first) - 1.0) + 1.5 * phase.cos()
                - 0.5 * phase.sin()
        })
        .collect();
    let fit = fit_nuisance(&anchor_times, &signal, 1000.0, 0.7, 100_000.0, tolerances).unwrap();
    assert_eq!(fit.rank, NUISANCE_PARAMETERS);
    close(fit.coefficients[0], 0.5, 1e-9, "anchor DC");
    close(fit.coefficients[1], 0.25, 1e-9, "anchor trend");
    close(fit.coefficients[2], 1.5, 1e-9, "anchor a1");
    close(fit.coefficients[3], -0.5, 1e-9, "anchor b1");
    for coefficient in fit.coefficients.iter().skip(4) {
        close(*coefficient, 0.0, 1e-9, "anchor null");
    }
}

// F9: block planning near the u64 index ceiling returns a typed error,
// never a debug panic or a release wrap.
#[test]
fn block_plan_checked_arithmetic() {
    let request = BlockPlanRequest {
        intervals: vec![
            RoleInterval {
                role: CalibrationRole::Training,
                start: u64::MAX - 10,
                end: u64::MAX,
            },
            RoleInterval {
                role: CalibrationRole::Training,
                start: 0,
                end: 100,
            },
        ],
        block_len: 64,
        total_samples: u64::MAX,
        reference_frequency_hz: 1000.0,
        sample_interval_s: 1e-5,
        min_reference_cycles_per_block: 0.0,
        min_training_blocks: 1,
        min_training_intervals: 1,
    };
    assert_eq!(
        plan_blocks(&request).unwrap_err().code(),
        "invalid_calibration_request"
    );
}

// M3 review F3: the constructor applies the estimators' recipe-domain
// rules, so invalid recipes fail even when output and recipe agree.
#[test]
fn artifact_constructor_rejects_invalid_recipes() {
    use pmoke_analysis_core::{BlockPlan, CorrelationOutput, PhaseVarianceOutput};
    let case = case("white_stationary");
    let plan = plan_blocks(&plan_request(&case)).unwrap();
    let (times, residuals) = train_residuals(&case);
    let samples = assemble_samples(&times, &residuals, case.f_ref, case.phase_rad).unwrap();
    let variance = estimate_phase_variance(&samples, variance_recipe(&case)).unwrap();
    let recipe = CorrelationRecipe {
        max_lag: 64,
        shrinkage_eta: 0.05,
    };
    let standardized: Vec<Vec<f64>> = residuals
        .iter()
        .map(|block| block.iter().map(|value| value / 2.0).collect())
        .collect();
    let correlation = estimate_correlation(&standardized, None, case.dt, recipe).unwrap();
    let base = ArtifactRequest {
        model_id: "wp3-test-recipe-domains".to_string(),
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
            reference_frequency_rel_uncertainty: None,
            phase_convention: CALIBRATION_PHASE_CONVENTION.to_string(),
            acquisition: AcquisitionMeta {
                device: None,
                gain: None,
                bandwidth_hz: None,
            },
        },
        variance: variance.clone(),
        correlation: Some(correlation.clone()),
        recipe: variance_recipe(&case),
        correlation_recipe: Some(recipe),
        plan: BlockPlan {
            intervals: plan.intervals.clone(),
            blocks: plan.blocks.clone(),
            exclusions: plan.exclusions.clone(),
        },
        source_digests: Vec::new(),
        seed: 0,
        tuning: TuningMode::Fixed,
        heldout: HeldoutReport {
            blocks: 0,
            profile_rmse_v2: None,
            standardized_lag1: None,
        },
        frequency_tol_overridden: false,
    };
    // Positive control: the valid request still builds.
    build_artifact(base.clone()).unwrap();
    // One-bin phase table with a matching recipe: phase estimation needs at
    // least two bins, so agreement on bins=1 is not validation.
    let mut one_bin = base.clone();
    one_bin.variance = PhaseVarianceOutput {
        variances: variance.variances[..1].to_vec(),
        v0: variance.v0,
        counts: variance.counts[..1].to_vec(),
        distinct_cycles: variance.distinct_cycles[..1].to_vec(),
        contributing_blocks: variance.contributing_blocks,
        raw: variance.raw[..1].to_vec(),
        smoothed: variance.smoothed[..1].to_vec(),
        floor_activations: Vec::new(),
    };
    one_bin.recipe = PhaseVarianceRecipe {
        bins: 1,
        ..variance_recipe(&case)
    };
    assert_eq!(
        build_artifact(one_bin).unwrap_err().code(),
        "invalid_calibration_request"
    );
    // Out-of-range shrinkage and negative floor, each agreed by construction.
    let mut bad_alpha = base.clone();
    bad_alpha.recipe.shrinkage_alpha = 2.0;
    assert_eq!(
        build_artifact(bad_alpha).unwrap_err().code(),
        "invalid_calibration_request"
    );
    let mut bad_floor = base.clone();
    bad_floor.recipe.floor_ratio = -0.05;
    assert_eq!(
        build_artifact(bad_floor).unwrap_err().code(),
        "invalid_calibration_request"
    );
    // Negative correlation eta agreed by both output and recipe records.
    let mut bad_eta = base.clone();
    bad_eta.correlation = Some(CorrelationOutput {
        eta: -0.1,
        ..correlation.clone()
    });
    bad_eta.correlation_recipe = Some(CorrelationRecipe {
        max_lag: 64,
        shrinkage_eta: -0.1,
    });
    assert_eq!(
        build_artifact(bad_eta).unwrap_err().code(),
        "invalid_calibration_request"
    );
}

// M3 review F4: the correlation clock must match the bound acquisition
// interval; doubling lag step and duration together fails construction,
// and the valid artifact carries an exactly agreeing clock.
#[test]
fn artifact_constructor_rejects_inconsistent_lag_clock() {
    use pmoke_analysis_core::{BlockPlan, CorrelationOutput};
    let case = case("white_stationary");
    let plan = plan_blocks(&plan_request(&case)).unwrap();
    let (times, residuals) = train_residuals(&case);
    let samples = assemble_samples(&times, &residuals, case.f_ref, case.phase_rad).unwrap();
    let variance = estimate_phase_variance(&samples, variance_recipe(&case)).unwrap();
    let recipe = CorrelationRecipe {
        max_lag: 64,
        shrinkage_eta: 0.05,
    };
    let standardized: Vec<Vec<f64>> = residuals
        .iter()
        .map(|block| block.iter().map(|value| value / 2.0).collect())
        .collect();
    let correlation = estimate_correlation(&standardized, None, case.dt, recipe).unwrap();
    let base = ArtifactRequest {
        model_id: "wp3-test-lag-clock".to_string(),
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
            reference_frequency_rel_uncertainty: None,
            phase_convention: CALIBRATION_PHASE_CONVENTION.to_string(),
            acquisition: AcquisitionMeta {
                device: None,
                gain: None,
                bandwidth_hz: None,
            },
        },
        variance,
        correlation: Some(correlation.clone()),
        recipe: variance_recipe(&case),
        correlation_recipe: Some(recipe),
        plan: BlockPlan {
            intervals: plan.intervals.clone(),
            blocks: plan.blocks.clone(),
            exclusions: plan.exclusions.clone(),
        },
        source_digests: Vec::new(),
        seed: 0,
        tuning: TuningMode::Fixed,
        heldout: HeldoutReport {
            blocks: 0,
            profile_rmse_v2: None,
            standardized_lag1: None,
        },
        frequency_tol_overridden: false,
    };
    let built = build_artifact(base.clone()).unwrap();
    let stored = built.artifact.correlation.unwrap();
    assert_eq!(stored.lag_step_s, case.dt);
    // Doubled lag step with a consistently doubled duration still
    // contradicts the bound acquisition clock: rejected at construction.
    let mut skewed = base.clone();
    skewed.correlation = Some(CorrelationOutput {
        lag_step_s: 2.0 * case.dt,
        physical_duration_s: 2.0 * correlation.physical_duration_s,
        ..correlation
    });
    assert_eq!(
        build_artifact(skewed).unwrap_err().code(),
        "invalid_calibration_request"
    );
}

// M3 review F5: unknown acquisition conditions stay explicitly unverified,
// including when both sides are unknown. Absence of evidence is not
// evidence of a match.
#[test]
fn applicability_unknown_state_matrix() {
    let case = case("white_stationary");
    let plan = plan_blocks(&plan_request(&case)).unwrap();
    let (times, residuals) = train_residuals(&case);
    let samples = assemble_samples(&times, &residuals, case.f_ref, case.phase_rad).unwrap();
    let variance = estimate_phase_variance(&samples, variance_recipe(&case)).unwrap();
    let known_acquisition = AcquisitionMeta {
        device: Some("synthetic".to_string()),
        gain: Some(2.0),
        bandwidth_hz: Some(50_000.0),
    };
    let unknown_acquisition = AcquisitionMeta {
        device: None,
        gain: None,
        bandwidth_hz: None,
    };
    let build = |acquisition: AcquisitionMeta, adc: Option<String>| {
        build_artifact(ArtifactRequest {
            model_id: "wp3-test-unknown-matrix".to_string(),
            pmoke_version: "0.0.0-test".to_string(),
            backend_versions: BTreeMap::new(),
            binding: ModelBinding {
                channel: 3,
                voltage_unit: "V".to_string(),
                adc_scale_provenance: adc,
                sample_interval_s: case.dt,
                recorded_original_dt_s: None,
                sample_interval_rel_tol: DEFAULT_DT_REL_TOL,
                reference_frequency_hz: case.f_ref,
                frequency_rel_tol: DEFAULT_FREQ_REL_TOL,
                reference_frequency_rel_uncertainty: None,
                phase_convention: CALIBRATION_PHASE_CONVENTION.to_string(),
                acquisition,
            },
            variance: variance.clone(),
            correlation: None,
            recipe: variance_recipe(&case),
            correlation_recipe: None,
            plan: plan.clone(),
            source_digests: Vec::new(),
            seed: 0,
            tuning: TuningMode::Fixed,
            heldout: HeldoutReport {
                blocks: 0,
                profile_rmse_v2: None,
                standardized_lag1: None,
            },
            frequency_tol_overridden: false,
        })
        .unwrap()
        .artifact
    };
    let context = |acquisition: AcquisitionMeta, adc: Option<String>| ApplicabilityRequest {
        channel: 3,
        sample_interval_s: case.dt,
        reference_frequency_hz: case.f_ref,
        voltage_unit: "V".to_string(),
        phase_convention: CALIBRATION_PHASE_CONVENTION.to_string(),
        adc_scale_provenance: adc,
        acquisition,
        reference_frequency_rel_uncertainty: None,
    };
    let verified: Vec<String> = vec![];
    // Equal known states: compatible with no warnings.
    let model = build(known_acquisition.clone(), Some("scale-a".to_string()));
    let report = inspect_applicability(
        &model,
        &context(known_acquisition.clone(), Some("scale-a".to_string())),
    );
    assert!(report.compatible);
    assert_eq!(report.warnings, verified);
    // Unequal known states: hard errors (device as well as gain).
    let report = inspect_applicability(
        &model,
        &context(
            AcquisitionMeta {
                device: Some("other".to_string()),
                ..known_acquisition.clone()
            },
            Some("scale-a".to_string()),
        ),
    );
    assert!(!report.compatible);
    assert_eq!(report.errors[0].code, "model_binding_mismatch");
    // One-sided absence: compatible with the unverified warning.
    let report = inspect_applicability(
        &model,
        &context(unknown_acquisition.clone(), Some("scale-a".to_string())),
    );
    assert!(report.compatible);
    assert_eq!(
        report.warnings,
        vec!["unverified_acquisition_conditions".to_string()]
    );
    // Both sides unknown: still compatible, but the warning must fire.
    let unknown_model = build(unknown_acquisition.clone(), None);
    let report = inspect_applicability(&unknown_model, &context(unknown_acquisition, None));
    assert!(report.compatible);
    assert!(report.errors.is_empty());
    assert_eq!(
        report.warnings,
        vec!["unverified_acquisition_conditions".to_string()]
    );
}

// M3 review F6: within-bin variance is mean-offset invariant. Both bins
// carry exact sample variance 2 under offsets 0 and 1e8; the centered
// two-pass computation returns [2, 2] in both cases.
#[test]
fn phase_variance_is_mean_offset_invariant() {
    use pmoke_analysis_core::PhaseVarianceRecipe;
    use std::f64::consts::FRAC_PI_2;
    let recipe = PhaseVarianceRecipe {
        bins: 2,
        min_samples_per_bin: 2,
        min_cycles_per_bin: 2,
        min_contributing_blocks: 2,
        shrinkage_alpha: 0.1,
        floor_ratio: 0.05,
    };
    for offset in [0.0, 1e8] {
        let mut samples = Vec::new();
        for (bin, center) in [FRAC_PI_2, 3.0 * FRAC_PI_2].iter().enumerate() {
            let mean = if bin == 0 { offset } else { -offset };
            for i in 0..2 {
                samples.push(CalSample {
                    phase_rad: *center,
                    cycle: i as i64,
                    residual: mean + if i == 0 { -1.0 } else { 1.0 },
                    block: i as usize,
                });
            }
        }
        let output = estimate_phase_variance(&samples, recipe).unwrap();
        assert_eq!(output.raw, vec![2.0, 2.0], "offset={offset}");
    }
}

// M3 review F7: overflowing lag-product accumulators are a typed failure,
// never affirmative adequacy with zeroed statistics.
#[test]
fn adequacy_accumulator_overflow_is_rejected() {
    use std::f64::consts::TAU;
    let values: Vec<f64> = (0..256)
        .map(|i| 1e154 + if i % 2 == 0 { -1e140 } else { 1e140 })
        .collect();
    let phases: Vec<f64> = (0..256)
        .map(|i| TAU * ((i % 8) as f64 + 0.5) / 8.0)
        .collect();
    let group = AdequacyGroup {
        standardized: values,
        phases,
    };
    let policy = AdequacyPolicy {
        lags: 4,
        min_pairs_per_cell: 10,
        max_phase_spread: 0.2,
        max_reserved_shift: 0.2,
    };
    assert_eq!(
        scs_adequacy(
            std::slice::from_ref(&group),
            std::slice::from_ref(&group),
            policy
        )
        .unwrap_err()
        .code(),
        "non_finite_output"
    );
}
