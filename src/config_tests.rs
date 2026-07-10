use super::{
    ConfigLoad, Connection, FetchAnalysisInput, FetchOutput, LockinLpfKind, PlotDecimation,
    ValidationTarget, load_from_path, load_from_str, validate_for_target, validate_sensor_metadata,
};
use std::fs;

#[test]
fn v2_fetch_output_defaults_to_csv() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    );

    match load_from_str(&text) {
        ConfigLoad::Ready { config, .. } => {
            assert_eq!(config.fetch.output, FetchOutput::Csv);
            assert_eq!(config.fetch.analysis_input, FetchAnalysisInput::Csv);
        }
        other => panic!("expected ready load, got {other:?}"),
    }
}

#[test]
fn screenshot_config_defaults_to_disabled() {
    let text = v3_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    );

    match load_from_str(&text) {
        ConfigLoad::Ready { config, .. } => {
            assert!(!config.screenshot.enabled);
            let normalized = toml::to_string_pretty(&config).unwrap();
            assert!(normalized.contains("[screenshot]"));
            assert!(!normalized.contains("scope_path"));
            assert!(!normalized.contains("source_path"));
        }
        other => panic!("expected ready load, got {other:?}"),
    }
}

#[test]
fn screenshot_config_accepts_minimal_pc_capture_settings() {
    let text = v3_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    )
    .replacen(
        "version = 3",
        "version = 3\n\n[screenshot]\nenabled = true",
        1,
    );

    match load_from_str(&text) {
        ConfigLoad::Ready { config, .. } => {
            assert!(config.screenshot.enabled);
        }
        other => panic!("expected ready screenshot config, got {other:?}"),
    }
}

#[test]
fn screenshot_config_rejects_removed_scope_path_setting() {
    let text = v3_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    )
    .replacen(
        "version = 3",
        "version = 3\n\n[screenshot]\nenabled = true\nscope_path = \"C:/screenshot.png\"",
        1,
    );

    match load_from_str(&text) {
        ConfigLoad::Diagnostics(diag) => assert!(
            diag.diagnostics
                .iter()
                .any(|issue| issue.message.contains("unknown field `scope_path`")),
            "missing removed scope_path diagnostic: {diag:?}"
        ),
        other => panic!("expected diagnostics for removed scope_path, got {other:?}"),
    }
}

#[test]
fn screenshot_config_rejects_legacy_image_table() {
    let text = v3_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    )
    .replacen("version = 3", "version = 3\n\n[image]\nenabled = true", 1);

    match load_from_str(&text) {
        ConfigLoad::Diagnostics(diag) => assert!(
            diag.diagnostics
                .iter()
                .any(|issue| issue.message.contains("unknown field `image`")),
            "missing legacy image diagnostic: {diag:?}"
        ),
        other => panic!("expected diagnostics for legacy image config, got {other:?}"),
    }
}

