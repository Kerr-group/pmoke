//! Stage-minimal config tests (FR-01/FR-04, Issue #264, Card A).
//!
//! Absent unrelated v7 sections fill with inert defaults at parse; full
//! configs load byte-identically; missing required items yield named
//! diagnostics with how-to-add hints; the version/roles/channels core stays
//! required.

use super::*;
use crate::config::{
    Connection, FetchAnalysisInput, FetchOutput, LockinEdgePolicy, LockinEstimator,
    LockinWindowKind, MokeType, PlotDecimation,
};

/// Canonical full v7 config (mirrors the v7 fixture shape).
const FULL_V7: &str = r#"version = 7
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

/// Sensor-class skeleton: only the always-required core plus `[pulse]`; every
/// other section is an absent unrelated section filled with inert defaults.
const SENSOR_SKELETON: &str = r#"version = 7
[[sensors]]
channel = 1
scale = { factor = 1.0 }
label = "field"
unit = "T"
[pulse]
background_before = { start = -0.005, end = -0.001 }
background_after = { start = 0.01, end = 0.02 }
"#;

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

fn diagnostics_of(text: &str) -> Vec<crate::config::ConfigDiagnostic> {
    match load_from_str(text) {
        ConfigLoad::Ready { .. } => panic!("expected diagnostics, got ready load"),
        ConfigLoad::Diagnostics(diagnostics) => diagnostics.diagnostics,
    }
}

#[test]
fn minimal_sensor_skeleton_loads_ready_with_inert_defaults() {
    let config = ready_config(SENSOR_SKELETON);
    assert_eq!(config.version, 7);

    // Always-required core: roles/channels derived from the explicit sensor.
    assert_eq!(config.roles.sensor_ch, vec![1]);
    assert_eq!(config.roles.reference_ch, 0);
    assert!(config.roles.signal_ch.is_empty());
    assert_eq!(
        config
            .channels
            .iter()
            .map(|ch| ch.index)
            .collect::<Vec<_>>(),
        vec![0, 1]
    );

    // Defaulted scope: well-formed dummy, never real hardware.
    let oscilloscope = &config
        .instruments
        .as_ref()
        .expect("defaulted scope keeps instruments present")
        .oscilloscope;
    assert_eq!(oscilloscope.model, "DHO5108");
    assert!(matches!(
        &oscilloscope.connection,
        Connection::Tcpip { ip, port } if ip == "127.0.0.1" && *port == 9
    ));

    // Defaulted data mirrors the native fetch default.
    assert!(matches!(config.fetch.output, FetchOutput::Csv));
    assert!(matches!(
        config.fetch.analysis_input,
        FetchAnalysisInput::Csv
    ));
    assert!(!config.screenshot.enabled);

    // Defaulted reference is the unspecified sentinel.
    assert_eq!(config.reference.stride_samples, 100);
    assert_eq!(config.reference.window_samples, 1000);

    // Defaulted lockin is the legacy boxcar with an empty channel set.
    assert!(config.roles.signal_ch.is_empty());
    assert_eq!(config.lockin.workers, 1);
    assert_eq!(config.lockin.stride_samples, 100);
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

    // Defaulted phase/moke satisfy full validation without authorizing runs.
    assert_eq!(config.phase.m_omega_t0_offset, vec![0.0; 6]);
    assert_eq!(config.moke.use_sensor_ch, 1);
    assert!(matches!(config.moke.moke_type, MokeType::Standard));
    assert_eq!(config.moke.factor, 1.0);
}

#[test]
fn skeleton_without_pulse_loads_default_windows() {
    let text = SENSOR_SKELETON.replace(
        "[pulse]\nbackground_before = { start = -0.005, end = -0.001 }\nbackground_after = { start = 0.01, end = 0.02 }\n",
        "",
    );
    let config = ready_config(&text);
    assert_eq!(config.pulse.bg_window_before.start, -0.005);
    assert_eq!(config.pulse.bg_window_before.end, -0.001);
    assert_eq!(config.pulse.bg_window_after.start, 0.01);
    assert_eq!(config.pulse.bg_window_after.end, 0.02);
}

