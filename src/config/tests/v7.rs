use super::*;
use crate::config::{GlsNoiseMode, LockinEdgePolicy, LockinEstimator, LockinWindowKind};

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
fn v7_rejects_the_removed_filter_table() {
    let text = V7_BASE.replace(
        "[lockin.window]",
        "filter = { kind = \"boxcar_legacy\", half_window_cycles = 1.0 }\n[lockin.window]",
    );
    let diagnostics = diagnostic_paths(&text);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.contains("filter")),
        "unexpected diagnostics: {diagnostics:?}"
    );
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
