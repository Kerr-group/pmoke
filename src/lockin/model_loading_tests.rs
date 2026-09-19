use super::*;
use crate::config::{EstimatorCalibration, GlsNoiseMode};
use crate::lockin::joint::NoiseModelSource;

const CHANNEL: u8 = 3;
const DT: f64 = 1.0e-5;
const F_REF: f64 = 1_000.0;

/// Minimal valid v1 artifact JSON for channel 3, dt 1e-5, f_ref 1 kHz.
fn artifact_json() -> String {
    r#"{

  "schema_version": 1,
  "algorithm_version": "pmoke-calibration-v1",
  "model_id": "test-model-ch3",
  "builder": {
    "pmoke_version": "0.4.1",
    "backend_versions": {},
    "recipe_settings": {}
  },
  "binding": {
    "channel": 3,
    "voltage_unit": "V",
    "adc_scale_provenance": null,
    "sample_interval_s": 0.00001,
    "recorded_original_dt_s": null,
    "sample_interval_rel_tol": 1e-9,
    "reference_frequency_hz": 1000.0,
    "frequency_rel_tol": 1e-9,
    "phase_convention": "phi=2*pi*f*t-reference_phase_rad",
    "acquisition": {"device": null, "gain": null, "bandwidth_hz": null}
  },
  "reference_variance_v2": 0.01,
  "phase": {
    "bins": 4,
    "center_convention": "bin_centers_at_2pi*(i+0.5)/bins",
    "variances_v2": [0.01, 0.012, 0.011, 0.009],
    "interpolation": "periodic_linear_variance_v1"
  },
  "correlation": {
    "lags": [1.0, 0.5, 0.25],
    "lag_step_s": 0.00001,
    "support_samples": 3,
    "taper": "bartlett_v1",
    "shrinkage_eta": 0.01,
    "spd_validated": true,
    "tail_energy": null,
    "max_tail_lag": 2
  },
  "regularization": {
    "smoothing": "none",
    "shrinkage_alpha_v": 0.0,
    "variance_floor_ratio": 0.0,
    "floor_activations": [],
    "correlation_taper": "bartlett_v1",
    "correlation_eta": 0.01
  },
  "training": {
    "source_digests": [],
    "intervals": [],
    "blocks": [],
    "exclusions": [],
    "seed": 7,
    "per_bin_counts": [4, 4, 4, 4],
    "per_bin_cycles": [2, 2, 2, 2],
    "contributing_blocks": 1
  },
  "validation": {
    "heldout_blocks": 0,
    "profile_rmse_v2": null,
    "standardized_lag1": null,
    "dof_treatment": "none",
    "nuisance_params_per_block": 0,
    "limits": []
  },
  "capabilities": {
    "modes": ["identity", "phase_diagonal", "phase_correlated"],
    "geometry_restrictions": []
  }
}"#
    .to_string()
}

fn write_case(dir: &std::path::Path, name: &str, bytes: &[u8]) -> (PathBuf, String) {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    let digest = crate::utils::checksum::sha256_hex(bytes);
    (path, digest)
}

