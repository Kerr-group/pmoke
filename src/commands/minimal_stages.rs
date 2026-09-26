//! Stage-minimal sensor/li fixtures (FR-02/FR-03/FR-05, Issue #264, Card B).
//!
//! The skeletons below contain exactly the declared required-item sections
//! for each stage ([`crate::config::SENSOR_REQUIRED_ITEMS`] /
//! [`crate::config::LI_REQUIRED_ITEMS`]); every other section is absent and
//! filled by the Card A inert defaults. Validation tests clear `instruments`
//! entirely (FR-03: the recorded-data gate must not require
//! `[instruments.*]`); the end-to-end tests run with the defaulted dummy
//! scope (snapshot provenance only, never dialed) and prove the stages start
//! and publish with no `[scope]` section. Negative tests prove a missing
//! required item fails with the FR-04 named diagnostic, and the invariance
//! test proves reads outside the sensor set do not change stage output.

use crate::config::{
    Config, ConfigLoad, LI_REQUIRED_ITEMS, SENSOR_REQUIRED_ITEMS, ValidationTarget, load_from_path,
    required_items_for_target, validate_for_target,
};
use std::path::{Path, PathBuf};

/// Sensor-minimal v7 skeleton: only the sensor required sections are
/// explicit. `[scope]`/`[data]`/`[reference]`/`[lockin]`/`[phase]`/`[moke]`
/// are absent (Card A inert defaults); `[plot] mode = "off"` is explicit
/// because the default plot mode renders.
const SENSOR_SKELETON: &str = r#"version = 7
[[sensors]]
channel = 1
scale = { factor = 1.0 }
label = "field"
unit = "T"
[pulse]
background_before = { start = -9e-6, end = -1e-6 }
background_after = { start = 11e-6, end = 19e-6 }
[plot]
mode = "off"
"#;

/// Lock-in-minimal v7 skeleton: the sensor skeleton plus the
/// reference/signal/lockin-class items. `[lockin] channels` derives
/// `roles.signal_ch`; `[reference]` carries the fit geometry at the fixture
/// trace scale.
const LI_SKELETON: &str = r#"version = 7
[[sensors]]
channel = 1
scale = { factor = 1.0 }
label = "field"
unit = "T"
[pulse]
background_before = { start = -9e-6, end = -1e-6 }
background_after = { start = 11e-6, end = 19e-6 }
[plot]
mode = "off"
[reference]
channel = 2
fft_window = { start = 0.0, end = 20e-6 }
stride_samples = 10
window_samples = 100
[lockin]
channels = [3]
workers = 1
stride_samples = 10
[lockin.window]
kind = "reference_cycles"
half_window_cycles = 1.0
edge_policy = "legacy_trim"
[lockin.estimator]
kind = "boxcar_legacy"
"#;

/// Synthetic trace shared by both fixtures: 3000 samples at 10 ns covering
/// [-10 us, 20 us), so both pulse background windows contain data. Channel 1
/// is the sensor (1 MHz sine plus DC), channel 2 the reference sine, channel
/// 3 the phase-shifted signal sine.
const TRACE_SAMPLES: usize = 3000;
const TRACE_DT: f64 = 10e-9;
const TRACE_T0: f64 = -10e-6;
const TRACE_FREQ: f64 = 1e6;

fn unique_run_dir(prefix: &str) -> PathBuf {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "pmoke-minimal-{prefix}-{}-{nonce}",
        std::process::id()
    ))
}

fn write_waveform_csv(waveforms_dir: &Path) {
    std::fs::create_dir_all(waveforms_dir).unwrap();
    let mut csv = String::from("t,ch1,ch2,ch3\n");
    for i in 0..TRACE_SAMPLES {
        let t = TRACE_T0 + i as f64 * TRACE_DT;
        let phase = 2.0 * std::f64::consts::PI * TRACE_FREQ * t;
        csv.push_str(&format!(
            "{t:.12e},{:.12},{:.12},{:.12}\n",
            phase.sin() + 0.3,
            phase.sin(),
            0.5 * (phase + 0.3).sin(),
        ));
    }
    std::fs::write(waveforms_dir.join("waveform.csv"), csv).unwrap();
}

