use super::*;
use crate::config::{
    GlsCovarianceOutput, GlsFailurePolicy, GlsNoiseMode, JointHarmonicGlsConfig, LockinEdgePolicy,
    LockinEstimator, LockinWindowKind,
};
use crate::test_support::test_config;

const V7_BASE: &str = r#"version = 7
[scope]
model = "DHO5108"
connection = "tcp://192.0.2.10:55255"
[data]
output = "raw"
input = "raw"
[[sensors]]
channel = 1
scale = { factor = 1.0 }
label = "field"
unit = "T"
[pulse]
background_before = { start = -0.005, end = -0.001 }
background_after = { start = 0.01, end = 0.02 }
[reference]
channel = 2
fft_window = { start = 0.0, end = 0.005 }
stride_samples = 100
window_samples = 1000
[lockin]
channels = [3]
workers = 2
stride_samples = 100
[lockin.window]
kind = "reference_cycles"
half_window_cycles = 1.0
edge_policy = "legacy_trim"
[lockin.estimator]
kind = "boxcar_legacy"
[phase]
offsets = [0, 0, 0, 0, 0, 0]
[moke]
sensor = 1
method = "harmonics"
factor = -1.0
"#;

const SHA_A: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn v7_gls_config() -> String {
    V7_BASE.replace(
        "[lockin.estimator]\nkind = \"boxcar_legacy\"",
        &format!(
            "[lockin.estimator]\nkind = \"joint_harmonic_gls\"\nfit_harmonics = [1, 2, 3, 4, 5, 6]\noutput_harmonics = [1, 2, 3, 4, 5, 6]\nnoise_mode = \"identity\"\n[[lockin.estimator.calibrations]]\nchannel = 3\npath = \"calibration/ch3.json\"\nsha256 = \"{SHA_A}\""
        ),
    )
}

fn ready_config(text: &str) -> crate::config::Config {
    match load_from_str(text) {
        ConfigLoad::Ready { config, .. } => config,
        ConfigLoad::Diagnostics(diagnostics) => panic!(
            "expected ready load, got {:?}",
            diagnostics
                .diagnostics
                .iter()
                .map(|diagnostic| format!(
                    "{}: {}",
                    diagnostic.path.as_deref().unwrap_or("-"),
                    diagnostic.message
                ))
                .collect::<Vec<_>>()
        ),
    }
}

fn diagnostic_paths(text: &str) -> Vec<String> {
    match load_from_str(text) {
        ConfigLoad::Ready { .. } => panic!("expected diagnostics, got ready load"),
        ConfigLoad::Diagnostics(diagnostics) => diagnostics
            .diagnostics
            .iter()
            .map(|diagnostic| {
                format!(
                    "{}: {}",
                    diagnostic.path.as_deref().unwrap_or("-"),
                    diagnostic.message
                )
            })
            .collect(),
    }
}

#[test]
fn v7_legacy_loads_window_and_estimator_contracts() {
    let config = ready_config(V7_BASE);
    assert_eq!(config.version, 7);
    assert_eq!(config.lockin.window.kind, LockinWindowKind::ReferenceCycles);
    assert_eq!(config.lockin.window.half_window_cycles, 1.0);
    assert_eq!(
        config.lockin.window.edge_policy,
        LockinEdgePolicy::LegacyTrim
    );
    assert!(matches!(
        config.lockin.estimator,
        LockinEstimator::BoxcarLegacy
    ));
    assert_eq!(config.lockin.estimator_name(), "boxcar_legacy");
    assert_eq!(
        config.lockin.lpf_half_window_cycles,
        config.lockin.window.half_window_cycles
    );
}

#[test]
fn v7_gls_loads_full_settings() {
    let config = ready_config(&v7_gls_config());
    match &config.lockin.estimator {
        LockinEstimator::JointHarmonicGls(gls) => {
            assert_eq!(gls.fit_harmonics, vec![1, 2, 3, 4, 5, 6]);
            assert_eq!(gls.output_harmonics, vec![1, 2, 3, 4, 5, 6]);
            assert_eq!(gls.envelope_degree, 0);
            assert!(matches!(gls.noise_mode, GlsNoiseMode::Identity));
            assert_eq!(gls.calibrations.len(), 1);
            assert_eq!(gls.calibrations[0].channel, 3);
            assert_eq!(gls.calibrations[0].sha256, SHA_A);
        }
        LockinEstimator::BoxcarLegacy => panic!("expected GLS estimator"),
    }
    assert_eq!(config.lockin.estimator_name(), "joint_harmonic_gls");
}

