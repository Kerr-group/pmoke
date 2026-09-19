//! Uncertainty-aware calibration applicability tolerance (Issue #274).
//!
//! Pure-core evidence for the combination rule
//! `tol = max(1e-9, 3 * sqrt(u_build^2 + u_apply^2))`: bound-formula units
//! (floor, coverage factor, root-sum-square, absent-uncertainty fallback),
//! the 5.852e-9 same-condition pair validating, the 2.5% mismatch pair
//! refusing, legacy (uncertainty-absent) behavior pinned, and the
//! build/inspect audit record. All fixtures are synthetic; no measurement
//! data enters the repository.

use pmoke_analysis_core::{
    AcquisitionMeta, ApplicabilityRequest, ArtifactRequest, BlockPlan,
    CALIBRATION_PHASE_CONVENTION, DEFAULT_DT_REL_TOL, FREQUENCY_TOL_COVERAGE_K,
    FREQUENCY_TOL_FLOOR, HeldoutReport, ModelBinding, PhaseVarianceOutput, PhaseVarianceRecipe,
    TuningMode, build_artifact, effective_frequency_rel_tol, frequency_rel_tol_bound,
    inspect_applicability, resolve_build_frequency_tol,
};
use std::collections::BTreeMap;

const F_REF: f64 = 1_161_611.04;
const DT: f64 = 1e-9;

fn close(a: f64, b: f64, tolerance: f64, context: &str) {
    assert!(
        (a - b).abs() <= tolerance,
        "{context}: {a} vs {b} (tol {tolerance})"
    );
}