#[test]
fn each_absent_unrelated_section_defaults_individually() {
    let removals = [
        (
            "scope",
            "[scope]\nmodel = \"DHO5108\"\nconnection = \"tcp://192.0.2.10:55255\"\n",
        ),
        ("data", "[data]\noutput = \"raw\"\ninput = \"raw\"\n"),
        (
            "pulse",
            "[pulse]\nbackground_before = { start = -0.005, end = -0.001 }\nbackground_after = { start = 0.01, end = 0.02 }\n",
        ),
        (
            "reference",
            "[reference]\nchannel = 2\nfft_window = { start = 0.0, end = 0.005 }\nstride_samples = 100\nwindow_samples = 1000\n",
        ),
        (
            "lockin",
            "[lockin]\nchannels = [3]\nworkers = 2\nstride_samples = 100\n[lockin.window]\nkind = \"reference_cycles\"\nhalf_window_cycles = 1.0\nedge_policy = \"legacy_trim\"\n[lockin.estimator]\nkind = \"boxcar_legacy\"\n",
        ),
        ("phase", "[phase]\noffsets = [0, 0, 0, 0, 0, 0]\n"),
        (
            "moke",
            "[moke]\nsensor = 1\nmethod = \"harmonics\"\nfactor = -1.0\n",
        ),
    ];
    for (section, block) in removals {
        let text = FULL_V7.replacen(block, "", 1);
        assert_ne!(text, FULL_V7, "removal block for {section} did not match");
        match load_from_str(&text) {
            ConfigLoad::Ready { config, .. } => {
                assert_eq!(config.version, 7, "section {section}");
                // The surviving explicit sections keep their values.
                if section != "scope" {
                    let oscilloscope = &config.instruments.as_ref().unwrap().oscilloscope;
                    assert!(matches!(
                        &oscilloscope.connection,
                        Connection::Tcpip { ip, .. } if ip == "192.0.2.10"
                    ));
                }
                if section != "reference" {
                    assert_eq!(config.roles.reference_ch, 2, "section {section}");
                }
                if section != "lockin" {
                    assert_eq!(config.roles.signal_ch, vec![3], "section {section}");
                    assert_eq!(config.lockin.workers, 2, "section {section}");
                }
                if section != "moke" {
                    assert!(matches!(config.moke.moke_type, MokeType::Harmonics));
                    assert_eq!(config.moke.factor, -1.0);
                }
            }
            ConfigLoad::Diagnostics(diagnostics) => panic!(
                "section {section} absent should default, got {:?}",
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
}

#[test]
fn full_config_load_output_is_identical() {
    // Pins the no-regression contract: present sections never see defaults.
    let config = ready_config(FULL_V7);

    let oscilloscope = &config.instruments.as_ref().unwrap().oscilloscope;
    assert_eq!(oscilloscope.model, "DHO5108");
    assert!(matches!(
        &oscilloscope.connection,
        Connection::Tcpip { ip, port } if ip == "192.0.2.10" && *port == 55255
    ));
    assert!(matches!(config.fetch.output, FetchOutput::Raw));
    assert!(matches!(
        config.fetch.analysis_input,
        FetchAnalysisInput::Raw
    ));

    assert_eq!(config.roles.sensor_ch, vec![1]);
    assert_eq!(config.roles.reference_ch, 2);
    assert_eq!(config.roles.signal_ch, vec![3]);
    assert_eq!(
        config
            .channels
            .iter()
            .map(|ch| ch.index)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );

    assert_eq!(config.pulse.bg_window_before.start, -0.005);
    assert_eq!(config.pulse.bg_window_before.end, -0.001);
    assert_eq!(config.pulse.bg_window_after.start, 0.01);
    assert_eq!(config.pulse.bg_window_after.end, 0.02);

    assert_eq!(config.reference.stride_samples, 100);
    assert_eq!(config.reference.window_samples, 1000);

    assert_eq!(config.lockin.workers, 2);
    assert_eq!(config.roles.signal_ch, vec![3]);
    assert_eq!(config.lockin.stride_samples, 100);
    assert_eq!(config.lockin.window.half_window_cycles, 1.0);
    assert_eq!(config.lockin.lpf_half_window_cycles, 1.0);
    assert!(matches!(
        config.lockin.estimator,
        LockinEstimator::BoxcarLegacy
    ));
    assert!(!config.lockin.save_npy);

    assert_eq!(config.phase.m_omega_t0_offset, vec![0.0; 6]);

    assert_eq!(config.moke.use_sensor_ch, 1);
    assert!(matches!(config.moke.moke_type, MokeType::Harmonics));
    assert_eq!(config.moke.factor, -1.0);

    assert_eq!(config.plot.max_points, 100_000);
    assert_eq!(config.plot.output_dir, "plots");
    assert!(matches!(config.plot.decimation, PlotDecimation::Stride));
    assert!(!config.plot.fail_on_error);
}

#[test]
fn missing_version_names_the_item_and_how_to_add() {
    let diagnostics = diagnostics_of("[[sensors]]\nchannel = 1\n");
    let version = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.path.as_deref() == Some("version"))
        .expect("expected a version diagnostic");
    assert!(
        version.message.contains("missing required"),
        "unexpected message: {}",
        version.message
    );
    let suggestion = version.suggestion.as_deref().unwrap_or_default();
    assert!(
        suggestion.contains("version = 7"),
        "unexpected suggestion: {suggestion}"
    );
}

#[test]
fn missing_scope_model_names_the_item_and_how_to_add() {
    let text = FULL_V7.replacen("model = \"DHO5108\"\n", "", 1);
    let diagnostics = diagnostics_of(&text);
    let scoped = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.path.as_deref() == Some("scope.model"))
        .expect("expected a scope.model diagnostic");
    assert!(
        scoped
            .message
            .contains("missing required item `scope.model`"),
        "unexpected message: {}",
        scoped.message
    );
    let suggestion = scoped.suggestion.as_deref().unwrap_or_default();
    assert!(
        suggestion.contains("model = \"DHO5108\""),
        "unexpected suggestion: {suggestion}"
    );
}

#[test]
fn unknown_fields_are_still_rejected() {
    // `deny_unknown_fields` stays: typos never become silent defaults.
    let text = format!("{FULL_V7}top_level_unknown = true\n");
    let diagnostics = diagnostics_of(&text);
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("top_level_unknown")),
        "typo should be rejected, got {:?}",
        diagnostics
            .iter()
            .map(|diagnostic| format!(
                "{}: {}",
                diagnostic.path.as_deref().unwrap_or("-"),
                diagnostic.message
            ))
            .collect::<Vec<_>>()
    );
}

#[test]
fn version_only_skeleton_fails_on_the_required_core() {
    // version/roles/channels stay always-required: with no sensors there is no
    // role membership for the defaulted moke section to satisfy.
    let diagnostics = diagnostics_of("version = 7\n");
    assert!(
        diagnostics
            .iter()
            .any(|diagnostic| diagnostic.path.as_deref() == Some("moke.sensor")),
        "expected a required-core diagnostic, got {:?}",
        diagnostics
            .iter()
            .map(|diagnostic| format!(
                "{}: {}",
                diagnostic.path.as_deref().unwrap_or("-"),
                diagnostic.message
            ))
            .collect::<Vec<_>>()
    );
}