#[test]
fn v7_filter_table_reports_a_migration_diagnostic() {
    let text = V7_BASE.replace(
        "[lockin.window]",
        "filter = { kind = \"boxcar_legacy\", half_window_cycles = 1.0 }\n[lockin.window]",
    );
    match load_from_str(&text) {
        ConfigLoad::Ready { .. } => panic!("expected diagnostics"),
        ConfigLoad::Diagnostics(diagnostics) => {
            assert_eq!(diagnostics.diagnostics.len(), 1);
            let diagnostic = &diagnostics.diagnostics[0];
            assert!(matches!(diagnostic.kind, DiagnosticKind::Migration));
            assert!(diagnostic.message.contains("lockin.window"));
            assert!(
                diagnostic
                    .suggestion
                    .as_deref()
                    .unwrap_or_default()
                    .contains("migrate --to 7")
            );
        }
    }
}

#[test]
fn v6_window_section_reports_a_migration_diagnostic() {
    let text = V7_BASE
        .replace("version = 7", "version = 6")
        .replace("[lockin.window]\nkind = \"reference_cycles\"\nhalf_window_cycles = 1.0\nedge_policy = \"legacy_trim\"\n[lockin.estimator]\nkind = \"boxcar_legacy\"\n", "filter = { kind = \"boxcar_legacy\", half_window_cycles = 1.0 }\n[lockin.window]\nkind = \"reference_cycles\"\n");
    match load_from_str(&text) {
        ConfigLoad::Ready { .. } => panic!("expected diagnostics"),
        ConfigLoad::Diagnostics(diagnostics) => {
            let diagnostic = &diagnostics.diagnostics[0];
            assert!(matches!(diagnostic.kind, DiagnosticKind::Migration));
            assert!(diagnostic.message.contains("v7"));
        }
    }
}

#[test]
fn native_and_browser_core_agree_on_v7_contracts() {
    let valid = [V7_BASE.to_string(), v7_gls_config()];
    for text in &valid {
        let native_ready = matches!(load_from_str(text), ConfigLoad::Ready { .. });
        let core_valid = crate::config::validate_config_toml_core(text).valid;
        assert!(native_ready, "native rejected a valid config");
        assert!(core_valid, "core rejected a valid config");
    }
    let mutants: &[(&str, &str)] = &[
        (
            "[lockin.window]",
            "filter = { kind = \"boxcar_legacy\", half_window_cycles = 1.0 }\n[lockin.window]",
        ),
        (
            "kind = \"boxcar_legacy\"",
            "kind = \"boxcar_legacy\"\nfit_harmonics = [1]",
        ),
        (
            "output_harmonics = [1, 2, 3, 4, 5, 6]",
            "output_harmonics = [1, 2]",
        ),
        ("half_window_cycles = 1.0", "half_window_cycles = 0.0"),
        ("half_window_cycles = 1.0", "half_window_cycles = inf"),
        ("noise_mode = \"identity\"", "noise_mode = \"quantum\""),
        ("envelope_degree = 0", "envelope_degree = 2"),
        ("version = 7", "version = 6"),
    ];
    for (from, to) in mutants {
        for base in [V7_BASE.to_string(), v7_gls_config()] {
            if !base.contains(from) {
                continue;
            }
            let text = base.replacen(from, to, 1);
            let native_ready = matches!(load_from_str(&text), ConfigLoad::Ready { .. });
            let core_valid = crate::config::validate_config_toml_core(&text).valid;
            assert!(!native_ready, "native accepted mutant {from:?} -> {to:?}");
            assert_eq!(
                native_ready, core_valid,
                "native/core disagreement for mutant {from:?} -> {to:?}"
            );
        }
    }
}

#[test]
fn v6_rejects_window_and_estimator_keys() {
    let text = V7_BASE
        .replace("version = 7", "version = 6")
        .replace("[lockin.window]\nkind = \"reference_cycles\"\nhalf_window_cycles = 1.0\nedge_policy = \"legacy_trim\"\n[lockin.estimator]\nkind = \"boxcar_legacy\"\n", "filter = { kind = \"boxcar_legacy\", half_window_cycles = 1.0 }\n[lockin.window]\nkind = \"reference_cycles\"\n");
    let diagnostics = diagnostic_paths(&text);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("window")),
        "unexpected diagnostics: {diagnostics:?}"
    );
}