/// Writes `config.toml` plus the recorded `waveform.csv` (no acquisition
/// manifest: the default Csv input needs only the waveform file) and loads
/// the config exactly as the CLI would.
fn load_fixture(root: &Path, skeleton: &str) -> Config {
    unsafe {
        std::env::set_var("MPLBACKEND", "agg");
    }
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(root.join("config.toml"), skeleton).unwrap();
    write_waveform_csv(&root.join("acquisition").join("waveforms"));
    match load_from_path(root.join("config.toml")) {
        ConfigLoad::Ready { config, .. } => config,
        ConfigLoad::Diagnostics(diagnostics) => panic!(
            "minimal skeleton must load ready, got {:?}",
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

/// Loads a skeleton for validation-only tests against a caller-owned run
/// directory (the recorded input file must exist for the target gate to
/// pass).
fn load_for_validation(root: &Path, skeleton: &str) -> Config {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(root.join("config.toml"), skeleton).unwrap();
    let mut config = match load_from_path(root.join("config.toml")) {
        ConfigLoad::Ready { config, .. } => config,
        ConfigLoad::Diagnostics(diagnostics) => panic!(
            "skeleton must load ready, got {:?}",
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
    };
    config.set_artifact_root(root.to_path_buf());
    config
}

fn sensor_csv_row_count(sensor_csv: &Path) -> usize {
    let text = std::fs::read_to_string(sensor_csv).unwrap();
    let mut lines = text.lines();
    let header = lines.next().unwrap_or_default();
    assert!(
        header.contains("field rate"),
        "unexpected sensor header: {header}"
    );
    lines.count()
}

#[test]
fn required_sets_are_declared_for_sensor_and_li_only() {
    assert_eq!(
        required_items_for_target(ValidationTarget::Sensor),
        Some(SENSOR_REQUIRED_ITEMS)
    );
    // The li set is a strict superset: `run_sensor_stage` runs first, so
    // every sensor item stays required.
    let li = required_items_for_target(ValidationTarget::Li).expect("li set is declared");
    assert_eq!(li, LI_REQUIRED_ITEMS);
    for item in SENSOR_REQUIRED_ITEMS {
        assert!(li.contains(item), "li set must keep sensor item {item}");
    }
    for item in [
        "reference.channel",
        "reference.fft_window",
        "reference.stride_samples",
        "reference.window_samples",
        "lockin.channels",
        "lockin.workers",
        "lockin.window",
        "lockin.estimator",
    ] {
        assert!(li.contains(&item), "li set must declare {item}");
    }
    // Card B declares no other target: acquisition commands and the other
    // analysis stages keep their existing behavior.
    for target in [
        ValidationTarget::Single,
        ValidationTarget::Fetch,
        ValidationTarget::Auto,
        ValidationTarget::Reference,
        ValidationTarget::Signal,
        ValidationTarget::Analyze,
    ] {
        assert!(
            required_items_for_target(target).is_none(),
            "no required set is declared for {target:?}"
        );
    }
}

#[test]
fn sensor_and_li_validate_with_instruments_absent() {
    let root = unique_run_dir("noscope-validation");
    let mut sensor = load_for_validation(&root, SENSOR_SKELETON);
    let mut li = load_for_validation(&root, LI_SKELETON);
    write_waveform_csv(&root.join("acquisition").join("waveforms"));
    // FR-03: recorded-data validation must not require `[instruments.*]`,
    // so even a fully absent section passes.
    sensor.instruments = None;
    li.instruments = None;
    validate_for_target(&sensor, ValidationTarget::Sensor).unwrap();
    validate_for_target(&li, ValidationTarget::Li).unwrap();
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn acquisition_targets_still_require_instruments() {
    let root = unique_run_dir("acquisition-gate");
    let mut config = load_for_validation(&root, SENSOR_SKELETON);
    write_waveform_csv(&root.join("acquisition").join("waveforms"));
    config.instruments = None;
    for target in [ValidationTarget::Single, ValidationTarget::Fetch] {
        let error = validate_for_target(&config, target).unwrap_err();
        assert!(
            error.to_string().contains("instruments"),
            "{target:?} must still require instruments, got: {error:#}"
        );
    }
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn missing_required_items_name_the_item() {
    let root = unique_run_dir("missing-items");
    let mut sensor = load_for_validation(&root, SENSOR_SKELETON);
    let mut li = load_for_validation(&root, LI_SKELETON);
    write_waveform_csv(&root.join("acquisition").join("waveforms"));

    sensor.roles.sensor_ch.clear();
    let error = validate_for_target(&sensor, ValidationTarget::Sensor).unwrap_err();
    assert!(
        error.to_string().contains("roles.sensor_ch"),
        "sensor gate must name roles.sensor_ch, got: {error:#}"
    );

    sensor.roles.sensor_ch = vec![1];
    sensor
        .channels
        .iter_mut()
        .find(|channel| channel.index == 1)
        .expect("sensor channel entry exists")
        .label = None;
    let error = validate_for_target(&sensor, ValidationTarget::Sensor).unwrap_err();
    assert!(
        error.to_string().contains("label"),
        "sensor gate must name the missing label, got: {error:#}"
    );

    li.roles.reference_ch = 0;
    let error = validate_for_target(&li, ValidationTarget::Li).unwrap_err();
    assert!(
        error.to_string().contains("roles.reference_ch"),
        "li gate must name roles.reference_ch, got: {error:#}"
    );

    li.roles.reference_ch = 2;
    li.roles.signal_ch.clear();
    let error = validate_for_target(&li, ValidationTarget::Li).unwrap_err();
    assert!(
        error.to_string().contains("roles.signal_ch"),
        "li gate must name roles.signal_ch, got: {error:#}"
    );

    std::fs::remove_file(
        root.join("acquisition")
            .join("waveforms")
            .join("waveform.csv"),
    )
    .unwrap();
    sensor
        .channels
        .iter_mut()
        .find(|channel| channel.index == 1)
        .expect("sensor channel entry exists")
        .label = Some("field".to_string());
    let error = validate_for_target(&sensor, ValidationTarget::Sensor).unwrap_err();
    assert!(
        error.to_string().contains("waveform.csv"),
        "recorded-data gate must name waveform.csv, got: {error:#}"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn config_validate_without_target_keeps_full_validation() {
    // `pmoke config validate` runs the load path with no target, so the full
    // load-time rules still apply to stage-minimal configs: the clean
    // skeleton is Ready, and a full-validation violation is Diagnostics.
    assert!(matches!(
        crate::config::load_from_str(SENSOR_SKELETON),
        ConfigLoad::Ready { .. }
    ));
    let bad_plot = SENSOR_SKELETON.replace(
        "[plot]\nmode = \"off\"",
        "[plot]\nmode = \"off\"\nmax_points = 0",
    );
    match crate::config::load_from_str(&bad_plot) {
        ConfigLoad::Ready { .. } => panic!("max_points = 0 must not validate"),
        ConfigLoad::Diagnostics(diagnostics) => assert!(
            diagnostics
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.path.as_deref() == Some("plot.max_points")),
            "full validation must flag plot.max_points, got {:?}",
            diagnostics
                .diagnostics
                .iter()
                .map(|diagnostic| (diagnostic.path.clone(), diagnostic.message.clone()))
                .collect::<Vec<_>>()
        ),
    }
}

#[test]
fn sensor_runs_from_stage_minimal_config_without_scope_section() {
    let root = unique_run_dir("sensor");
    let config = load_fixture(&root, SENSOR_SKELETON);
    // No `[scope]` in the skeleton: the Card A dummy (loopback discard,
    // snapshot provenance only, never dialed by analysis stages) fills it.
    // FR-03 removes the validation gate, so the recorded-data run proceeds.
    let oscilloscope = &config
        .instruments
        .as_ref()
        .expect("defaulted scope keeps instruments present")
        .oscilloscope;
    assert_eq!(oscilloscope.model, "DHO5108");

    crate::commands::sensor::sensor(&config).unwrap();

    let sensor_csv = config.paths().sensor_csv();
    assert!(
        sensor_csv.is_file(),
        "sensor must publish {}",
        sensor_csv.display()
    );
    // Defaulted lockin stride 100 over 3000 samples: floor((3000 - 1) / 100) + 1.
    assert_eq!(sensor_csv_row_count(&sensor_csv), 30);
    assert!(
        config.paths().run_manifest().is_file(),
        "sensor must leave a run manifest"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn li_runs_from_stage_minimal_config_without_scope_section() {
    let root = unique_run_dir("li");
    let config = load_fixture(&root, LI_SKELETON);

    crate::commands::li::li(&config).unwrap();

    let li_csv = config.paths().lockin_xy_csv(3);
    assert!(li_csv.is_file(), "li must publish {}", li_csv.display());
    assert!(
        config.paths().run_manifest().is_file(),
        "li must leave a run manifest"
    );
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn sensor_output_ignores_sections_outside_its_required_set() {
    // Reads outside the sensor required set are forbidden: the same recorded
    // data run from a skeleton carrying valid-but-different unrelated
    // sections must publish byte-identical output. Every table below is
    // complete (inner sections have no partial defaults); the values are
    // valid yet distinct from the Card A defaults, except
    // `lockin.stride_samples`, which is a sensor-required item and must stay
    // on the same grid for the outputs to be comparable.
    let unrelated = r#"[reference]
channel = 2
fft_window = { start = 1.0, end = 2.0 }
stride_samples = 7
window_samples = 13
[lockin]
channels = []
workers = 4
stride_samples = 100
[lockin.window]
kind = "reference_cycles"
half_window_cycles = 2.0
edge_policy = "legacy_trim"
[lockin.estimator]
kind = "boxcar_legacy"
[phase]
offsets = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6]
[moke]
sensor = 1
method = "harmonics"
factor = 2.5
"#;
    let root_a = unique_run_dir("sensor-set-a");
    let root_b = unique_run_dir("sensor-set-b");
    let skeleton_b = format!("{SENSOR_SKELETON}{unrelated}");
    let config_a = load_fixture(&root_a, SENSOR_SKELETON);
    let config_b = load_fixture(&root_b, &skeleton_b);

    crate::commands::sensor::sensor(&config_a).unwrap();
    crate::commands::sensor::sensor(&config_b).unwrap();

    let csv_a = std::fs::read(config_a.paths().sensor_csv()).unwrap();
    let csv_b = std::fs::read(config_b.paths().sensor_csv()).unwrap();
    assert_eq!(
        csv_a, csv_b,
        "unrelated sections must not change sensor output"
    );
    std::fs::remove_dir_all(&root_a).unwrap();
    std::fs::remove_dir_all(&root_b).unwrap();
}