#[test]
fn screenshot_target_accepts_pc_capture_transports_and_rejects_gpib() {
    for (connection, should_pass) in [
        (
            r#"{ protocol = "tcpip", ip = "192.168.10.100", port = 55255 }"#,
            true,
        ),
        (
            r#"{ protocol = "usbtmc", resource = "USB0::DHO::INSTR" }"#,
            true,
        ),
        (r#"{ protocol = "gpib", board = 0, address = 1 }"#, false),
    ] {
        let text = v3_base_lockin(
            r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
        )
        .replacen(
            "version = 3",
            &format!(
                r#"version = 3

[instruments.oscilloscope]
connection = {connection}
model = "DHO5108"

[screenshot]
enabled = true"#
            ),
            1,
        );
        let ConfigLoad::Ready { config, .. } = load_from_str(&text) else {
            panic!("expected ready config for {connection}");
        };

        assert_eq!(
            validate_for_target(&config, ValidationTarget::Screenshot).is_ok(),
            should_pass,
            "unexpected screenshot validation result for {connection}"
        );
    }
}

#[test]
fn load_from_path_records_config_location_without_serializing_it() {
    let dir = std::env::temp_dir().join(format!(
        "pmoke_screenshot_config_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&dir).unwrap();
    let path = dir.join("experiment.toml");
    fs::write(
        &path,
        v3_base_lockin(
            r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
        ),
    )
    .unwrap();

    match load_from_path(&path) {
        ConfigLoad::Ready { config, .. } => {
            assert_eq!(config.source_path, path);
            assert!(
                !toml::to_string_pretty(&config)
                    .unwrap()
                    .contains("source_path")
            );
        }
        other => panic!("expected ready load, got {other:?}"),
    }
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn v2_fetch_options_accept_raw_csv_and_auto() {
    for (output, expected_output, input, expected_input) in [
        ("raw", FetchOutput::Raw, "raw", FetchAnalysisInput::Raw),
        (
            "csv_and_raw",
            FetchOutput::CsvAndRaw,
            "auto",
            FetchAnalysisInput::Auto,
        ),
    ] {
        let text = v2_base_lockin(
            r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
        )
        .replacen(
            "version = 2",
            &format!(
                r#"version = 2

[fetch]
output = "{output}"
analysis_input = "{input}""#
            ),
            1,
        );

        match load_from_str(&text) {
            ConfigLoad::Ready { config, .. } => {
                assert_eq!(config.fetch.output, expected_output);
                assert_eq!(config.fetch.analysis_input, expected_input);
            }
            other => panic!("expected ready load for {output}/{input}, got {other:?}"),
        }
    }
}

#[test]
fn v2_plot_options_default_to_safe_noninteractive_save() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    );

    match load_from_str(&text) {
        ConfigLoad::Ready { config, .. } => {
            assert!(config.plot.enabled);
            assert!(config.plot.save);
            assert!(!config.plot.interactive);
            assert_eq!(config.plot.output_dir, "plots");
            assert_eq!(config.plot.max_points, 100_000);
            assert_eq!(config.plot.decimation, PlotDecimation::Stride);
            assert!(!config.plot.fail_on_error);
        }
        other => panic!("expected ready load, got {other:?}"),
    }
}

#[test]
fn v2_timebase_warning_explains_legacy_fallback() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    );

    match load_from_str(&text) {
        ConfigLoad::Ready { warnings, .. } => {
            let warning = warnings
                .iter()
                .find(|warning| warning.message.contains("[timebase]"))
                .expect("v2 timebase warning");
            assert!(
                warning
                    .message
                    .contains("used only when raw.csv has no time column")
            );
            assert!(warning.message.contains("recorded time axis"));
        }
        other => panic!("expected ready load, got {other:?}"),
    }
}

#[test]
fn v2_plot_options_load() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    )
    .replacen(
        "version = 2",
        r#"version = 2

[plot]
enabled = false
save = false
interactive = true
output_dir = "figures"
max_points = 1234
decimation = "stride"
fail_on_error = true"#,
        1,
    );

    match load_from_str(&text) {
        ConfigLoad::Ready { config, .. } => {
            assert!(!config.plot.enabled);
            assert!(!config.plot.save);
            assert!(config.plot.interactive);
            assert_eq!(config.plot.output_dir, "figures");
            assert_eq!(config.plot.max_points, 1234);
            assert_eq!(config.plot.decimation, PlotDecimation::Stride);
            assert!(config.plot.fail_on_error);
        }
        other => panic!("expected ready load, got {other:?}"),
    }
}

