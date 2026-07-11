use super::*;

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
