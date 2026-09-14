use super::*;
use crate::config::{GlsCovarianceOutput, GlsNoiseMode, JointHarmonicGlsConfig};
use crate::lockin::joint::{ModelBinding, QualityRow, WindowStatus};
use crate::lockin::lockin_params::LockinParams;
use crate::lockin::model_loading::PIPELINE_VOLTAGE_UNIT;
use pmoke_analysis_core::joint::{JointSolverTolerances, NoiseMode, NoiseModel};

fn test_gls() -> JointHarmonicGlsConfig {
    JointHarmonicGlsConfig {
        fit_harmonics: (1..=12).collect(),
        output_harmonics: (1..=6).collect(),
        envelope_degree: 0,
        noise_mode: GlsNoiseMode::Identity,
        covariance_output: GlsCovarianceOutput::Diagonal,
        failure_policy: crate::config::GlsFailurePolicy::Error,
        calibrations: Vec::new(),
    }
}

fn test_params() -> LockinParams {
    LockinParams {
        dt: 1.0e-5,
        sample_rate: 100_000.0,
        output_rate: 10_000.0,
        stride: 10,
        length: 3_000,
        f_ref: 1_000.0,
        lpf_kind: crate::config::LockinLpfKind::BoxcarLegacy,
        cutoff_hz: None,
        cutoff_source: crate::lockin::lockin_params::CutoffSource::ReferenceRatio,
        fallback_used: false,
        omega: 0.0,
        t_half: 0.001,
        n_half: 100,
        i_start: 12,
        i_end: 138,
    }
}

fn test_binding() -> ModelBinding {
    ModelBinding {
        channel: 3,
        path: "calibration/ch3-noise.json".to_string(),
        sha256: "0".repeat(64),
        noise_mode: GlsNoiseMode::Identity,
    }
}

fn test_noise() -> NoiseModel {
    NoiseModel {
        mode: NoiseMode::Identity,
        reference_variance_v2: 0.01,
        variance_bins: None,
        correlation: None,
    }
}

fn quality_row(center: usize, jitter: f64) -> QualityRow {
    QualityRow {
        original_center_index: center,
        time_s: center as f64 * 1.0e-4,
        scaled_design_condition: 3.5,
        residual_rms_v: 1e-12,
        rank: 25,
        status: WindowStatus::for_jitter(jitter),
        jitter_applied_v2: jitter,
        noise_mode: "identity",
    }
}

#[test]
fn snapshot_freezes_execution_context() {
    let gls = test_gls();
    let params = test_params();
    let rows: Vec<QualityRow> = (12..=138).map(|center| quality_row(center, 0.0)).collect();
    let snapshot = build_estimator_snapshot(
        3,
        &gls,
        &test_noise(),
        &JointSolverTolerances::default(),
        crate::lockin::joint::JOINT_SOLVER_ID,
        1_000.0,
        0.0,
        1.0e-5,
        &params,
        &test_binding(),
        &rows,
    )
    .unwrap();
    assert_eq!(snapshot.schema_version, 1);
    assert_eq!(snapshot.channel, 3);
    assert_eq!(snapshot.solver.id, "joint-direct-qr/1");
    assert_eq!(snapshot.signal_model.fit_harmonics.len(), 12);
    assert_eq!(
        snapshot.signal_model.output_harmonics,
        vec![1, 2, 3, 4, 5, 6]
    );
    assert_eq!(snapshot.noise_model.mode, "identity");
    assert_eq!(snapshot.noise_model.reference_variance_v2, 0.01);
    assert_eq!(snapshot.model.sha256, "0".repeat(64));
    assert_eq!(snapshot.reference.frequency_hz, 1_000.0);
    assert_eq!(snapshot.geometry.half_window_taps, 101);
    assert_eq!(snapshot.geometry.stride_samples, 10);
    assert_eq!(snapshot.geometry.output_count, 127);
    assert_eq!(snapshot.quality_summary.windows, 127);
    assert_eq!(snapshot.quality_summary.ok, 127);
    assert_eq!(snapshot.quality_summary.warnings, 0);
    assert!(snapshot.quality_summary.warning_centers.is_empty());
    assert_eq!(snapshot.covariance.covariance_mode, "design_model");
    assert_eq!(snapshot.covariance.serialization, "diagonal");
    assert!(snapshot.covariance.conditioning.contains("conditional"));
    assert!(snapshot.covariance.conditioning.contains(&"0".repeat(64)));
    assert!(snapshot.errors.is_empty());
}