#[test]
fn v7_gls_validation_rejects_bad_contracts() {
    let cases = [
        (
            "output_harmonics = [1, 2, 3, 4, 5, 6]",
            "output_harmonics = [1, 2, 3]",
            "lockin.estimator.output_harmonics",
        ),
        (
            "noise_mode = \"identity\"",
            "noise_mode = \"identity\"\nenvelope_degree = 1",
            "lockin.estimator.envelope_degree",
        ),
        (
            "fit_harmonics = [1, 2, 3, 4, 5, 6]",
            "fit_harmonics = [1, 3, 2, 4, 5, 6]",
            "lockin.estimator.fit_harmonics",
        ),
        (
            "fit_harmonics = [1, 2, 3, 4, 5, 6]",
            "fit_harmonics = [1, 2, 3, 4, 5]",
            "lockin.estimator.fit_harmonics",
        ),
        (
            &format!("sha256 = \"{SHA_A}\""),
            "sha256 = \"${CALIBRATION_SHA256}\"",
            "lockin.estimator.calibrations[0].sha256",
        ),
    ];
    for (from, to, path) in cases {
        let diagnostics = diagnostic_paths(&v7_gls_config().replace(from, to));
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.starts_with(path)),
            "expected {path}, got {diagnostics:?}"
        );
    }
}

#[test]
fn v7_gls_rejects_missing_extra_and_duplicate_bindings() {
    let missing = v7_gls_config().replace(
        &format!(
            "[[lockin.estimator.calibrations]]\nchannel = 3\npath = \"calibration/ch3.json\"\nsha256 = \"{SHA_A}\"\n"
        ),
        "",
    );
    let diagnostics = diagnostic_paths(&missing);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.starts_with("lockin.estimator.calibrations:")),
        "unexpected diagnostics: {diagnostics:?}"
    );

    let extra = v7_gls_config().replace("channel = 3", "channel = 9");
    let diagnostics = diagnostic_paths(&extra);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.starts_with("lockin.estimator.calibrations[0].channel")),
        "unexpected diagnostics: {diagnostics:?}"
    );

    let duplicate = v7_gls_config().replace(
        &format!("sha256 = \"{SHA_A}\""),
        &format!(
            "sha256 = \"{SHA_A}\"\n[[lockin.estimator.calibrations]]\nchannel = 3\npath = \"calibration/ch3b.json\"\nsha256 = \"{SHA_A}\""
        ),
    );
    let diagnostics = diagnostic_paths(&duplicate);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("duplicate")),
        "unexpected diagnostics: {diagnostics:?}"
    );
}

fn v7_gls_prepulse_config() -> String {
    V7_BASE.replace(
        "[lockin.estimator]\nkind = \"boxcar_legacy\"",
        "[lockin.estimator]\nkind = \"joint_harmonic_gls\"\nfit_harmonics = [1, 2, 3, 4, 5, 6]\noutput_harmonics = [1, 2, 3, 4, 5, 6]\nnoise_mode = \"identity\"\ncalibration_source = \"prepulse\"",
    )
}

#[test]
fn v7_gls_calibration_source_defaults_to_artifact() {
    let config = ready_config(&v7_gls_config());
    match &config.lockin.estimator {
        LockinEstimator::JointHarmonicGls(gls) => {
            assert!(matches!(
                gls.calibration_source,
                crate::config::GlsCalibrationSource::Artifact
            ));
        }
        LockinEstimator::BoxcarLegacy => panic!("expected GLS estimator"),
    }
}

#[test]
fn v7_gls_prepulse_loads_without_bindings() {
    let config = ready_config(&v7_gls_prepulse_config());
    match &config.lockin.estimator {
        LockinEstimator::JointHarmonicGls(gls) => {
            assert!(matches!(
                gls.calibration_source,
                crate::config::GlsCalibrationSource::Prepulse
            ));
            assert!(gls.calibrations.is_empty());
        }
        LockinEstimator::BoxcarLegacy => panic!("expected GLS estimator"),
    }
}

#[test]
fn v7_gls_prepulse_rejects_file_bindings() {
    let text = v7_gls_prepulse_config().replace(
        "calibration_source = \"prepulse\"",
        &format!(
            "calibration_source = \"prepulse\"\n[[lockin.estimator.calibrations]]\nchannel = 3\npath = \"calibration/ch3.json\"\nsha256 = \"{SHA_A}\""
        ),
    );
    let diagnostics = diagnostic_paths(&text);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.starts_with("lockin.estimator.calibration_source")),
        "unexpected diagnostics: {diagnostics:?}"
    );
}