fn source_for(dir: &std::path::Path, digest: &str, bytes_name: &str) -> FileNoiseModelSource {
    FileNoiseModelSource::new(
        dir.to_path_buf(),
        &[EstimatorCalibration {
            channel: CHANNEL,
            path: bytes_name.to_string(),
            sha256: digest.to_string(),
        }],
        PIPELINE_VOLTAGE_UNIT.to_string(),
        DT,
        F_REF,
        // Fixture artifacts predate uncertainty recording: floor gate.
        None,
    )
    .unwrap()
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmoke_model_loading_{name}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn file_source_loads_identity_and_phase_diagonal() {
    let dir = temp_dir("ok");
    let json = artifact_json();
    let (_, digest) = write_case(&dir, "ch3.json", json.as_bytes());
    let source = source_for(&dir, &digest, "ch3.json");
    let (model, binding) = source.load(CHANNEL, GlsNoiseMode::Identity).unwrap();
    assert_eq!(model.reference_variance_v2, 0.01);
    assert!(model.variance_bins.is_none());
    assert!(model.correlation.is_none());
    assert_eq!(binding.channel, CHANNEL);
    assert_eq!(binding.sha256, digest);
    let (model, _) = source.load(CHANNEL, GlsNoiseMode::PhaseDiagonal).unwrap();
    assert_eq!(
        model.variance_bins.unwrap(),
        vec![0.01, 0.012, 0.011, 0.009]
    );
    let (model, _) = source.load(CHANNEL, GlsNoiseMode::PhaseCorrelated).unwrap();
    assert!(model.correlation.is_some());
    // Stationary bridges on the SPD-validated tapered table.
    let (model, _) = source
        .load(CHANNEL, GlsNoiseMode::StationaryCorrelated)
        .unwrap();
    assert!(model.variance_bins.is_none());
    assert!(model.correlation.is_some());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn malformed_expected_digest_fails_before_file_access() {
    let dir = temp_dir("digest");
    let result = FileNoiseModelSource::new(
        dir.clone(),
        &[EstimatorCalibration {
            channel: CHANNEL,
            path: "missing.json".to_string(),
            sha256: "${CALIBRATION_SHA256}".to_string(),
        }],
        PIPELINE_VOLTAGE_UNIT.to_string(),
        DT,
        F_REF,
        None,
    );
    assert!(result.is_err());
    for bad in ["abc", &"A".repeat(64), &"g".repeat(64), ""] {
        assert!(expect_digest_format(bad).is_err(), "{bad}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn changed_bytes_after_binding_are_a_hard_mismatch() {
    let dir = temp_dir("changed");
    let json = artifact_json();
    let (path, digest) = write_case(&dir, "ch3.json", json.as_bytes());
    let source = source_for(&dir, &digest, "ch3.json");
    source.load(CHANNEL, GlsNoiseMode::Identity).unwrap();
    // Flip one byte in place: the frozen buffer hashes differently.
    let mut bytes = std::fs::read(&path).unwrap();
    let position = bytes.iter().position(|byte| *byte == b'7').unwrap();
    bytes[position] = b'8';
    std::fs::write(&path, &bytes).unwrap();
    let error = source.load(CHANNEL, GlsNoiseMode::Identity).err().unwrap();
    assert!(
        format!("{error:#}").contains("digest mismatch"),
        "{error:#}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn duplicate_keys_are_rejected() {
    let dir = temp_dir("dupkeys");
    let mut json = artifact_json();
    json = json.replacen(
        "\"reference_variance_v2\": 0.01,",
        "\"reference_variance_v2\": 0.01,\n  \"reference_variance_v2\": 0.02,",
        1,
    );
    let (_, digest) = write_case(&dir, "ch3.json", json.as_bytes());
    let source = source_for(&dir, &digest, "ch3.json");
    let error = source.load(CHANNEL, GlsNoiseMode::Identity).err().unwrap();
    assert!(
        format!("{error:#}").contains("duplicate object key"),
        "{error:#}"
    );
    // Nested duplicates are caught too.
    let nested = r#"{"a": {"b": 1, "b": 2}}"#;
    assert!(reject_duplicate_keys(nested.as_bytes()).is_err());
    assert!(reject_duplicate_keys(br#"{"a": [1, {"b": 1}]}"#).is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn oversized_artifacts_are_rejected_before_parsing() {
    let dir = temp_dir("oversized");
    let big = vec![b' '; MAX_CALIBRATION_ARTIFACT_BYTES + 1];
    let path = dir.join("big.json");
    std::fs::write(&path, &big).unwrap();
    assert!(read_frozen_artifact(&path).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn wrong_binding_fields_fail() {
    let dir = temp_dir("binding");
    // Wrong channel.
    let json = artifact_json().replacen("\"channel\": 3,", "\"channel\": 4,", 1);
    let (_, digest) = write_case(&dir, "ch3.json", json.as_bytes());
    let source = source_for(&dir, &digest, "ch3.json");
    let error = source.load(CHANNEL, GlsNoiseMode::Identity).err().unwrap();
    assert!(format!("{error:#}").contains("not applicable"), "{error:#}");
    // Wrong reference frequency.
    let json = artifact_json().replacen(
        "\"reference_frequency_hz\": 1000.0,",
        "\"reference_frequency_hz\": 2000.0,",
        1,
    );
    let (_, digest) = write_case(&dir, "ch3b.json", json.as_bytes());
    let source = source_for(&dir, &digest, "ch3b.json");
    assert!(
        format!(
            "{:?}",
            source.load(CHANNEL, GlsNoiseMode::Identity).err().unwrap()
        )
        .contains("not applicable")
    );
    // Wrong units.
    let json = artifact_json().replacen("\"voltage_unit\": \"V\",", "\"voltage_unit\": \"mV\",", 1);
    let (_, digest) = write_case(&dir, "ch3c.json", json.as_bytes());
    let source = source_for(&dir, &digest, "ch3c.json");
    assert!(source.load(CHANNEL, GlsNoiseMode::Identity).is_err());
    // Wrong dt via source construction (run at 2x rate).
    let json = artifact_json();
    let (_, digest) = write_case(&dir, "ch3d.json", json.as_bytes());
    let source = FileNoiseModelSource::new(
        dir.clone(),
        &[EstimatorCalibration {
            channel: CHANNEL,
            path: "ch3d.json".to_string(),
            sha256: digest,
        }],
        PIPELINE_VOLTAGE_UNIT.to_string(),
        2.0 * DT,
        F_REF,
        None,
    )
    .unwrap();
    assert!(source.load(CHANNEL, GlsNoiseMode::Identity).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unknown_schema_and_conventions_fail() {
    let dir = temp_dir("schema");
    // Unknown schema version.
    let json = artifact_json().replacen("\"schema_version\": 1,", "\"schema_version\": 2,", 1);
    let (_, digest) = write_case(&dir, "ch3.json", json.as_bytes());
    let source = source_for(&dir, &digest, "ch3.json");
    assert!(
        format!(
            "{error:#}",
            error = source.load(CHANNEL, GlsNoiseMode::Identity).err().unwrap()
        )
        .contains("unsupported schema_version")
    );
    // Unknown top-level field (deny_unknown_fields).
    let json = artifact_json().replacen(
        "\"model_id\": \"test-model-ch3\",",
        "\"model_id\": \"test-model-ch3\",\n  \"future_field\": true,",
        1,
    );
    let (_, digest) = write_case(&dir, "ch3b.json", json.as_bytes());
    let source = source_for(&dir, &digest, "ch3b.json");
    assert!(source.load(CHANNEL, GlsNoiseMode::Identity).is_err());
    // Unknown phase convention.
    let json = artifact_json().replacen(
        "\"phase_convention\": \"phi=2*pi*f*t-reference_phase_rad\",",
        "\"phase_convention\": \"other\",",
        1,
    );
    let (_, digest) = write_case(&dir, "ch3c.json", json.as_bytes());
    let source = source_for(&dir, &digest, "ch3c.json");
    assert!(source.load(CHANNEL, GlsNoiseMode::Identity).is_err());
    // Unknown interpolation.
    let json = artifact_json().replacen(
        "\"interpolation\": \"periodic_linear_variance_v1\"",
        "\"interpolation\": \"spline\"",
        1,
    );
    let (_, digest) = write_case(&dir, "ch3d.json", json.as_bytes());
    let source = source_for(&dir, &digest, "ch3d.json");
    assert!(source.load(CHANNEL, GlsNoiseMode::PhaseDiagonal).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn missing_components_and_scaled_channels_fail() {
    let dir = temp_dir("components");
    let json = artifact_json();
    let (_, digest) = write_case(&dir, "ch3.json", json.as_bytes());
    // Corrupted variances (negative bin).
    let bad = json.replacen("0.012", "-0.5", 1);
    let (_, bad_digest) = write_case(&dir, "bad.json", bad.as_bytes());
    let source = source_for(&dir, &bad_digest, "bad.json");
    assert!(source.load(CHANNEL, GlsNoiseMode::PhaseDiagonal).is_err());
    // Missing channel binding.
    let source = source_for(&dir, &digest, "ch3.json");
    assert!(source.load(9, GlsNoiseMode::Identity).is_err());
    // Duplicate bindings rejected at construction.
    assert!(
        FileNoiseModelSource::new(
            dir.clone(),
            &[
                EstimatorCalibration {
                    channel: CHANNEL,
                    path: "ch3.json".to_string(),
                    sha256: "0".repeat(64),
                },
                EstimatorCalibration {
                    channel: CHANNEL,
                    path: "ch3.json".to_string(),
                    sha256: "0".repeat(64),
                },
            ],
            PIPELINE_VOLTAGE_UNIT.to_string(),
            DT,
            F_REF,
            None,
        )
        .is_err()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unvalidated_correlation_is_rejected_for_correlated_modes() {
    let dir = temp_dir("spd");
    let json = artifact_json().replacen("\"spd_validated\": true,", "\"spd_validated\": false,", 1);
    let (_, digest) = write_case(&dir, "ch3.json", json.as_bytes());
    let source = source_for(&dir, &digest, "ch3.json");
    // Identity still loads (no correlation needed); correlated modes refuse.
    source.load(CHANNEL, GlsNoiseMode::Identity).unwrap();
    assert!(source.load(CHANNEL, GlsNoiseMode::PhaseCorrelated).is_err());
    assert!(
        source
            .load(CHANNEL, GlsNoiseMode::StationaryCorrelated)
            .is_err()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn uncertainty_combination_admits_ppb_pair_and_refuses_mismatch() {
    use pmoke_analysis_core::calibration::{CalibrationArtifact, frequency_rel_tol_bound};
    // Artifact with a recorded build-side uncertainty (tol widened at
    // build through the combination rule, Issue #274 FR-01).
    let mut artifact: CalibrationArtifact = serde_json::from_str(&artifact_json()).unwrap();
    artifact.binding.reference_frequency_rel_uncertainty = Some(2e-9);
    artifact.binding.frequency_rel_tol = frequency_rel_tol_bound(Some(2e-9), None);
    // Same-condition pair at 5.852e-9 with inference-side uncertainty
    // (Issue #274 FR-02/FR-06): the combination covers it.
    let (model, _) = noise_model_from_artifact(
        &artifact,
        CHANNEL,
        GlsNoiseMode::Identity,
        DT,
        F_REF * (1.0 + 5.852e-9),
        PIPELINE_VOLTAGE_UNIT,
        Some(2e-9),
    )
    .unwrap();
    assert!(model.reference_variance_v2 > 0.0);
    // The stored build-side widening alone already admits the pair.
    noise_model_from_artifact(
        &artifact,
        CHANNEL,
        GlsNoiseMode::Identity,
        DT,
        F_REF * (1.0 + 5.852e-9),
        PIPELINE_VOLTAGE_UNIT,
        None,
    )
    .unwrap();
    // A true 2.5% mismatch still refuses (FR-06).
    assert!(
        noise_model_from_artifact(
            &artifact,
            CHANNEL,
            GlsNoiseMode::Identity,
            DT,
            F_REF * 1.025,
            PIPELINE_VOLTAGE_UNIT,
            Some(2e-9),
        )
        .is_err()
    );
}

#[test]
fn joint_end_to_end_through_run_li() {
    use crate::config::{
        GlsCovarianceOutput, GlsFailurePolicy, GlsNoiseMode, JointHarmonicGlsConfig,
    };
    use crate::config::{LockinEstimator, LockinWindow};
    use crate::lockin::run_li;
    use crate::test_support::test_config;
    use crate::utils::time_axis::TimeAxisRef;
    use std::f64::consts::PI;

    let dir = temp_dir("e2e");
    let json = artifact_json();
    let (_, digest) = write_case(&dir, "ch3.json", json.as_bytes());

    let mut cfg = test_config(vec![1], vec![3]);
    cfg.roles.reference_ch = 2;
    cfg.source_path = dir.join("config.toml");
    cfg.set_artifact_root(dir.clone());
    cfg.lockin.workers = 1;
    cfg.lockin.stride_samples = 10;
    cfg.lockin.lpf_half_window_cycles = 1.0;
    cfg.lockin.window = LockinWindow::legacy_boxcar(1.0);
    cfg.lockin.estimator = LockinEstimator::JointHarmonicGls(JointHarmonicGlsConfig {
        fit_harmonics: (1..=12).collect(),
        output_harmonics: (1..=6).collect(),
        envelope_degree: 0,
        noise_mode: GlsNoiseMode::Identity,
        covariance_output: GlsCovarianceOutput::Diagonal,
        failure_policy: GlsFailurePolicy::Error,
        calibration_source: crate::config::GlsCalibrationSource::Artifact,
        calibrations: vec![EstimatorCalibration {
            channel: CHANNEL,
            path: "ch3.json".to_string(),
            sha256: digest,
        }],
    });

    let dt = DT;
    let samples = 3_000usize;
    let time: Vec<f64> = (0..samples).map(|index| index as f64 * dt).collect();
    let sensor: Vec<f64> = time.iter().map(|t| 0.001 * t).collect();
    let reference: Vec<f64> = time.iter().map(|t| (2.0 * PI * F_REF * t).sin()).collect();
    let signal: Vec<f64> = time
        .iter()
        .map(|t| (2.0 * PI * F_REF * t + 0.3).sin() + 0.2 * (2.0 * PI * 3.0 * F_REF * t).sin())
        .collect();
    let data = vec![sensor, reference, signal];

    let (_, _, _, li_results, _, provenance) =
        run_li(&cfg, TimeAxisRef::Explicit(&time), &data).unwrap();
    assert_eq!(li_results.len(), 1);
    assert_eq!(li_results[0].len(), 12);
    let encoded = serde_json::to_value(&provenance).unwrap();
    assert_eq!(encoded["kind"], "joint_harmonic_gls");

    let paths = cfg.paths();
    let quality = std::fs::read_to_string(paths.lockin_quality_csv(3)).unwrap();
    assert!(
        quality.starts_with("original_center_index,time_s,"),
        "{quality}"
    );
    let covariance = std::fs::read_to_string(paths.lockin_covariance_csv(3)).unwrap();
    assert!(
        covariance.starts_with("time_s,cov_x1_x1_v2,"),
        "{covariance}"
    );
    assert!(paths.lockin_covariance_npy(3).is_file());
    assert!(paths.lockin_xy_csv(3).is_file());
    let snapshot = std::fs::read_to_string(paths.lockin_estimator_json(3)).unwrap();
    let snapshot_value: serde_json::Value = serde_json::from_str(&snapshot).unwrap();
    assert_eq!(snapshot_value["channel"], 3);
    assert_eq!(snapshot_value["solver"]["id"], "joint-direct-qr/1");
    assert_eq!(snapshot_value["noise_model"]["mode"], "identity");
    assert_eq!(
        snapshot_value["covariance"]["covariance_mode"],
        "design_model"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn loader_tail_metadata_accepts_integer_max_without_panicking() {
    let dir = temp_dir("tail_integer_max");
    let mut artifact: serde_json::Value = serde_json::from_str(&artifact_json()).unwrap();
    artifact["correlation"]["max_tail_lag"] = serde_json::json!(usize::MAX);
    let bytes = serde_json::to_vec(&artifact).unwrap();
    let (_, digest) = write_case(&dir, "ch3.json", &bytes);
    let source = source_for(&dir, &digest, "ch3.json");
    // A longer declared diagnostic horizon is not an addition request.
    // Validate its ordering without overflowing at the integer boundary.
    assert!(source.load(CHANNEL, GlsNoiseMode::PhaseCorrelated).is_ok());
    artifact["correlation"]["max_tail_lag"] = serde_json::json!(1);
    let bytes = serde_json::to_vec(&artifact).unwrap();
    let (_, digest) = write_case(&dir, "short.json", &bytes);
    let source = source_for(&dir, &digest, "short.json");
    assert!(source.load(CHANNEL, GlsNoiseMode::PhaseCorrelated).is_err());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn incompatible_artifact_contents_fail_after_hashing() {
    // A correct digest authenticates the bytes but does not make their
    // contents valid: every case below carries a fresh correct digest.
    let dir = temp_dir("contents");
    // Unknown algorithm version.
    let json = artifact_json().replacen(
        "\"algorithm_version\": \"pmoke-calibration-v1\",",
        "\"algorithm_version\": \"future-incompatible-v999\",",
        1,
    );
    let (_, digest) = write_case(&dir, "ch3.json", json.as_bytes());
    let source = source_for(&dir, &digest, "ch3.json");
    assert!(
        format!(
            "{error:#}",
            error = source.load(CHANNEL, GlsNoiseMode::Identity).err().unwrap()
        )
        .contains("unknown algorithm_version")
    );
    // Declared bin count disagrees with the carried variances.
    let json = artifact_json().replacen("\"bins\": 4,", "\"bins\": 999,", 1);
    let (_, digest) = write_case(&dir, "ch3b.json", json.as_bytes());
    let source = source_for(&dir, &digest, "ch3b.json");
    assert!(source.load(CHANNEL, GlsNoiseMode::Identity).is_err());
    // Unknown nested binding field (nested deny_unknown_fields).
    let json = artifact_json().replacen(
        "\"acquisition\": {\"device\": null, \"gain\": null, \"bandwidth_hz\": null}",
        "\"acquisition\": {\"device\": null, \"gain\": null, \"bandwidth_hz\": null, \"future\": 1}",
        1,
    );
    let (_, digest) = write_case(&dir, "ch3c.json", json.as_bytes());
    let source = source_for(&dir, &digest, "ch3c.json");
    assert!(source.load(CHANNEL, GlsNoiseMode::Identity).is_err());
    // Out-of-range correlation eta agreed by both records.
    let json = artifact_json()
        .replacen("\"shrinkage_eta\": 0.01,", "\"shrinkage_eta\": 2.0,", 1)
        .replacen("\"correlation_eta\": 0.01", "\"correlation_eta\": 2.0", 1);
    let (_, digest) = write_case(&dir, "ch3d.json", json.as_bytes());
    let source = source_for(&dir, &digest, "ch3d.json");
    assert!(source.load(CHANNEL, GlsNoiseMode::PhaseCorrelated).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn retained_calibration_survives_external_removal() {
    use crate::config::{
        GlsCovarianceOutput, GlsFailurePolicy, GlsNoiseMode, JointHarmonicGlsConfig,
    };
    use crate::config::{LockinEstimator, LockinWindow};
    use crate::lockin::run_li;
    use crate::test_support::test_config;
    use crate::utils::time_axis::TimeAxisRef;
    use std::f64::consts::PI;

    // v4 retention: the exact validated artifact bytes are persisted under
    // analysis/lockin/ch3_calibration.json, so the run stays verifiable
    // after the external model file is removed.
    let dir = temp_dir("retained");
    let json = artifact_json();
    let (_, digest) = write_case(&dir, "ch3.json", json.as_bytes());

    let mut cfg = test_config(vec![1], vec![3]);
    cfg.roles.reference_ch = 2;
    cfg.source_path = dir.join("config.toml");
    cfg.set_artifact_root(dir.clone());
    cfg.lockin.workers = 1;
    cfg.lockin.stride_samples = 10;
    cfg.lockin.lpf_half_window_cycles = 1.0;
    cfg.lockin.window = LockinWindow::legacy_boxcar(1.0);
    cfg.lockin.estimator = LockinEstimator::JointHarmonicGls(JointHarmonicGlsConfig {
        fit_harmonics: (1..=12).collect(),
        output_harmonics: (1..=6).collect(),
        envelope_degree: 0,
        noise_mode: GlsNoiseMode::Identity,
        covariance_output: GlsCovarianceOutput::Diagonal,
        failure_policy: GlsFailurePolicy::Error,
        calibration_source: crate::config::GlsCalibrationSource::Artifact,
        calibrations: vec![EstimatorCalibration {
            channel: CHANNEL,
            path: "ch3.json".to_string(),
            sha256: digest.clone(),
        }],
    });

    let dt = DT;
    let samples = 3_000usize;
    let time: Vec<f64> = (0..samples).map(|index| index as f64 * dt).collect();
    let sensor: Vec<f64> = time.iter().map(|t| 0.001 * t).collect();
    let reference: Vec<f64> = time.iter().map(|t| (2.0 * PI * F_REF * t).sin()).collect();
    let signal: Vec<f64> = time
        .iter()
        .map(|t| (2.0 * PI * F_REF * t + 0.3).sin())
        .collect();
    let data = vec![sensor, reference, signal];

    run_li(&cfg, TimeAxisRef::Explicit(&time), &data).unwrap();
    let paths = cfg.paths();
    let retained = paths.lockin_calibration_json(3);
    assert!(retained.is_file());
    // Byte identity with the external source plus digest agreement.
    assert_eq!(std::fs::read(&retained).unwrap(), json.as_bytes());
    assert_eq!(
        crate::utils::checksum::file_sha256(&retained).unwrap(),
        digest
    );
    // Manifest registration through the real consumer, with schema v4.
    std::fs::write(paths.analysis_manifest(), "schema_version = 3\n").unwrap();
    std::fs::write(paths.analysis_source_config(), b"version = 3\n").unwrap();
    std::fs::write(paths.analysis_resolved_config(), b"version = 3\n").unwrap();
    crate::lockin::provenance::refresh_analysis_manifest_outputs(&cfg, "li").unwrap();
    let manifest = std::fs::read_to_string(paths.analysis_manifest()).unwrap();
    let value: toml::Value = toml::from_str(&manifest).unwrap();
    assert_eq!(value["schema_version"].as_integer(), Some(4));
    let artifacts = value["artifacts"].as_array().unwrap();
    let calibration = artifacts
        .iter()
        .find(|artifact| artifact["kind"].as_str() == Some("lockin_calibration"))
        .expect("lockin_calibration artifact missing");
    assert_eq!(calibration["channel"].as_integer(), Some(3));
    assert!(
        calibration["format"]
            .as_str()
            .is_some_and(|format| format == format!("json;sha256={digest}"))
    );
    // External removal changes nothing about the retained bytes.
    std::fs::remove_file(dir.join("ch3.json")).unwrap();
    assert_eq!(std::fs::read(&retained).unwrap(), json.as_bytes());
    crate::lockin::provenance::refresh_analysis_manifest_outputs(&cfg, "li").unwrap();
    let manifest = std::fs::read_to_string(paths.analysis_manifest()).unwrap();
    let value: toml::Value = toml::from_str(&manifest).unwrap();
    assert!(
        value["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|artifact| artifact["kind"].as_str() == Some("lockin_calibration"))
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