#[test]
fn snapshot_summarizes_warnings_and_jitter() {
    let gls = test_gls();
    let params = test_params();
    let mut rows: Vec<QualityRow> = (12..=138).map(|center| quality_row(center, 0.0)).collect();
    rows[5] = quality_row(17, 1e-6);
    rows[90] = quality_row(102, 2e-6);
    let snapshot = build_estimator_snapshot(
        3,
        &gls,
        &test_noise(),
        &JointSolverTolerances::default(),
        crate::lockin::joint::JOINT_SOLVER_ID,
        1_000.0,
        0.0,
        1.0e-5,
        &params,
        &test_binding(),
        &rows,
    )
    .unwrap();
    assert_eq!(snapshot.quality_summary.ok, 125);
    assert_eq!(snapshot.quality_summary.warnings, 2);
    assert_eq!(snapshot.quality_summary.warning_centers, vec![17, 102]);
    assert_eq!(snapshot.quality_summary.max_jitter_applied_v2, 2e-6);
}

#[test]
fn snapshot_round_trips_through_disk_and_manifest() {
    let gls = test_gls();
    let params = test_params();
    let rows: Vec<QualityRow> = (12..=20).map(|center| quality_row(center, 0.0)).collect();
    // Narrow the geometry view to the provided rows for this unit check.
    let mut narrow = params;
    narrow.i_end = 20;
    let snapshot = build_estimator_snapshot(
        3,
        &gls,
        &test_noise(),
        &JointSolverTolerances::default(),
        crate::lockin::joint::JOINT_SOLVER_ID,
        1_000.0,
        0.0,
        1.0e-5,
        &narrow,
        &test_binding(),
        &rows,
    )
    .unwrap();
    let dir = std::env::temp_dir().join(format!("pmoke_estimator_snapshot_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("analysis/lockin")).unwrap();
    let mut cfg = crate::test_support::test_config(vec![1], vec![3]);
    cfg.set_artifact_root(dir.clone());
    let paths = cfg.paths();
    write_estimator_json(&paths.lockin_estimator_json(3), &snapshot).unwrap();
    // Refusing to overwrite keeps a committed snapshot immutable.
    assert!(write_estimator_json(&paths.lockin_estimator_json(3), &snapshot).is_err());
    // Deterministic bytes: rebuilding from the same context matches exactly.
    let again = build_estimator_snapshot(
        3,
        &gls,
        &test_noise(),
        &JointSolverTolerances::default(),
        crate::lockin::joint::JOINT_SOLVER_ID,
        1_000.0,
        0.0,
        1.0e-5,
        &narrow,
        &test_binding(),
        &rows,
    )
    .unwrap();
    let first = std::fs::read(paths.lockin_estimator_json(3)).unwrap();
    std::fs::remove_file(paths.lockin_estimator_json(3)).unwrap();
    write_estimator_json(&paths.lockin_estimator_json(3), &again).unwrap();
    assert_eq!(
        first,
        std::fs::read(paths.lockin_estimator_json(3)).unwrap()
    );

    // Manifest registration through the real consumer.
    std::fs::write(
        paths.lockin_quality_csv(3),
        "original_center_index,time_s,scaled_design_condition,residual_rms_v,rank,status\n12,0.0,3.5,1e-12,25,ok\n",
    )
    .unwrap();
    std::fs::write(paths.analysis_manifest(), "schema_version = 3\n").unwrap();
    std::fs::write(paths.analysis_source_config(), b"version = 3\n").unwrap();
    std::fs::write(paths.analysis_resolved_config(), b"version = 3\n").unwrap();
    crate::lockin::provenance::refresh_analysis_manifest_outputs(&cfg, "li").unwrap();
    let manifest = std::fs::read_to_string(paths.analysis_manifest()).unwrap();
    let value: toml::Value = toml::from_str(&manifest).unwrap();
    let artifacts = value["artifacts"].as_array().unwrap();
    let snapshot_artifact = artifacts
        .iter()
        .find(|artifact| artifact["kind"].as_str() == Some("lockin_estimator"))
        .expect("lockin_estimator artifact missing");
    assert_eq!(snapshot_artifact["channel"].as_integer(), Some(3));
    assert_eq!(snapshot_artifact["format"].as_str(), Some("json"));
    assert!(snapshot_artifact["file"].as_str().is_some());
    let depends = snapshot_artifact["depends_on"].as_array().unwrap();
    assert!(
        depends
            .iter()
            .any(|entry| entry.as_str() == Some("lockin/ch3_quality.csv"))
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn corrupt_snapshot_fails_refresh() {
    let dir = std::env::temp_dir().join(format!(
        "pmoke_estimator_snapshot_corrupt_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("analysis/lockin")).unwrap();
    let mut cfg = crate::test_support::test_config(vec![1], vec![3]);
    cfg.set_artifact_root(dir.clone());
    let paths = cfg.paths();
    std::fs::write(paths.lockin_estimator_json(3), b"{not json").unwrap();
    std::fs::write(paths.analysis_manifest(), "schema_version = 3\n").unwrap();
    std::fs::write(paths.analysis_source_config(), b"version = 3\n").unwrap();
    std::fs::write(paths.analysis_resolved_config(), b"version = 3\n").unwrap();
    assert!(crate::lockin::provenance::refresh_analysis_manifest_outputs(&cfg, "li").is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn pipeline_unit_is_documented() {
    assert_eq!(PIPELINE_VOLTAGE_UNIT, "V");
}

/// Structural snapshot validation (M4/M5 review F3): valid JSON that is
/// not a valid snapshot must fail refresh. Each case mutates one aspect
/// of a builder-produced valid snapshot.
#[test]
fn corrupt_value_snapshot_fails_refresh() {
    fn refresh_with(text: &str) -> anyhow::Result<()> {
        let dir = std::env::temp_dir().join(format!(
            "pmoke_estimator_snapshot_value_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("analysis/lockin")).unwrap();
        let mut cfg = crate::test_support::test_config(vec![1], vec![3]);
        cfg.set_artifact_root(dir.clone());
        let paths = cfg.paths();
        std::fs::write(paths.lockin_estimator_json(3), text).unwrap();
        std::fs::write(paths.analysis_manifest(), "schema_version = 3\n").unwrap();
        std::fs::write(paths.analysis_source_config(), b"version = 3\n").unwrap();
        std::fs::write(paths.analysis_resolved_config(), b"version = 3\n").unwrap();
        let result = crate::lockin::provenance::refresh_analysis_manifest_outputs(&cfg, "li");
        std::fs::remove_dir_all(&dir).unwrap();
        result
    }

    // JSON null is syntactically valid but carries no snapshot.
    assert!(refresh_with("null").is_err());
    // A wrong-type JSON document is not a snapshot either.
    assert!(refresh_with("[1, 2, 3]").is_err());

    let gls = test_gls();
    let params = test_params();
    let rows: Vec<QualityRow> = (12..=138).map(|center| quality_row(center, 0.0)).collect();
    let snapshot = build_estimator_snapshot(
        3,
        &gls,
        &test_noise(),
        &JointSolverTolerances::default(),
        crate::lockin::joint::JOINT_SOLVER_ID,
        1_000.0,
        0.0,
        1.0e-5,
        &params,
        &test_binding(),
        &rows,
    )
    .unwrap();
    // The builder-produced snapshot registers cleanly (control).
    let mut valid = serde_json::to_value(&snapshot).expect("snapshot serializes to a JSON value");
    assert!(
        refresh_with(&serde_json::to_string_pretty(&valid).unwrap()).is_ok(),
        "builder snapshot must validate"
    );
    // Wrong channel for the filename.
    valid["channel"] = serde_json::json!(5);
    assert!(refresh_with(&serde_json::to_string_pretty(&valid).unwrap()).is_err());
    // Missing geometry record.
    let mut missing = serde_json::to_value(&snapshot).unwrap();
    missing.as_object_mut().unwrap().remove("geometry");
    assert!(refresh_with(&serde_json::to_string_pretty(&missing).unwrap()).is_err());
    // Malformed model receipt digest.
    let mut receipt = serde_json::to_value(&snapshot).unwrap();
    receipt["model"]["sha256"] = serde_json::json!("not-a-digest");
    assert!(refresh_with(&serde_json::to_string_pretty(&receipt).unwrap()).is_err());
    // Unknown top-level field.
    let mut extra = serde_json::to_value(&snapshot).unwrap();
    extra["future_field"] = serde_json::json!(true);
    assert!(refresh_with(&serde_json::to_string_pretty(&extra).unwrap()).is_err());
    // Carried errors are never registrable.
    let mut errors = serde_json::to_value(&snapshot).unwrap();
    errors["errors"] = serde_json::json!(["window 4 failed"]);
    assert!(refresh_with(&serde_json::to_string_pretty(&errors).unwrap()).is_err());
}
