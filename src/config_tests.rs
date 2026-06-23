use super::{
    ConfigLoad, Connection, FetchAnalysisInput, FetchOutput, LockinLpfKind, PlotDecimation,
    load_from_str,
};

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
model = "DHO5108"
memory_depth = 200_000_000"#,
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
