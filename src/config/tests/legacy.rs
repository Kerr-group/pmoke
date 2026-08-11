use super::*;

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
fn v2_fixture_defaults_to_explicit_boxcar_kind() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    );

    let ConfigLoad::Ready { config, .. } = load_from_str(&text) else {
        panic!("expected ready load");
    };
    assert_eq!(config.lockin.lpf_kind, LockinLpfKind::BoxcarLegacy);
}

#[test]
fn v2_removed_default_lpf_requires_explicit_migration_choice() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
"#,
    )
    .replacen("lpf_kind = \"boxcar_legacy\"\n", "", 1);

    let ConfigLoad::Diagnostics(diag) = load_from_str(&text) else {
        panic!("expected migration diagnostics");
    };
    assert!(diag.diagnostics.iter().any(|issue| {
        issue.path.as_deref() == Some("lockin.lpf_kind")
            && issue.message.contains("removed fir_zero_phase default")
    }));
}

#[test]
fn v2_legacy_cutoff_fields_are_warned_and_ignored() {
    let text = v2_base_lockin(
        r#"
workers = 1
stride_samples = 1
lpf_half_window_cycles = 1.0
lpf_cutoff_hz = 0.1
lpf_cutoff_ref_ratio = 0.1
"#,
    );

    let ConfigLoad::Ready { warnings, .. } = load_from_str(&text) else {
        panic!("expected ready load");
    };
    assert!(warnings.iter().any(|warning| {
        warning.message.contains("lpf_cutoff_hz") && warning.message.contains("ignored")
    }));
}

#[test]
fn v2_removed_fir_boxcar_kind_requires_migration() {
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

    let ConfigLoad::Diagnostics(diag) = load_from_str(&text) else {
        panic!("expected migration diagnostics");
    };
    assert!(diag.diagnostics.iter().any(|issue| {
        issue.path.as_deref() == Some("lockin.lpf_kind")
            && issue.message.contains("fir_boxcar_enbw")
    }));
}

#[test]
fn v2_removed_sync_iir_zero_phase_requires_migration() {
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

    let ConfigLoad::Diagnostics(diag) = load_from_str(&text) else {
        panic!("expected migration diagnostics");
    };
    assert!(diag.diagnostics.iter().any(|issue| {
        issue.path.as_deref() == Some("lockin.lpf_kind")
            && issue.message.contains("sync_iir_zero_phase")
    }));
}

#[test]
fn v2_removed_sync_iir_zero_phase_requires_migration_even_with_invalid_order() {
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
            assert!(diag.diagnostics.iter().any(|issue| {
                issue.path.as_deref() == Some("lockin.lpf_kind")
                    && issue.message.contains("sync_iir_zero_phase")
            }));
        }
        other => panic!("expected diagnostics, got {:?}", other),
    }
}

#[test]
fn v2_removed_sync_iir_zero_phase_requires_migration_even_with_non_finite_options() {
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
            assert!(diag.diagnostics.iter().any(|issue| {
                issue.path.as_deref() == Some("lockin.lpf_kind")
                    && issue.message.contains("sync_iir_zero_phase")
            }));
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