#[test]
fn v2_plot_output_dir_must_not_be_empty() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    )
    .replacen(
        "version = 2",
        r#"version = 2

[plot]
output_dir = """#,
        1,
    );

    match load_from_str(&text) {
        ConfigLoad::Diagnostics(diag) => {
            assert!(
                diag.diagnostics
                    .iter()
                    .any(|issue| issue.path.as_deref() == Some("plot.output_dir"))
            );
        }
        other => panic!("expected diagnostics, got {other:?}"),
    }
}

#[test]
fn v2_plot_max_points_must_be_positive() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    )
    .replacen(
        "version = 2",
        r#"version = 2

[plot]
max_points = 0"#,
        1,
    );

    match load_from_str(&text) {
        ConfigLoad::Diagnostics(diag) => {
            assert!(
                diag.diagnostics
                    .iter()
                    .any(|issue| issue.path.as_deref() == Some("plot.max_points"))
            );
        }
        other => panic!("expected diagnostics, got {other:?}"),
    }
}

#[test]
fn v2_usbtmc_connection_loads() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    )
    .replacen(
        "version = 2",
        r#"version = 2

[instruments.oscilloscope]
connection = { protocol = "usbtmc", resource = "USB0::0x1AB1::0x0450::DHO5A27090041::INSTR" }
model = "DHO5108""#,
        1,
    );

    match load_from_str(&text) {
        ConfigLoad::Ready { config, .. } => {
            let connection = &config.instruments.unwrap().oscilloscope.connection;
            assert!(matches!(
                connection,
                Connection::Usbtmc { resource }
                    if resource == "USB0::0x1AB1::0x0450::DHO5A27090041::INSTR"
            ));
        }
        other => panic!("expected ready load, got {other:?}"),
    }
}

#[test]
fn v2_rejects_configured_memory_depth() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    )
    .replacen(
        "version = 2",
        r#"version = 2

[instruments.oscilloscope]
connection = { protocol = "tcpip", ip = "192.168.10.100", port = 55255 }
model = "DHO5108"
memory_depth = 200_000_000"#,
        1,
    );

    match load_from_str(&text) {
        ConfigLoad::Diagnostics(diag) => {
            assert!(
                diag.diagnostics
                    .iter()
                    .any(|issue| issue.message.contains("memory_depth"))
            );
        }
        other => panic!("expected diagnostics, got {other:?}"),
    }
}

#[test]
fn v3_loads_without_timebase_and_normalized_config_omits_timebase() {
    let text = v3_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    );

    match load_from_str(&text) {
        ConfigLoad::Ready { config, .. } => {
            assert_eq!(config.version, 3);
            assert!(config.legacy_timebase.is_none());
            let normalized = toml::to_string_pretty(&config).unwrap();
            assert!(!normalized.contains("[timebase]"));
            assert!(!normalized.contains("legacy_timebase"));
        }
        other => panic!("expected ready load, got {other:?}"),
    }
}

#[test]
fn v3_rejects_timebase_section() {
    let text = v3_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    )
    .replacen(
        "version = 3",
        r#"version = 3

[timebase]
t0 = 0.0
dt = 1.0"#,
        1,
    );

    match load_from_str(&text) {
        ConfigLoad::Diagnostics(diag) => {
            assert_eq!(diag.version, Some(3));
            assert!(
                diag.diagnostics
                    .iter()
                    .any(|issue| issue.path.as_deref() == Some("timebase"))
            );
        }
        other => panic!("expected diagnostics, got {other:?}"),
    }
}

#[test]
fn expression_values_accept_pi_variable_and_function() {
    let text = v3_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    )
    .replacen(
        "m_omega_t0_offset = [0,0,0,0,0,0]",
        r#"m_omega_t0_offset = ["pi", "pi()/2", "-pi", "2*pi", 0, 1]"#,
        1,
    );

    match load_from_str(&text) {
        ConfigLoad::Ready { config, .. } => {
            assert_eq!(config.phase.m_omega_t0_offset.len(), 6);
            assert!((config.phase.m_omega_t0_offset[0] - std::f64::consts::PI).abs() < 1e-12);
            assert!((config.phase.m_omega_t0_offset[1] - std::f64::consts::PI / 2.0).abs() < 1e-12);
            assert!((config.phase.m_omega_t0_offset[2] + std::f64::consts::PI).abs() < 1e-12);
            assert!((config.phase.m_omega_t0_offset[3] - 2.0 * std::f64::consts::PI).abs() < 1e-12);
        }
        other => panic!("expected ready load, got {other:?}"),
    }
}

#[test]
fn expression_values_reject_print_call() {
    let text = v3_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    )
    .replacen(
        "m_omega_t0_offset = [0,0,0,0,0,0]",
        r#"m_omega_t0_offset = ["print(1)", 0, 0, 0, 0, 0]"#,
        1,
    );

    match load_from_str(&text) {
        ConfigLoad::Diagnostics(diag) => {
            assert!(
                diag.diagnostics
                    .iter()
                    .any(|issue| issue.message.contains("print() is not allowed"))
            );
        }
        other => panic!("expected diagnostics, got {other:?}"),
    }
}

#[test]
fn expression_values_reject_non_finite_results() {
    let text = v3_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    )
    .replacen(
        "m_omega_t0_offset = [0,0,0,0,0,0]",
        r#"m_omega_t0_offset = ["NaN", 0, 0, 0, 0, 0]"#,
        1,
    );

    match load_from_str(&text) {
        ConfigLoad::Diagnostics(diag) => {
            assert!(
                diag.diagnostics
                    .iter()
                    .any(|issue| issue.path.as_deref() == Some("phase.m_omega_t0_offset[0]"))
            );
        }
        other => panic!("expected diagnostics, got {other:?}"),
    }
}

#[test]
fn v1_filter_length_maps_to_half_window_cycles_and_legacy_boxcar() {
    let text = r#"
version = 1

[timebase]
t0 = 0.0
dt = 1.0

[roles]
sensor_ch = [1]
reference_ch = [2]
signal_ch = [3]

[[channels]]
index = 1
factor = 1.0
label = "B"
unit_out = "T"

[[channels]]
index = 2

[[channels]]
index = 3

[pulse]
bg_window_before = { start = -1.0, end = -0.5 }
bg_window_after = { start = 0.5, end = 1.0 }

[reference]
fft_window = { start = 0.0, end = 1.0 }
stride_samples = 10
window_samples = 10

[lockin]
workers = 1
stride_samples = 1
filter_length_samples = 1

[phase]
use_signal_ch = [3]
m_omega_t0_offset = [0,0,0,0,0,0]

[kerr]
use_sensor_ch = 1
kerr_type = "harmonics"
factor = 1
"#;

    let load = load_from_str(text);
    match load {
        ConfigLoad::Ready { config, warnings } => {
            assert_eq!(config.lockin.lpf_half_window_cycles, 1.0);
            assert_eq!(config.lockin.lpf_kind, LockinLpfKind::BoxcarLegacy);
            assert!(!warnings.is_empty());
        }
        other => panic!("expected ready load, got {:?}", other),
    }
}

#[test]
fn v1_phase_subset_becomes_migration_diagnostic() {
    let text = r#"
version = 1

[timebase]
t0 = 0.0
dt = 1.0

[roles]
sensor_ch = [1]
reference_ch = [2]
signal_ch = [3,4]

[[channels]]
index = 1
factor = 1.0
label = "B"
unit_out = "T"

[[channels]]
index = 2

[[channels]]
index = 3

[[channels]]
index = 4

[pulse]
bg_window_before = { start = -1.0, end = -0.5 }
bg_window_after = { start = 0.5, end = 1.0 }

[reference]
fft_window = { start = 0.0, end = 1.0 }
stride_samples = 10
window_samples = 10

[lockin]
workers = 1
stride_samples = 1
filter_length_samples = 1

[phase]
use_signal_ch = [3]
m_omega_t0_offset = [0,0,0,0,0,0]

[kerr]
use_sensor_ch = 1
kerr_type = "harmonics"
factor = 1
"#;

    match load_from_str(text) {
        ConfigLoad::Diagnostics(diag) => {
            assert_eq!(diag.version, Some(1));
            assert_eq!(diag.diagnostics.len(), 1);
        }
        other => panic!("expected diagnostics, got {:?}", other),
    }
}

#[test]
fn v2_unknown_deprecated_key_is_schema_diagnostic() {
    let text = r#"
version = 2

[timebase]
t0 = 0.0
dt = 1.0

[roles]
sensor_ch = [1]
reference_ch = 2
signal_ch = [3]

[[channels]]
index = 1
factor = 1.0
label = "B"
unit_out = "T"

[[channels]]
index = 2

[[channels]]
index = 3

[pulse]
bg_window_before = { start = -1.0, end = -0.5 }
bg_window_after = { start = 0.5, end = 1.0 }

[reference]
fft_window = { start = 0.0, end = 1.0 }
stride_samples = 10
window_samples = 10

[lockin]
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
filter_length_samples = 1

[phase]
m_omega_t0_offset = [0,0,0,0,0,0]

[kerr]
use_sensor_ch = 1
kerr_type = "harmonics"
factor = 1
"#;

    match load_from_str(text) {
        ConfigLoad::Diagnostics(diag) => {
            assert_eq!(diag.version, Some(2));
            assert!(!diag.diagnostics.is_empty());
        }
        other => panic!("expected diagnostics, got {:?}", other),
    }
}

#[test]
fn v2_fir_zero_phase_without_cutoff_warns_but_loads() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    );

    match load_from_str(&text) {
        ConfigLoad::Ready { warnings, .. } => {
            assert!(
                warnings
                    .iter()
                    .any(|warning| warning.message.contains("no cutoff is specified"))
            );
        }
        other => panic!("expected ready load, got {:?}", other),
    }
}

#[test]
fn v2_cutoff_hz_and_ratio_are_mutually_exclusive() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
lpf_cutoff_hz = 0.1
lpf_cutoff_ref_ratio = 0.1
"#,
    );

    match load_from_str(&text) {
        ConfigLoad::Diagnostics(diag) => {
            assert!(
                diag.diagnostics
                    .iter()
                    .any(|issue| issue.message.contains("mutually exclusive"))
            );
        }
        other => panic!("expected diagnostics, got {:?}", other),
    }
}

#[test]
fn v2_ignored_cutoffs_do_not_block_non_fir_zero_phase_modes() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_kind = "fir_boxcar_enbw"
lpf_half_window_cycles = 1.0
lpf_cutoff_hz = -0.1
lpf_cutoff_ref_ratio = -0.1
"#,
    );

    match load_from_str(&text) {
        ConfigLoad::Ready { .. } => {}
        other => panic!("expected ready load, got {:?}", other),
    }
}

#[test]
fn v2_sync_iir_zero_phase_loads_with_iir_options() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_kind = "sync_iir_zero_phase"
lpf_half_window_cycles = 1.0
lpf_cutoff_ref_ratio = 0.02
lpf_sync_average_cycles = 2.0
lpf_iir_order = 4
"#,
    );

    match load_from_str(&text) {
        ConfigLoad::Ready { config, .. } => {
            assert_eq!(config.lockin.lpf_kind, LockinLpfKind::SyncIirZeroPhase);
            assert_eq!(config.lockin.lpf_sync_average_cycles, 2.0);
            assert_eq!(config.lockin.lpf_iir_order, 4);
        }
        other => panic!("expected ready load, got {:?}", other),
    }
}

#[test]
fn v2_sync_iir_zero_phase_rejects_odd_iir_order() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_kind = "sync_iir_zero_phase"
lpf_half_window_cycles = 1.0
lpf_cutoff_ref_ratio = 0.02
lpf_iir_order = 3
"#,
    );

    match load_from_str(&text) {
        ConfigLoad::Diagnostics(diag) => {
            assert!(
                diag.diagnostics
                    .iter()
                    .any(|issue| issue.path.as_deref() == Some("lockin.lpf_iir_order"))
            );
        }
        other => panic!("expected diagnostics, got {:?}", other),
    }
}

#[test]
fn v2_sync_iir_zero_phase_rejects_non_finite_sync_cycles() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_kind = "sync_iir_zero_phase"
lpf_half_window_cycles = 1.0
lpf_cutoff_ref_ratio = 0.02
lpf_sync_average_cycles = inf
"#,
    );

    match load_from_str(&text) {
        ConfigLoad::Diagnostics(diag) => {
            assert!(
                diag.diagnostics
                    .iter()
                    .any(|issue| issue.path.as_deref() == Some("lockin.lpf_sync_average_cycles"))
            );
        }
        other => panic!("expected diagnostics, got {:?}", other),
    }
}

#[test]
fn v2_invalid_debug_label_is_diagnostic() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
lpf_cutoff_hz = 0.1
lpf_debug_label = "../bad"
"#,
    );

    match load_from_str(&text) {
        ConfigLoad::Diagnostics(diag) => {
            assert!(
                diag.diagnostics
                    .iter()
                    .any(|issue| issue.path.as_deref() == Some("lockin.lpf_debug_label"))
            );
        }
        other => panic!("expected diagnostics, got {:?}", other),
    }
}

#[test]
fn sensor_scale_to_abs_max_is_accepted_for_sensor_channel() {
    let text = v3_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    )
    .replacen("factor = 1.0", "scale_to_abs_max = -55.0", 1);

    let ConfigLoad::Ready { config, .. } = load_from_str(&text) else {
        panic!("expected ready load");
    };

    validate_sensor_metadata(&config).unwrap();
    assert_eq!(config.channels[0].factor, None);
    assert_eq!(config.channels[0].scale_to_abs_max, Some(-55.0));

    let normalized = toml::to_string_pretty(&config).unwrap();
    assert!(normalized.contains("scale_to_abs_max = -55.0"));
}

#[test]
fn sensor_scale_to_abs_max_rejects_ambiguous_sensor_scale() {
    let text = v3_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    )
    .replacen("factor = 1.0", "factor = 1.0\nscale_to_abs_max = 55.0", 1);

    let ConfigLoad::Ready { config, .. } = load_from_str(&text) else {
        panic!("expected ready load");
    };

    let error = validate_sensor_metadata(&config).unwrap_err();
    assert!(error.to_string().contains("cannot set both"));
}

#[test]
fn sensor_scale_to_abs_max_rejects_zero_target() {
    let text = v3_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    )
    .replacen("factor = 1.0", "scale_to_abs_max = 0.0", 1);

    let ConfigLoad::Ready { config, .. } = load_from_str(&text) else {
        panic!("expected ready load");
    };

    let error = validate_sensor_metadata(&config).unwrap_err();
    assert!(error.to_string().contains("finite and non-zero"));
}

#[test]
fn sensor_scale_to_abs_max_rejects_non_sensor_channel() {
    let text = v3_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    )
    .replacen("index = 2", "index = 2\nscale_to_abs_max = 1.0", 1);

    let ConfigLoad::Ready { config, .. } = load_from_str(&text) else {
        panic!("expected ready load");
    };

    let error = validate_sensor_metadata(&config).unwrap_err();
    assert!(error.to_string().contains("not listed in roles.sensor_ch"));
}

fn v2_base_lockin(lockin: &str) -> String {
    format!(
        r#"
version = 2

[timebase]
t0 = 0.0
dt = 1.0

[roles]
sensor_ch = [1]
reference_ch = 2
signal_ch = [3]

[[channels]]
index = 1
factor = 1.0
label = "B"
unit_out = "T"

[[channels]]
index = 2

[[channels]]
index = 3

[pulse]
bg_window_before = {{ start = -1.0, end = -0.5 }}
bg_window_after = {{ start = 0.5, end = 1.0 }}

[reference]
fft_window = {{ start = 0.0, end = 1.0 }}
stride_samples = 10
window_samples = 10

[lockin]
{lockin}

[phase]
m_omega_t0_offset = [0,0,0,0,0,0]

[kerr]
use_sensor_ch = 1
kerr_type = "harmonics"
factor = 1
"#
    )
}

fn v3_base_lockin(lockin: &str) -> String {
    format!(
        r#"
version = 3

[roles]
sensor_ch = [1]
reference_ch = 2
signal_ch = [3]

[[channels]]
index = 1
factor = 1.0
label = "B"
unit_out = "T"

[[channels]]
index = 2

[[channels]]
index = 3

[pulse]
bg_window_before = {{ start = -1.0, end = -0.5 }}
bg_window_after = {{ start = 0.5, end = 1.0 }}

[reference]
fft_window = {{ start = 0.0, end = 1.0 }}
stride_samples = 10
window_samples = 10

[lockin]
{lockin}

[phase]
m_omega_t0_offset = [0,0,0,0,0,0]

[kerr]
use_sensor_ch = 1
kerr_type = "harmonics"
factor = 1
"#
    )
}

fn v4_base() -> String {
    r#"
version = 4

[scope]
model = "DHO5108"
connection = "tcp://10.249.11.19:55255"

[data]
output = "raw"
input = "raw"
screenshot = true

[channels]
reference = 3
signals = [2]

[[sensors]]
channel = 1
scale = { factor = -39364.84663082185 }
label = "$B$"
unit = "T"

[[sensors]]
channel = 4
scale = { factor = 1.0 }
label = "$V$"
unit = "V"

[pulse]
background_before = { start = -5e-3, end = -0.1e-3 }
background_after = { start = 4.2e-3, end = 15e-3 }

[reference]
fft_window = { start = 0.0, end = 15e-3 }
stride_samples = 10_000
window_samples = 1_000

[lockin]
workers = 2
stride_samples = 100
filter = { kind = "boxcar_legacy", half_window_cycles = 1.0 }

[phase]
offsets = [0, 0, 0, 0, 0, 0]

[kerr]
sensor = 1
method = "standard"
factor = 1.0

[plot]
mode = "both"
"#
    .to_string()
}

#[test]
fn v4_base_schema_normalizes_to_runtime_config() {
    match load_from_str(&v4_base()) {
        ConfigLoad::Ready { config, warnings } => {
            assert_eq!(config.version, 4);
            assert!(warnings.is_empty());
            assert_eq!(config.fetch.output, FetchOutput::Raw);
            assert_eq!(config.fetch.analysis_input, FetchAnalysisInput::Raw);
            assert!(config.screenshot.enabled);
            assert_eq!(config.roles.sensor_ch, vec![1, 4]);
            assert_eq!(config.roles.reference_ch, 3);
            assert_eq!(config.roles.signal_ch, vec![2]);
            assert!(config.plot.enabled);
            assert!(config.plot.save);
            assert!(config.plot.interactive);
            assert_eq!(config.lockin.lpf_kind, LockinLpfKind::BoxcarLegacy);
            assert_eq!(config.lockin.lpf_half_window_cycles, 1.0);
            assert!(matches!(
                config.instruments.unwrap().oscilloscope.connection,
                Connection::Tcpip { ip, port }
                    if ip == "10.249.11.19" && port == 55255
            ));
        }
        other => panic!("expected ready v4 config, got {other:?}"),
    }
}

#[test]
fn v4_max_abs_scale_normalizes_polarity_to_signed_target() {
    let text = v4_base().replace(
        "scale = { factor = -39364.84663082185 }",
        "scale = { max_abs = 55.0, polarity = -1 }",
    );

    let ConfigLoad::Ready { config, .. } = load_from_str(&text) else {
        panic!("expected ready v4 max-abs config");
    };
    let sensor = config
        .channels
        .iter()
        .find(|channel| channel.index == 1)
        .unwrap();
    assert_eq!(sensor.factor, None);
    assert_eq!(sensor.scale_to_abs_max, Some(-55.0));
}

#[test]
fn v4_scale_rejects_mixed_or_invalid_options() {
    for replacement in [
        "scale = { factor = 1.0, max_abs = 55.0, polarity = 1 }",
        "scale = { max_abs = 0.0, polarity = 1 }",
        "scale = { max_abs = 55.0, polarity = 0 }",
        "scale = { factor = 0.0 }",
    ] {
        let text = v4_base().replace("scale = { factor = -39364.84663082185 }", replacement);
        assert!(
            matches!(load_from_str(&text), ConfigLoad::Diagnostics(_)),
            "expected v4 scale diagnostics for {replacement}"
        );
    }
}

#[test]
fn v4_rejects_channel_role_collisions() {
    let text = v4_base().replace("reference = 3", "reference = 1");
    match load_from_str(&text) {
        ConfigLoad::Diagnostics(diag) => assert!(
            diag.diagnostics
                .iter()
                .any(|issue| issue.message.contains("duplicate channel index")),
            "missing duplicate channel diagnostic: {diag:?}"
        ),
        other => panic!("expected v4 channel diagnostics, got {other:?}"),
    }
}

#[test]
fn v4_invalid_generator_connection_is_diagnostic_not_panic() {
    let text = v4_base().replacen(
        "[data]",
        "[generator]\nmodel = \"WF1946B\"\nconnection = \"not-a-connection\"\n\n[data]",
        1,
    );
    match load_from_str(&text) {
        ConfigLoad::Diagnostics(diag) => assert!(
            diag.diagnostics
                .iter()
                .any(|issue| issue.path.as_deref() == Some("generator.connection")),
            "missing generator connection diagnostic: {diag:?}"
        ),
        other => panic!("expected v4 generator diagnostics, got {other:?}"),
    }
}

#[test]
fn v4_generator_and_connection_strings_normalize() {
    let text = v4_base().replacen(
        "[data]",
        "[generator]\nmodel = \"WF1946B\"\nconnection = \"gpib://0/11\"\n\n[data]",
        1,
    );
    let ConfigLoad::Ready { config, .. } = load_from_str(&text) else {
        panic!("expected ready v4 generator config");
    };
    assert!(matches!(
        config
            .instruments
            .unwrap()
            .function_generator
            .unwrap()
            .connection,
        Connection::Gpib {
            board: 0,
            address: 11
        }
    ));

    let ipv6 = v4_base().replace("tcp://10.249.11.19:55255", "tcp://[2001:db8::1]:55255");
    let ConfigLoad::Ready { config, .. } = load_from_str(&ipv6) else {
        panic!("expected ready v4 IPv6 config");
    };
    assert!(matches!(
        config.instruments.unwrap().oscilloscope.connection,
        Connection::Tcpip { ip, port } if ip == "2001:db8::1" && port == 55255
    ));
}

#[test]
fn v4_output_both_and_filter_variants_normalize() {
    let text = v4_base()
        .replace("output = \"raw\"", "output = \"both\"")
        .replace(
            "filter = { kind = \"boxcar_legacy\", half_window_cycles = 1.0 }",
            "filter = { kind = \"sync_iir_zero_phase\", half_window_cycles = 2.0, cutoff_ref_ratio = 0.25, sync_average_cycles = 1.5, iir_order = 4 }",
        );
    let ConfigLoad::Ready { config, .. } = load_from_str(&text) else {
        panic!("expected ready v4 sync-IIR config");
    };
    assert_eq!(config.fetch.output, FetchOutput::CsvAndRaw);
    assert_eq!(config.lockin.lpf_kind, LockinLpfKind::SyncIirZeroPhase);
    assert_eq!(config.lockin.lpf_half_window_cycles, 2.0);
    assert_eq!(config.lockin.lpf_cutoff_ref_ratio, Some(0.25));
    assert_eq!(config.lockin.lpf_sync_average_cycles, 1.5);
    assert_eq!(config.lockin.lpf_iir_order, 4);
}

#[test]
fn v4_filter_rejects_fields_from_another_variant() {
    let text = v4_base().replace(
        "filter = { kind = \"boxcar_legacy\", half_window_cycles = 1.0 }",
        "filter = { kind = \"boxcar_legacy\", half_window_cycles = 1.0, cutoff_hz = 10.0 }",
    );
    assert!(matches!(load_from_str(&text), ConfigLoad::Diagnostics(_)));
}

#[test]
fn v4_plot_modes_map_to_consistent_flags() {
    for (mode, expected) in [
        ("off", (false, false, false)),
        ("save", (true, true, false)),
        ("interactive", (true, false, true)),
        ("both", (true, true, true)),
    ] {
        let text = v4_base().replace("mode = \"both\"", &format!("mode = \"{mode}\""));
        let ConfigLoad::Ready { config, .. } = load_from_str(&text) else {
            panic!("expected ready plot mode {mode}");
        };
        assert_eq!(
            (
                config.plot.enabled,
                config.plot.save,
                config.plot.interactive
            ),
            expected
        );
    }
}