#[test]
fn v7_gls_rejects_unknown_calibration_source() {
    let text = v7_gls_config().replace(
        "noise_mode = \"identity\"",
        "noise_mode = \"identity\"\ncalibration_source = \"file\"",
    );
    let diagnostics = diagnostic_paths(&text);
    assert!(
        diagnostics.join("\n").contains("expected `artifact` or `prepulse`"),
        "unexpected diagnostics: {diagnostics:?}"
    );
}

#[test]
fn v7_legacy_rejects_gls_only_settings() {
    let text = V7_BASE.replace(
        "kind = \"boxcar_legacy\"",
        "kind = \"boxcar_legacy\"\nfit_harmonics = [1, 2]",
    );
    let diagnostics = diagnostic_paths(&text);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("fit_harmonics")),
        "unexpected diagnostics: {diagnostics:?}"
    );
}

#[test]
fn v7_normalized_render_round_trips() {
    for text in [V7_BASE.to_string(), v7_gls_config()] {
        let config = ready_config(&text);
        let normalized = render_normalized_config(&config).unwrap();
        assert!(normalized.contains("version = 7"));
        assert!(normalized.contains("[lockin.window]"));
        assert!(normalized.contains("[lockin.estimator]"));
        let reloaded = ready_config(&normalized);
        assert_eq!(reloaded.version, 7);
        assert_eq!(
            reloaded.lockin.estimator_name(),
            config.lockin.estimator_name()
        );
    }
}

fn gls_config(calibrations: Vec<crate::config::EstimatorCalibration>) -> JointHarmonicGlsConfig {
    JointHarmonicGlsConfig {
        fit_harmonics: vec![1, 2, 3, 4, 5, 6],
        output_harmonics: vec![1, 2, 3, 4, 5, 6],
        envelope_degree: 0,
        noise_mode: GlsNoiseMode::Identity,
        covariance_output: GlsCovarianceOutput::Diagonal,
        failure_policy: GlsFailurePolicy::Error,
        calibration_source: crate::config::GlsCalibrationSource::Artifact,
        calibrations,
    }
}

fn good_calibration() -> crate::config::EstimatorCalibration {
    crate::config::EstimatorCalibration {
        channel: 3,
        path: "calibration/ch3.json".to_string(),
        sha256: SHA_A.to_string(),
    }
}

fn estimator_error_paths(cfg: &mut crate::config::Config) -> Vec<String> {
    super::super::validation::validate_common(cfg)
        .errors
        .iter()
        .filter_map(|error| error.path.clone())
        .filter(|path| path.starts_with("lockin.estimator") || path.starts_with("lockin.window"))
        .collect()
}

#[test]
fn native_validation_accepts_a_good_gls_config_directly() {
    let mut cfg = test_config(vec![1], vec![3]);
    cfg.lockin.estimator = LockinEstimator::JointHarmonicGls(gls_config(vec![good_calibration()]));
    assert_eq!(estimator_error_paths(&mut cfg), Vec::<String>::new());
}

