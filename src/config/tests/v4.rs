use super::*;
use std::path::PathBuf;

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
channel = 3
fft_window = { start = 0.0, end = 15e-3 }
stride_samples = 10_000
window_samples = 1_000

[lockin]
signal_channels = [2]
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
fn v4_normalized_output_uses_v4_schema_and_round_trips() {
    let text = v4_base().replace(
        "scale = { factor = -39364.84663082185 }",
        "scale = { max_abs = 55.0, polarity = -1 }",
    );
    let ConfigLoad::Ready { config, .. } = load_from_str(&text) else {
        panic!("expected ready v4 config");
    };

    let rendered = render_normalized_config(&config).unwrap();
    assert!(rendered.contains("[scope]"));
    assert!(rendered.contains("[[sensors]]"));
    assert!(rendered.contains("[reference]"));
    assert!(rendered.contains("channel = 3"));
    assert!(rendered.contains("signal_channels = [2]"));
    assert!(!rendered.contains("[instruments]"));
    assert!(!rendered.contains("[roles]"));
    assert!(!rendered.contains("[channels]"));

    let ConfigLoad::Ready {
        config: round_trip, ..
    } = load_from_str(&rendered)
    else {
        panic!("rendered v4 config must be readable:\n{rendered}");
    };
    let sensor = round_trip
        .channels
        .iter()
        .find(|channel| channel.index == 1)
        .unwrap();
    assert_eq!(sensor.scale_to_abs_max, Some(-55.0));
    assert_eq!(round_trip.fetch.output, FetchOutput::Raw);
    assert_eq!(round_trip.lockin.lpf_kind, LockinLpfKind::BoxcarLegacy);
}

#[test]
fn v4_rejects_removed_channels_role_table() {
    let text = v4_base()
        .replace("channel = 3\nfft_window", "fft_window")
        .replace("signal_channels = [2]\n", "")
        .replacen(
            "[[sensors]]",
            "[channels]\nreference = 3\nsignals = [2]\n\n[[sensors]]",
            1,
        );
    assert!(matches!(load_from_str(&text), ConfigLoad::Diagnostics(_)));
}

#[test]
fn v4_requires_local_reference_and_signal_channel_assignments() {
    for text in [
        v4_base().replace("channel = 3\nfft_window", "fft_window"),
        v4_base().replace("signal_channels = [2]\n", ""),
    ] {
        assert!(matches!(load_from_str(&text), ConfigLoad::Diagnostics(_)));
    }
}

#[test]
fn v4_artifacts_are_relative_to_config_directory() {
    let dir = std::env::temp_dir().join(format!(
        "pmoke_v4_artifacts_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir(&dir).unwrap();
    let path = dir.join("experiment.toml");
    fs::write(&path, v4_base()).unwrap();

    let ConfigLoad::Ready { config, .. } = load_from_path(&path) else {
        panic!("expected ready v4 config from path");
    };
    assert_eq!(config.artifact_path("raw.csv"), dir.join("raw.csv"));
    assert_eq!(
        config.artifact_path("raw_waveform"),
        dir.join("raw_waveform")
    );
    assert_eq!(config.plot.output_dir, dir.join("plots").to_string_lossy());

    fs::remove_dir_all(dir).unwrap();
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
    let text = v4_base().replace("[reference]\nchannel = 3", "[reference]\nchannel = 1");
    match load_from_str(&text) {
        ConfigLoad::Diagnostics(diag) => assert!(
            diag.diagnostics.iter().any(|issue| {
                issue.path.as_deref() == Some("reference.channel")
                    && issue.message.contains("assigned more than once")
            }),
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
fn v4_validation_reports_v4_field_names() {
    let text = v4_base()
        .replace(
            "filter = { kind = \"boxcar_legacy\", half_window_cycles = 1.0 }",
            "filter = { kind = \"boxcar_legacy\", half_window_cycles = 0.0 }",
        )
        .replace("offsets = [0, 0, 0, 0, 0, 0]", "offsets = [0, 0, 0]");
    let ConfigLoad::Diagnostics(diagnostics) = load_from_str(&text) else {
        panic!("expected v4 validation diagnostics");
    };
    assert!(diagnostics.diagnostics.iter().any(|diagnostic| {
        diagnostic.path.as_deref() == Some("lockin.filter.half_window_cycles")
            && diagnostic
                .message
                .contains("lockin.filter.half_window_cycles")
    }));
    assert!(diagnostics.diagnostics.iter().any(|diagnostic| {
        diagnostic.path.as_deref() == Some("phase.offsets")
            && diagnostic.message.contains("phase.offsets")
    }));
}

#[test]
fn v4_rejects_non_finite_time_windows() {
    let text = v4_base().replace(
        "fft_window = { start = 0.0, end = 15e-3 }",
        "fft_window = { start = -inf, end = 15e-3 }",
    );
    let ConfigLoad::Diagnostics(diagnostics) = load_from_str(&text) else {
        panic!("expected non-finite window diagnostic");
    };
    assert!(diagnostics.diagnostics.iter().any(|diagnostic| {
        diagnostic.path.as_deref() == Some("reference.fft_window")
            && diagnostic.message.contains("must be finite")
    }));
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

#[test]
fn v4_plot_decimation_modes_are_explicit() {
    for (name, expected) in [
        ("none", PlotDecimation::None),
        ("stride", PlotDecimation::Stride),
        ("min_max", PlotDecimation::MinMax),
    ] {
        let text = v4_base().replace(
            "mode = \"both\"",
            &format!("mode = \"both\"\ndecimation = \"{name}\""),
        );
        let ConfigLoad::Ready { config, .. } = load_from_str(&text) else {
            panic!("expected ready plot decimation {name}");
        };
        assert_eq!(config.plot.decimation, expected);
    }
}

#[test]
fn artifact_root_is_an_opt_in_override_for_relative_outputs() {
    let ConfigLoad::Ready { mut config, .. } = load_from_str(&v4_base()) else {
        panic!("expected ready v4 config");
    };
    config.set_artifact_root(PathBuf::from("shot_000123"));

    assert_eq!(
        config.artifact_path("raw_waveform"),
        PathBuf::from("shot_000123").join("raw_waveform")
    );
    assert_eq!(
        PathBuf::from(&config.plot.output_dir),
        PathBuf::from("shot_000123").join("plots")
    );
}

#[test]
fn artifact_root_does_not_rewrite_an_absolute_plot_directory() {
    let absolute = std::env::temp_dir().join("pmoke-absolute-plots");
    let text = v4_base().replace(
        "mode = \"both\"",
        &format!(
            "mode = \"both\"\noutput_dir = {:?}",
            absolute.to_string_lossy()
        ),
    );
    let ConfigLoad::Ready { mut config, .. } = load_from_str(&text) else {
        panic!("expected ready v4 config");
    };
    config.set_artifact_root(PathBuf::from("shot_000123"));

    assert_eq!(PathBuf::from(config.plot.output_dir), absolute);
}
