use super::*;
use crate::config::{GlsCovarianceOutput, GlsNoiseMode, JointHarmonicGlsConfig};
use crate::config::{LockinEstimator, LockinWindow};

fn write_request(dir: &Path, name: &str, text: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, text).unwrap();
    path
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmoke_compare_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn request_validation_rejects_bad_inputs() {
    let dir = temp_dir("request");
    // Missing file.
    assert!(load_compare_request(&dir.join("nope.toml")).is_err());
    // Unknown field.
    let path = write_request(
        &dir,
        "unknown.toml",
        "schema_version = 1\nbase_config = \"b.toml\"\noutput = \"out\"\ngate_version = \"g\"\n[[methods]]\nname = \"a\"\nestimator = \"boxcar_legacy\"\nfuture = true\n",
    );
    assert!(load_compare_request(&path).is_err());
    // Bad schema version.
    let path = write_request(
        &dir,
        "schema.toml",
        "schema_version = 2\nbase_config = \"b.toml\"\noutput = \"out\"\ngate_version = \"g\"\n[[methods]]\nname = \"a\"\nestimator = \"boxcar_legacy\"\n",
    );
    assert!(load_compare_request(&path).is_err());
    // Empty methods.
    let path = write_request(
        &dir,
        "empty.toml",
        "schema_version = 1\nbase_config = \"b.toml\"\noutput = \"out\"\ngate_version = \"g\"\nmethods = []\n",
    );
    assert!(load_compare_request(&path).is_err());
    // Duplicate names.
    let path = write_request(
        &dir,
        "dup.toml",
        "schema_version = 1\nbase_config = \"b.toml\"\noutput = \"out\"\ngate_version = \"g\"\n[[methods]]\nname = \"a\"\nestimator = \"boxcar_legacy\"\n[[methods]]\nname = \"a\"\nestimator = \"boxcar_legacy\"\n",
    );
    assert!(load_compare_request(&path).is_err());
    // Unsafe labels (traversal / separators).
    for bad in ["../evil", "a/b", "a b", ""] {
        let path = write_request(
            &dir,
            "label.toml",
            &format!(
                "schema_version = 1\nbase_config = \"b.toml\"\noutput = \"out\"\ngate_version = \"g\"\n[[methods]]\nname = \"{bad}\"\nestimator = \"boxcar_legacy\"\n"
            ),
        );
        assert!(load_compare_request(&path).is_err(), "{bad}");
    }
    // Traversal in paths.
    let path = write_request(
        &dir,
        "traverse.toml",
        "schema_version = 1\nbase_config = \"../b.toml\"\noutput = \"out\"\ngate_version = \"g\"\n[[methods]]\nname = \"a\"\nestimator = \"boxcar_legacy\"\n",
    );
    let (request, request_dir) = load_compare_request(&path).unwrap();
    assert!(resolve_request_path(&request_dir, &request.base_config).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn request_labels_cannot_alias_internal_or_other_method_directories() {
    let dir = temp_dir("reserved_names");
    for names in [
        vec!["_frozen_source"],
        vec!["_FROZEN_SOURCE"],
        vec!["baseline", "BASELINE"],
    ] {
        let mut text = String::from(
            "schema_version = 1\nbase_config = \"b.toml\"\noutput = \"out\"\ngate_version = \"g\"\n",
        );
        for name in names {
            text.push_str(&format!(
                "[[methods]]\nname = \"{name}\"\nestimator = \"boxcar_legacy\"\n"
            ));
        }
        let path = write_request(&dir, "request.toml", &text);
        assert!(
            load_compare_request(&path).is_err(),
            "accepted conflicting labels: {text}"
        );
    }
    // A distinct underscore-prefixed user label remains valid.
    let path = write_request(
        &dir,
        "valid.toml",
        "schema_version = 1\nbase_config = \"b.toml\"\noutput = \"out\"\ngate_version = \"g\"\n[[methods]]\nname = \"_candidate\"\nestimator = \"boxcar_legacy\"\n",
    );
    assert!(load_compare_request(&path).is_ok());
    assert!(!dir.join("out").exists());
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn destination_safety_blocks_overwrite_and_nesting() {
    let dir = temp_dir("dest");
    let base = dir.join("base");
    std::fs::create_dir_all(&base).unwrap();
    // Existing output.
    let existing = dir.join("out");
    std::fs::create_dir_all(&existing).unwrap();
    assert!(ensure_new_destination(&existing, &base).is_err());
    // Missing parent.
    assert!(ensure_new_destination(&dir.join("ghost/out"), &base).is_err());
    // Output nested inside the base root.
    assert!(ensure_new_destination(&base.join("out"), &base).is_err());
    // Base root nested inside the output parent is fine only when disjoint;
    // output equal to the base root is rejected.
    assert!(ensure_new_destination(&base, &base).is_err());
    // Disjoint fresh destination passes.
    let fresh = dir.join("fresh");
    assert!(ensure_new_destination(&fresh, &base).is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn missing_base_config_fails_before_any_write() {
    let dir = temp_dir("missing");
    let request = write_request(
        &dir,
        "request.toml",
        "schema_version = 1\nbase_config = \"ghost.toml\"\noutput = \"out\"\ngate_version = \"g\"\n[[methods]]\nname = \"a\"\nestimator = \"boxcar_legacy\"\n",
    );
    assert!(run_compare(&request, None).is_err());
    // Nothing was created beside the request.
    let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().collect();
    assert_eq!(entries.len(), 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

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
  "correlation": null,
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
    "modes": ["identity", "phase_diagonal"],
    "geometry_restrictions": []
  }
}"#
    .to_string()
}

#[test]
fn legs_share_grid_and_never_touch_the_source() {
    use crate::test_support::test_config;
    use crate::utils::waveform::WaveformData;
    use std::f64::consts::PI;

    let dir = temp_dir("legs");
    let json = artifact_json();
    std::fs::write(dir.join("ch3.json"), &json).unwrap();
    let digest = crate::utils::checksum::sha256_hex(json.as_bytes());

    let base_root = dir.join("base");
    std::fs::create_dir_all(&base_root).unwrap();
    // Frozen source input the legs copy: existence-checked by LI
    // validation, linked (not parsed) by manifest sources.
    std::fs::create_dir_all(base_root.join("acquisition/waveforms")).unwrap();
    std::fs::write(
        base_root.join("acquisition/waveforms/waveform.csv"),
        "time (s),ch1,ch2,ch3\n0,0,0,0\n",
    )
    .unwrap();
    std::fs::write(
        base_root.join("acquisition/manifest.toml"),
        "schema_version = 1\n",
    )
    .unwrap();
    let mut base_cfg = test_config(vec![1], vec![3]);
    base_cfg.roles.reference_ch = 2;
    // LI validation requires an oscilloscope block; legs never touch
    // hardware (data is injected, models are files).
    base_cfg.instruments = Some(crate::config::Instruments {
        function_generator: None,
        oscilloscope: crate::config::Oscilloscope {
            connection: crate::config::Connection::Tcpip {
                ip: "127.0.0.1".to_string(),
                port: 1,
            },
            model: "compare-test".to_string(),
        },
    });
    base_cfg.source_path = dir.join("config.toml");
    base_cfg.set_artifact_root(base_root.clone());
    base_cfg.lockin.workers = 1;
    base_cfg.lockin.stride_samples = 10;
    base_cfg.lockin.lpf_half_window_cycles = 1.0;
    base_cfg.lockin.window = LockinWindow::legacy_boxcar(1.0);
    base_cfg.lockin.estimator = LockinEstimator::JointHarmonicGls(JointHarmonicGlsConfig {
        fit_harmonics: (1..=12).collect(),
        output_harmonics: (1..=6).collect(),
        envelope_degree: 0,
        noise_mode: GlsNoiseMode::Identity,
        covariance_output: GlsCovarianceOutput::Diagonal,
        failure_policy: crate::config::GlsFailurePolicy::Error,
        calibration_source: crate::config::GlsCalibrationSource::Artifact,
        calibrations: vec![crate::config::EstimatorCalibration {
            channel: 3,
            path: "ch3.json".to_string(),
            sha256: digest,
        }],
    });
    // Channels from test_config carry display factors; the LI estimator
    // inputs are never rescaled, so clear scaling on the signal channel
    // for a raw-volts comparison (sensor channels keep theirs).
    for channel in &mut base_cfg.channels {
        if channel.index == 3 {
            channel.factor = None;
            channel.scale_to_abs_max = None;
        }
    }

    let dt = 1.0e-5;
    let samples = 3_000usize;
    let time: Vec<f64> = (0..samples).map(|index| index as f64 * dt).collect();
    let sensor: Vec<f64> = time.iter().map(|t| 0.001 * t).collect();
    let reference: Vec<f64> = time.iter().map(|t| (2.0 * PI * 1000.0 * t).sin()).collect();
    let signal: Vec<f64> = time
        .iter()
        .map(|t| (2.0 * PI * 1000.0 * t + 0.3).sin())
        .collect();
    let data = WaveformData {
        t: time.into(),
        channels: vec![sensor, reference, signal],
    };

    let methods = vec![
        CompareMethod {
            name: "boxcar".to_string(),
            estimator: CompareEstimator::BoxcarLegacy,
        },
        CompareMethod {
            name: "joint_identity".to_string(),
            estimator: CompareEstimator::Joint {
                noise_mode: GlsNoiseMode::Identity,
                covariance_output: GlsCovarianceOutput::Diagonal,
            },
        },
    ];
    let output = dir.join("comparison");
    std::fs::create_dir(&output).unwrap();
    // Test data is already frozen: legs seed from the base acquisition tree.
    let frozen = base_cfg.paths().acquisition_dir();
    let legs = run_legs(&base_cfg, &data, &methods, &output, &frozen).unwrap();
    assert_eq!(legs.len(), 2);
    for leg in &legs {
        assert_eq!(leg.status, LegStatus::Complete, "{leg:?}");
    }
    assert_eq!(legs[0].grid_fingerprint, legs[1].grid_fingerprint);
    assert_eq!(
        legs[0].reference_frequency_hz,
        legs[1].reference_frequency_hz
    );
    // Candidate artifacts exist per leg, including GLS-only diagnostics.
    assert!(output.join("boxcar/analysis/lockin/ch3_xy.csv").is_file());
    assert!(
        output
            .join("joint_identity/analysis/lockin/ch3_xy.csv")
            .is_file()
    );
    assert!(
        output
            .join("joint_identity/analysis/lockin/ch3_quality.csv")
            .is_file()
    );
    assert!(
        !output
            .join("boxcar/analysis/lockin/ch3_quality.csv")
            .exists()
    );
    assert!(
        output
            .join("joint_identity/analysis/lockin/ch3_estimator.json")
            .is_file()
    );
    // Leg manifests register estimator-specific kinds truthfully: the
    // joint leg publishes quality/covariance/snapshot artifacts while the
    // boxcar leg publishes none of them.
    for (leg, gls) in [("boxcar", false), ("joint_identity", true)] {
        let manifest =
            std::fs::read_to_string(output.join(format!("{leg}/analysis/manifest.toml"))).unwrap();
        let value: toml::Value = toml::from_str(&manifest).unwrap();
        let kinds: Vec<&str> = value["artifacts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|artifact| artifact["kind"].as_str())
            .collect();
        assert!(kinds.contains(&"lockin_xy"), "{leg}: {kinds:?}");
        assert_eq!(kinds.contains(&"lockin_quality"), gls, "{leg}");
        assert_eq!(kinds.contains(&"lockin_covariance"), gls, "{leg}");
        assert_eq!(kinds.contains(&"lockin_estimator"), gls, "{leg}");
    }
    // The source (base artifact root) gained no analysis output.
    assert!(!base_root.join("analysis").exists());
    std::fs::remove_dir_all(&dir).unwrap();
}

fn complete_leg(name: &str, grid: &str, f_ref: f64) -> LegReport {
    LegReport {
        name: name.to_string(),
        estimator: "boxcar_legacy".to_string(),
        status: LegStatus::Complete,
        code: None,
        message: None,
        grid_fingerprint: Some(grid.to_string()),
        reference_frequency_hz: Some(f_ref),
        artifact_dir: format!("out/{name}"),
    }
}

#[test]
fn summarize_legs_computes_truthful_status() {
    let legs = vec![
        complete_leg("a", "grid", 1000.0),
        complete_leg("b", "grid", 1000.0),
    ];
    assert_eq!(summarize_legs(&legs), (true, true, true));
    // A failed leg keeps its evidence but fails overall completion.
    let mut legs = vec![
        complete_leg("a", "grid", 1000.0),
        LegReport {
            status: LegStatus::Failed,
            code: Some("model_hash_mismatch".to_string()),
            message: Some("digest mismatch".to_string()),
            grid_fingerprint: None,
            reference_frequency_hz: None,
            ..complete_leg("b", "grid", 1000.0)
        },
    ];
    assert_eq!(summarize_legs(&legs), (true, true, false));
    // Divergent grids or references fail even when every leg completes.
    legs[1].status = LegStatus::Complete;
    legs[1].grid_fingerprint = Some("other".to_string());
    assert_eq!(summarize_legs(&legs), (false, true, false));
    legs[1].grid_fingerprint = Some("grid".to_string());
    legs[1].reference_frequency_hz = Some(1001.0);
    assert_eq!(summarize_legs(&legs), (true, false, false));
    // No evidence at all is not agreement.
    assert_eq!(summarize_legs(&[]), (false, true, false));
}

#[test]
fn unknown_estimator_modes_fail_visibly() {
    let dir = temp_dir("estimator");
    let path = write_request(
        &dir,
        "request.toml",
        "schema_version = 1\nbase_config = \"b.toml\"\noutput = \"out\"\ngate_version = \"g\"\n[[methods]]\nname = \"a\"\nestimator = \"fourier\"\n",
    );
    assert!(load_compare_request(&path).is_err());
    let path = write_request(
        &dir,
        "request2.toml",
        "schema_version = 1\nbase_config = \"b.toml\"\noutput = \"out\"\ngate_version = \"g\"\n[[methods]]\nname = \"a\"\n[methods.estimator.joint]\nnoise_mode = \"identity\"\n",
    );
    assert!(load_compare_request(&path).is_ok());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn joint_leg_without_bindings_fails_without_fallback() {
    use crate::test_support::test_config;
    use crate::utils::waveform::WaveformData;
    use std::f64::consts::PI;

    // Boxcar base config: no calibration bindings anywhere.
    let dir = temp_dir("nobindings");
    let base_root = dir.join("base");
    std::fs::create_dir_all(&base_root).unwrap();
    std::fs::create_dir_all(base_root.join("acquisition/waveforms")).unwrap();
    std::fs::write(
        base_root.join("acquisition/waveforms/waveform.csv"),
        "time (s),ch1,ch2,ch3\n0,0,0,0\n",
    )
    .unwrap();
    let mut base_cfg = test_config(vec![1], vec![3]);
    base_cfg.roles.reference_ch = 2;
    base_cfg.instruments = Some(crate::config::Instruments {
        function_generator: None,
        oscilloscope: crate::config::Oscilloscope {
            connection: crate::config::Connection::Tcpip {
                ip: "127.0.0.1".to_string(),
                port: 1,
            },
            model: "compare-test".to_string(),
        },
    });
    base_cfg.source_path = dir.join("config.toml");
    base_cfg.set_artifact_root(base_root);
    base_cfg.lockin.workers = 1;
    base_cfg.lockin.stride_samples = 10;
    base_cfg.lockin.lpf_half_window_cycles = 1.0;
    base_cfg.lockin.window = LockinWindow::legacy_boxcar(1.0);
    base_cfg.lockin.estimator = LockinEstimator::BoxcarLegacy;
    for channel in &mut base_cfg.channels {
        if channel.index == 3 {
            channel.factor = None;
            channel.scale_to_abs_max = None;
        }
    }
    let dt = 1.0e-5;
    let samples = 3_000usize;
    let time: Vec<f64> = (0..samples).map(|index| index as f64 * dt).collect();
    let sensor: Vec<f64> = time.iter().map(|t| 0.001 * t).collect();
    let reference: Vec<f64> = time.iter().map(|t| (2.0 * PI * 1000.0 * t).sin()).collect();
    let signal: Vec<f64> = time
        .iter()
        .map(|t| (2.0 * PI * 1000.0 * t + 0.3).sin())
        .collect();
    let data = WaveformData {
        t: time.into(),
        channels: vec![sensor, reference, signal],
    };
    let methods = vec![
        CompareMethod {
            name: "boxcar".to_string(),
            estimator: CompareEstimator::BoxcarLegacy,
        },
        CompareMethod {
            name: "joint_identity".to_string(),
            estimator: CompareEstimator::Joint {
                noise_mode: GlsNoiseMode::Identity,
                covariance_output: GlsCovarianceOutput::Diagonal,
            },
        },
    ];
    let output = dir.join("comparison");
    std::fs::create_dir(&output).unwrap();
    // Test data is already frozen: legs seed from the base acquisition tree.
    let frozen = base_cfg.paths().acquisition_dir();
    let legs = run_legs(&base_cfg, &data, &methods, &output, &frozen).unwrap();
    assert_eq!(legs[0].status, LegStatus::Complete);
    // The joint leg fails explicitly on missing bindings; nothing falls
    // back to boxcar (no quality artifact appears under its directory).
    assert_eq!(legs[1].status, LegStatus::Failed);
    let message = legs[1].message.clone().unwrap();
    assert!(message.contains("calibration bindings"), "{message}");
    assert!(
        !output
            .join("joint_identity/analysis/lockin/ch3_quality.csv")
            .exists()
    );
    assert!(
        !output
            .join("joint_identity/analysis/lockin/ch3_xy.csv")
            .exists()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn method_preserves_base_signal_model() {
    use crate::test_support::test_config;
    // A six-harmonic base model stays six harmonics after applying a
    // joint method: only the requested estimator/noise fields change.
    let mut cfg = test_config(vec![1], vec![3]);
    cfg.lockin.estimator = LockinEstimator::JointHarmonicGls(JointHarmonicGlsConfig {
        fit_harmonics: (1..=6).collect(),
        output_harmonics: (1..=6).collect(),
        envelope_degree: 0,
        noise_mode: GlsNoiseMode::PhaseDiagonal,
        covariance_output: GlsCovarianceOutput::Full,
        failure_policy: crate::config::GlsFailurePolicy::Error,
        calibration_source: crate::config::GlsCalibrationSource::Artifact,
        calibrations: vec![crate::config::EstimatorCalibration {
            channel: 3,
            path: "ch3.json".to_string(),
            sha256: "0".repeat(64),
        }],
    });
    let method = CompareMethod {
        name: "identity".to_string(),
        estimator: CompareEstimator::Joint {
            noise_mode: GlsNoiseMode::Identity,
            covariance_output: GlsCovarianceOutput::Diagonal,
        },
    };
    apply_method_estimator(&mut cfg, &method).unwrap();
    match &cfg.lockin.estimator {
        LockinEstimator::JointHarmonicGls(gls) => {
            assert_eq!(gls.fit_harmonics, (1..=6).collect::<Vec<_>>());
            assert_eq!(gls.output_harmonics, (1..=6).collect::<Vec<_>>());
            assert_eq!(gls.envelope_degree, 0);
            assert_eq!(gls.noise_mode, GlsNoiseMode::Identity);
            assert_eq!(gls.covariance_output, GlsCovarianceOutput::Diagonal);
            assert_eq!(gls.failure_policy, crate::config::GlsFailurePolicy::Error);
            assert_eq!(gls.calibrations.len(), 1);
        }
        LockinEstimator::BoxcarLegacy => panic!("method must stay joint"),
    }
}

#[test]
fn source_fingerprint_covers_waveform_bytes() {
    use crate::test_support::test_config;
    // The review repro: mutate the waveform CSV with an unchanged
    // manifest; the fingerprint must change.
    let dir = temp_dir("fingerprint");
    let acquisition = dir.join("acquisition");
    std::fs::create_dir_all(acquisition.join("waveforms")).unwrap();
    std::fs::write(acquisition.join("manifest.toml"), "schema_version = 1\n").unwrap();
    std::fs::write(
        acquisition.join("waveforms/waveform.csv"),
        "time (s),ch3\n0,0\n",
    )
    .unwrap();
    let mut cfg = test_config(vec![1], vec![3]);
    cfg.source_path = dir.join("config.toml");
    cfg.set_artifact_root(dir.clone());
    let before = hash_source_entries(&source_entries(&cfg).unwrap()).unwrap();
    // Waveform-only mutation changes the print.
    std::fs::write(
        acquisition.join("waveforms/waveform.csv"),
        "time (s),ch3\n0,1\n",
    )
    .unwrap();
    let mutated = hash_source_entries(&source_entries(&cfg).unwrap()).unwrap();
    assert_ne!(before, mutated);
    // Manifest-only mutation also changes it.
    std::fs::write(acquisition.join("manifest.toml"), "schema_version = 2\n").unwrap();
    let mutated_manifest = hash_source_entries(&source_entries(&cfg).unwrap()).unwrap();
    assert_ne!(mutated, mutated_manifest);
    // Untouched trees verify; mutated trees fail with source_changed.
    assert!(verify_source_tree(&acquisition, &mutated_manifest).is_ok());
    std::fs::write(
        acquisition.join("waveforms/waveform.csv"),
        "time (s),ch3\n0,2\n",
    )
    .unwrap();
    let error = verify_source_tree(&acquisition, &mutated_manifest)
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("source_changed"));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn frozen_copy_reads_identical_waveforms() {
    use crate::test_support::test_config;
    // Parsing from a frozen staging copy yields the same arrays as the
    // live read: legs and estimation share one frozen byte source.
    let dir = temp_dir("frozencopy");
    let live = dir.join("live/acquisition");
    std::fs::create_dir_all(live.join("waveforms")).unwrap();
    std::fs::write(live.join("manifest.toml"), "schema_version = 1\n").unwrap();
    let csv = "time (s),ch1,ch2,ch3\n0,0.1,0.2,0.3\n1e-05,0.4,0.5,0.6\n";
    std::fs::write(live.join("waveforms/waveform.csv"), csv).unwrap();
    let mut live_cfg = test_config(vec![1], vec![3]);
    live_cfg.roles.reference_ch = 2;
    live_cfg.source_path = dir.join("config.toml");
    live_cfg.set_artifact_root(dir.join("live"));
    let staging = dir.join("staging/acquisition");
    std::fs::create_dir_all(&staging).unwrap();
    let entries = source_entries(&live_cfg).unwrap();
    assert!(!entries.is_empty());
    freeze_source_entries(&entries, &staging).unwrap();
    assert_eq!(
        hash_source_entries(&entries).unwrap(),
        fingerprint_tree(&staging).unwrap()
    );
    let mut frozen_cfg = live_cfg.clone();
    frozen_cfg.set_artifact_root(dir.join("staging"));
    let live_data = crate::utils::waveform::read_all_fetched_waveforms(&live_cfg).unwrap();
    let frozen_data = crate::utils::waveform::read_all_fetched_waveforms(&frozen_cfg).unwrap();
    assert_eq!(frozen_data.channels, live_data.channels);
    // The config digest covers the parsed bytes, never a re-read.
    let mut cfg = test_config(vec![1], vec![3]);
    cfg.source_text = Some("version = 3\n".to_string());
    assert_eq!(
        config_source_digest(&cfg).unwrap(),
        crate::utils::checksum::sha256_hex(b"version = 3\n")
    );
    cfg.source_text = None;
    assert!(config_source_digest(&cfg).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