#[test]
fn native_validation_rejects_bad_gls_contracts_directly() {
    // output harmonics
    let mut cfg = test_config(vec![1], vec![3]);
    let mut gls = gls_config(vec![good_calibration()]);
    gls.output_harmonics = vec![1, 2, 3];
    cfg.lockin.estimator = LockinEstimator::JointHarmonicGls(gls);
    let paths = estimator_error_paths(&mut cfg);
    assert!(
        paths
            .iter()
            .any(|path| path == "lockin.estimator.output_harmonics"),
        "unexpected paths: {paths:?}"
    );

    // envelope degree
    let mut cfg = test_config(vec![1], vec![3]);
    let mut gls = gls_config(vec![good_calibration()]);
    gls.envelope_degree = 2;
    cfg.lockin.estimator = LockinEstimator::JointHarmonicGls(gls);
    let paths = estimator_error_paths(&mut cfg);
    assert!(
        paths
            .iter()
            .any(|path| path == "lockin.estimator.envelope_degree"),
        "unexpected paths: {paths:?}"
    );

    // template digest
    let mut cfg = test_config(vec![1], vec![3]);
    let mut bad = good_calibration();
    bad.sha256 = "${CALIBRATION_SHA256}".to_string();
    cfg.lockin.estimator = LockinEstimator::JointHarmonicGls(gls_config(vec![bad]));
    let paths = estimator_error_paths(&mut cfg);
    assert!(
        paths
            .iter()
            .any(|path| path == "lockin.estimator.calibrations[0].sha256"),
        "unexpected paths: {paths:?}"
    );

    // missing binding
    let mut cfg = test_config(vec![1], vec![3]);
    cfg.lockin.estimator = LockinEstimator::JointHarmonicGls(gls_config(Vec::new()));
    let paths = estimator_error_paths(&mut cfg);
    assert!(
        paths
            .iter()
            .any(|path| path == "lockin.estimator.calibrations"),
        "unexpected paths: {paths:?}"
    );

    // unsorted fit
    let mut cfg = test_config(vec![1], vec![3]);
    let mut gls = gls_config(vec![good_calibration()]);
    gls.fit_harmonics = vec![1, 3, 2, 4, 5, 6];
    cfg.lockin.estimator = LockinEstimator::JointHarmonicGls(gls);
    let paths = estimator_error_paths(&mut cfg);
    assert!(
        paths
            .iter()
            .any(|path| path.starts_with("lockin.estimator.fit_harmonics")),
        "unexpected paths: {paths:?}"
    );
}

#[test]
fn native_validation_flags_a_diverged_window_mirror_directly() {
    let mut cfg = test_config(vec![1], vec![3]);
    cfg.lockin.lpf_half_window_cycles = 2.0;
    let paths = estimator_error_paths(&mut cfg);
    assert!(
        paths
            .iter()
            .any(|path| path == "lockin.window.half_window_cycles"),
        "unexpected paths: {paths:?}"
    );
}

use crate::config::schema::{
    ConfigV7, EstimatorCalibrationV7, LockinEstimatorV7, LockinV7, LockinWindowV7,
};

fn lockin_v7_toml() -> String {
    V7_BASE
        .split("[lockin]\n")
        .nth(1)
        .unwrap()
        .split("[phase]")
        .next()
        .unwrap()
        .to_string()
}

#[test]
fn raw_v7_structs_reject_unknown_fields_directly() {
    // Full-document unknown top-level key.
    let document = format!("{V7_BASE}top_level_unknown = true\n");
    assert!(toml::from_str::<ConfigV7>(&document).is_err());

    // Lockin-level: the removed filter table is rejected.
    let lockin = lockin_v7_toml();
    assert!(
        toml::from_str::<LockinV7>(&lockin.replace(
            "[lockin.window]",
            "filter = { kind = \"boxcar_legacy\" }\n[lockin.window]"
        ))
        .is_err()
    );

    // Window-level unknown key.
    assert!(
        toml::from_str::<LockinWindowV7>(
            "kind = \"reference_cycles\"\nhalf_window_cycles = 1.0\nedge_policy = \"legacy_trim\"\ntricubic = true"
        )
        .is_err()
    );

    // Legacy estimator rejects GLS-only settings.
    assert!(
        toml::from_str::<LockinEstimatorV7>("kind = \"boxcar_legacy\"\nfit_harmonics = [1]")
            .is_err()
    );

    // GLS estimator rejects legacy filter keys (all required GLS fields
    // present so the unknown field is the only possible failure).
    assert!(
        toml::from_str::<LockinEstimatorV7>(
            "kind = \"joint_harmonic_gls\"\nfit_harmonics = [1, 2, 3, 4, 5, 6]\noutput_harmonics = [1, 2, 3, 4, 5, 6]\nnoise_mode = \"identity\"\nfilter = { kind = \"boxcar_legacy\" }"
        )
        .is_err()
    );

    // Calibration entries reject unknown keys.
    assert!(
        toml::from_str::<EstimatorCalibrationV7>(
            "channel = 3\npath = \"c.json\"\nsha256 = \"abc\"\nnote = \"hi\""
        )
        .is_err()
    );

    // Positive controls: the same fragments without unknown keys parse.
    assert!(toml::from_str::<ConfigV7>(V7_BASE).is_ok());
    assert!(
        toml::from_str::<LockinWindowV7>(
            "kind = \"reference_cycles\"\nhalf_window_cycles = 1.0\nedge_policy = \"legacy_trim\""
        )
        .is_ok()
    );
    assert!(toml::from_str::<LockinEstimatorV7>("kind = \"boxcar_legacy\"").is_ok());
}