/// Minimal synthetic artifact with a prescribed binding tolerance and an
/// optional recorded build-side uncertainty.
fn build_pair_artifact(
    frequency_rel_tol: f64,
    u_build: Option<f64>,
) -> pmoke_analysis_core::CalibrationArtifact {
    let binding = ModelBinding {
        channel: 3,
        voltage_unit: "V".to_string(),
        adc_scale_provenance: None,
        sample_interval_s: DT,
        recorded_original_dt_s: Some(DT),
        sample_interval_rel_tol: DEFAULT_DT_REL_TOL,
        reference_frequency_hz: F_REF,
        frequency_rel_tol,
        reference_frequency_rel_uncertainty: u_build,
        phase_convention: CALIBRATION_PHASE_CONVENTION.to_string(),
        acquisition: AcquisitionMeta {
            device: None,
            gain: None,
            bandwidth_hz: None,
        },
    };
    let bins = 8;
    let request = ArtifactRequest {
        model_id: "tol-fixture".to_string(),
        pmoke_version: "0.0.0-test".to_string(),
        backend_versions: BTreeMap::new(),
        binding,
        variance: PhaseVarianceOutput {
            variances: vec![1.0; bins],
            v0: 1.0,
            counts: vec![64; bins],
            distinct_cycles: vec![64; bins],
            contributing_blocks: 8,
            raw: vec![1.0; bins],
            smoothed: vec![1.0; bins],
            floor_activations: Vec::new(),
        },
        correlation: None,
        recipe: PhaseVarianceRecipe {
            bins,
            min_samples_per_bin: 8,
            min_cycles_per_bin: 2,
            min_contributing_blocks: 8,
            shrinkage_alpha: 0.1,
            floor_ratio: 0.05,
        },
        correlation_recipe: None,
        plan: BlockPlan {
            intervals: Vec::new(),
            blocks: Vec::new(),
            exclusions: Vec::new(),
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
    build_artifact(request).unwrap().artifact
}

fn context_at(frequency_hz: f64, u_apply: Option<f64>) -> ApplicabilityRequest {
    ApplicabilityRequest {
        channel: 3,
        sample_interval_s: DT,
        reference_frequency_hz: frequency_hz,
        voltage_unit: "V".to_string(),
        phase_convention: CALIBRATION_PHASE_CONVENTION.to_string(),
        adc_scale_provenance: None,
        acquisition: AcquisitionMeta {
            device: None,
            gain: None,
            bandwidth_hz: None,
        },
        reference_frequency_rel_uncertainty: u_apply,
    }
}

// ---------------------------------------------------------------------------
// Bound-formula units (FR-06).
// ---------------------------------------------------------------------------

#[test]
fn bound_falls_back_to_floor_when_both_sides_absent() {
    assert_eq!(frequency_rel_tol_bound(None, None), FREQUENCY_TOL_FLOOR);
    assert_eq!(frequency_rel_tol_bound(None, None), 1e-9);
}

#[test]
fn bound_applies_coverage_factor_to_single_side() {
    // k * u with the other side absent: 3 * 1e-9.
    close(
        frequency_rel_tol_bound(Some(1e-9), None),
        3e-9,
        1e-15,
        "single-side k scaling",
    );
    close(
        frequency_rel_tol_bound(None, Some(2e-9)),
        6e-9,
        1e-15,
        "apply-side k scaling",
    );
}

#[test]
fn bound_combines_sides_root_sum_square() {
    // 3 * sqrt(2) * 2e-9.
    let expected = FREQUENCY_TOL_COVERAGE_K * (2.0f64 * 2e-9 * 2e-9).sqrt();
    close(
        frequency_rel_tol_bound(Some(2e-9), Some(2e-9)),
        expected,
        1e-18,
        "RSS combination",
    );
    // Asymmetric sides: 3 * 5e-9 from u_build=3e-9, u_apply=4e-9.
    close(
        frequency_rel_tol_bound(Some(3e-9), Some(4e-9)),
        15e-9,
        1e-18,
        "asymmetric RSS",
    );
}

#[test]
fn bound_ignores_unusable_uncertainties() {
    for bad in [f64::NAN, f64::INFINITY, -1.0, 0.0] {
        assert_eq!(
            frequency_rel_tol_bound(Some(bad), None),
            FREQUENCY_TOL_FLOOR,
            "bad build u must fall back to floor"
        );
        assert_eq!(
            frequency_rel_tol_bound(None, Some(bad)),
            FREQUENCY_TOL_FLOOR,
            "bad apply u must fall back to floor"
        );
    }
    // One good side still widens when the other is unusable.
    close(
        frequency_rel_tol_bound(Some(f64::NAN), Some(2e-9)),
        6e-9,
        1e-15,
        "good side survives bad side",
    );
}

#[test]
fn bound_never_resolves_below_floor() {
    close(
        frequency_rel_tol_bound(Some(1e-12), Some(1e-12)),
        FREQUENCY_TOL_FLOOR,
        0.0,
        "tiny uncertainties stay at floor",
    );
}

#[test]
fn effective_tol_never_tightens_below_stored_or_bound() {
    // Legacy stored tol with no uncertainties: exact historical gate.
    assert_eq!(effective_frequency_rel_tol(1e-9, None, None), 1e-9);
    // Widening through either side.
    close(
        effective_frequency_rel_tol(1e-9, Some(2e-9), Some(2e-9)),
        frequency_rel_tol_bound(Some(2e-9), Some(2e-9)),
        0.0,
        "combination widens stored floor",
    );
    // A looser stored tol is honored, never tightened.
    assert_eq!(effective_frequency_rel_tol(1e-6, Some(2e-9), None), 1e-6);
    // A stored tol below the measured basis widens to the basis.
    close(
        effective_frequency_rel_tol(2e-9, Some(2e-9), Some(2e-9)),
        frequency_rel_tol_bound(Some(2e-9), Some(2e-9)),
        0.0,
        "measured basis wins over tight stored tol",
    );
    // Degenerate stored tol falls back to the combination.
    assert_eq!(
        effective_frequency_rel_tol(f64::NAN, Some(2e-9), None),
        frequency_rel_tol_bound(Some(2e-9), None)
    );
}

#[test]
fn resolve_build_tol_combines_or_honors_override() {
    let (tol, overridden) = resolve_build_frequency_tol(Some(2e-9), None).unwrap();
    assert!(!overridden);
    close(tol, 6e-9, 1e-18, "build-side combination");
    let (tol, overridden) = resolve_build_frequency_tol(None, None).unwrap();
    assert!(!overridden);
    assert_eq!(tol, 1e-9);
    let (tol, overridden) = resolve_build_frequency_tol(Some(2e-9), Some(1e-7)).unwrap();
    assert!(overridden);
    assert_eq!(tol, 1e-7);
    for bad in [0.0, -1e-9, f64::NAN, f64::INFINITY] {
        assert!(
            resolve_build_frequency_tol(None, Some(bad)).is_err(),
            "override {bad} must be rejected"
        );
    }
}

// ---------------------------------------------------------------------------
// Acceptance pairs (FR-06): 5.852e-9 validates, 2.5% refuses.
// ---------------------------------------------------------------------------

/// Per-side uncertainties in the measured reproducibility band: the
/// real-data anchor (same-setting inter-run spread 2.9e-9..5.6e-9)
/// supports per-side standard uncertainties around 2e-9, for which
/// 3 * sqrt(2) * 2e-9 = 8.49e-9 covers the 5.852e-9 pair.
const ANCHORED_U: f64 = 2e-9;

#[test]
fn same_condition_pair_at_5_852e_9_validates() {
    let stored = frequency_rel_tol_bound(Some(ANCHORED_U), None);
    let artifact = build_pair_artifact(stored, Some(ANCHORED_U));
    let request = context_at(F_REF * (1.0 + 5.852e-9), Some(ANCHORED_U));
    let report = inspect_applicability(&artifact, &request);
    assert!(
        report.compatible,
        "5.852e-9 pair must validate, errors: {:?}",
        report.errors
    );
    // The audit record carries the widened basis.
    let expected = FREQUENCY_TOL_COVERAGE_K * (2.0f64 * ANCHORED_U * ANCHORED_U).sqrt();
    close(
        report.effective_frequency_rel_tol,
        expected,
        1e-18,
        "effective tol audit",
    );
    assert_eq!(report.applied_build_rel_uncertainty, Some(ANCHORED_U));
    assert_eq!(report.applied_apply_rel_uncertainty, Some(ANCHORED_U));
}

#[test]
fn mismatch_pair_at_2_5_percent_refuses() {
    let stored = frequency_rel_tol_bound(Some(ANCHORED_U), None);
    let artifact = build_pair_artifact(stored, Some(ANCHORED_U));
    let request = context_at(F_REF * 1.025, Some(ANCHORED_U));
    let report = inspect_applicability(&artifact, &request);
    assert!(
        !report.compatible,
        "2.5% pair must refuse despite recorded uncertainties"
    );
    assert_eq!(report.errors[0].code, "model_binding_mismatch");
}

#[test]
fn legacy_pair_without_uncertainties_keeps_historical_gate() {
    // Uncertainty-absent artifact and request: the 5.852e-9 pair still
    // refuses exactly as before the change (the reported defect), while
    // an identical-frequency pair validates.
    let artifact = build_pair_artifact(1e-9, None);
    let deviated = context_at(F_REF * (1.0 + 5.852e-9), None);
    assert!(!inspect_applicability(&artifact, &deviated).compatible);
    let identical = context_at(F_REF, None);
    let report = inspect_applicability(&artifact, &identical);
    assert!(report.compatible);
    assert_eq!(report.effective_frequency_rel_tol, 1e-9);
    assert_eq!(report.applied_build_rel_uncertainty, None);
    assert_eq!(report.applied_apply_rel_uncertainty, None);
}

// ---------------------------------------------------------------------------
// Audit record: artifact carries the tol basis.
// ---------------------------------------------------------------------------

#[test]
fn artifact_records_tol_basis_in_recipe_settings() {
    let stored = frequency_rel_tol_bound(Some(ANCHORED_U), None);
    let artifact = build_pair_artifact(stored, Some(ANCHORED_U));
    assert_eq!(artifact.binding.frequency_rel_tol, stored);
    assert_eq!(
        artifact.binding.reference_frequency_rel_uncertainty,
        Some(ANCHORED_U)
    );
    let settings = &artifact.builder.recipe_settings;
    assert_eq!(
        settings.get("frequency_tol_k"),
        Some(&FREQUENCY_TOL_COVERAGE_K.to_string())
    );
    assert_eq!(
        settings.get("frequency_tol_floor"),
        Some(&FREQUENCY_TOL_FLOOR.to_string())
    );
    assert_eq!(
        settings.get("frequency_tol_u_build"),
        Some(&ANCHORED_U.to_string())
    );
    assert_eq!(
        settings.get("frequency_tol_u_apply"),
        Some(&"absent".to_string())
    );
    assert_eq!(
        settings.get("frequency_tol_overridden"),
        Some(&"false".to_string())
    );
}
