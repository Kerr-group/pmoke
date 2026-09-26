use super::*;

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmoke_calibrate_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Deterministic recorded waveform: 8 training blocks of a noisy tone plus
/// a tuning-validation interval, all at dt=1e-5, f_ref=1 kHz.
fn synthetic_waveform_csv(path: &Path, blocks: usize, block_len: usize) {
    let dt = 1.0e-5;
    let mut text = String::from("time (s),ch3\n");
    let mut state = 0x1234_5678_9abc_def1u64;
    let mut rand = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state as f64 / u64::MAX as f64) * 2.0 - 1.0
    };
    for i in 0..blocks * block_len {
        let time = i as f64 * dt;
        let tone = (2.0 * std::f64::consts::PI * 1_000.0 * time + 0.3).sin();
        let value = tone + 0.05 * rand();
        text.push_str(&format!("{time:.10e},{value:.10e}\n"));
    }
    std::fs::write(path, text).unwrap();
}

fn build_request_text(
    waveform: &str,
    output: &str,
    training_ranges: &[(u64, u64)],
    tuning_start: u64,
    total: u64,
) -> String {
    let mut text = format!(
        r#"schema_version = 1
model_id = "r1-test-model"
waveform_csv = "{waveform}"
channel_column = 1
sample_interval_s = 0.00001
reference_frequency_hz = 1000.0
reference_phase_rad = 0.0
channel = 3
seed = 7
block_len = 12800
output = "{output}"
"#
    );
    for (start, end) in training_ranges {
        text.push_str(&format!(
            "\n[[intervals]]\nrole = \"training\"\nstart = {start}\nend = {end}\n"
        ));
    }
    text.push_str(&format!(
        "\n[[intervals]]\nrole = \"tuning_validation\"\nstart = {tuning_start}\nend = {total}\n"
    ));
    text
}

#[test]
fn build_inspect_validate_round_trip() {
    let dir = temp_dir("roundtrip");
    let blocks = 10;
    let block_len = 12_800;
    // 8 training x 12800 @128 cycles/block, bins=8 default: ~12800/bin, ~128 cycles/bin.
    let waveform = dir.join("wave.csv");
    synthetic_waveform_csv(&waveform, blocks, block_len);
    let total = (blocks * block_len) as u64;
    let mid = (4 * block_len) as u64;
    let training_end = (8 * block_len) as u64;
    let request_path = dir.join("request.toml");
    let request_text = build_request_text(
        "wave.csv",
        "model",
        &[(0, mid), (mid, training_end)],
        training_end,
        total,
    );
    std::fs::write(&request_path, request_text).unwrap();
    run_build(&request_path, None).unwrap();

    let artifact = dir.join("model/calibration.json");
    assert!(artifact.is_file());
    let report: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("model/calibrate-report.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["model_id"], "r1-test-model");
    assert_eq!(report["adequate"], true);
    let digest = crate::utils::checksum::file_sha256(&artifact).unwrap();
    assert_eq!(report["sha256"], digest);

    // Tolerance basis record (Issue #274 FR-03): explicit-frequency TOML
    // build carries no build-side uncertainty, so the floor combination
    // applies and nothing is marked overridden.
    let basis = &report["tolerance_basis"];
    assert_eq!(basis["u_build"], serde_json::Value::Null);
    assert_eq!(basis["u_apply"], serde_json::Value::Null);
    assert_eq!(basis["coverage_k"], serde_json::json!(3.0));
    assert_eq!(basis["floor"], serde_json::json!(1e-9));
    assert_eq!(basis["final_tol"], serde_json::json!(1e-9));
    assert_eq!(basis["overridden"], serde_json::Value::Bool(false));
    let artifact_value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&artifact).unwrap()).unwrap();
    assert_eq!(
        artifact_value["binding"]["frequency_rel_tol"],
        serde_json::json!(1e-9)
    );
    assert_eq!(
        artifact_value["binding"]["reference_frequency_rel_uncertainty"],
        serde_json::Value::Null
    );
    assert_eq!(
        artifact_value["builder"]["recipe_settings"]["frequency_tol_overridden"],
        serde_json::json!("false")
    );

    // Deterministic bytes: rebuild into a second directory matches exactly.
    let request2 = dir.join("request2.toml");
    std::fs::write(
        &request2,
        build_request_text(
            "wave.csv",
            "model2",
            &[(0, mid), (mid, training_end)],
            training_end,
            total,
        ),
    )
    .unwrap();
    run_build(&request2, None).unwrap();
    assert_eq!(
        std::fs::read(dir.join("model/calibration.json")).unwrap(),
        std::fs::read(dir.join("model2/calibration.json")).unwrap()
    );

    // Inspect passes on the published artifact.
    run_inspect(&artifact).unwrap();

    // Validate passes against a matching context.
    let context = dir.join("context.toml");
    std::fs::write(
        &context,
        r#"channel = 3
sample_interval_s = 0.00001
reference_frequency_hz = 1000.0
voltage_unit = "V"
phase_convention = "phi=2*pi*f*t-reference_phase_rad"

[acquisition]
"#,
    )
    .unwrap();
    run_validate(&artifact, &context).unwrap();

    // A mismatched channel fails validation without touching the artifact.
    let bad_context = dir.join("bad-context.toml");
    std::fs::write(
        &bad_context,
        r#"channel = 5
sample_interval_s = 0.00001
reference_frequency_hz = 1000.0
voltage_unit = "V"
phase_convention = "phi=2*pi*f*t-reference_phase_rad"

[acquisition]
"#,
    )
    .unwrap();
    assert!(run_validate(&artifact, &bad_context).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Issue #274 FR-04: an explicit TOML tolerance override is honored and
/// marked as overridden in both the report and the artifact record.
#[test]
fn build_override_tol_is_honored_and_marked() {
    let dir = temp_dir("override");
    let blocks = 10;
    let block_len = 12_800;
    let waveform = dir.join("wave.csv");
    synthetic_waveform_csv(&waveform, blocks, block_len);
    let total = (blocks * block_len) as u64;
    let mid = (4 * block_len) as u64;
    let training_end = (8 * block_len) as u64;
    let request_path = dir.join("request.toml");
    let mut text = build_request_text(
        "wave.csv",
        "model",
        &[(0, mid), (mid, training_end)],
        training_end,
        total,
    );
    // Root-table key: prepend before the [[intervals]] tables so TOML
    // assigns it to the request (not to the last interval table).
    text = format!("frequency_rel_tol = 1e-7\n{text}");
    std::fs::write(&request_path, text).unwrap();
    run_build(&request_path, None).unwrap();

    let report: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("model/calibrate-report.json")).unwrap(),
    )
    .unwrap();
    let basis = &report["tolerance_basis"];
    assert_eq!(basis["overridden"], serde_json::Value::Bool(true));
    assert_eq!(basis["final_tol"], serde_json::json!(1e-7));
    let artifact_value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("model/calibration.json")).unwrap())
            .unwrap();
    assert_eq!(
        artifact_value["binding"]["frequency_rel_tol"],
        serde_json::json!(1e-7)
    );
    assert_eq!(
        artifact_value["builder"]["recipe_settings"]["frequency_tol_overridden"],
        serde_json::json!("true")
    );

    // A non-positive override is a hard build error, never silent.
    let bad_path = dir.join("bad-request.toml");
    let bad_text = build_request_text(
        "wave.csv",
        "bad",
        &[(0, mid), (mid, training_end)],
        training_end,
        total,
    );
    let bad_text = format!("frequency_rel_tol = -1e-9\n{bad_text}");
    std::fs::write(&bad_path, bad_text).unwrap();
    assert!(run_build(&bad_path, None).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Issue #274 FR-02/FR-06 end to end: a TOML context carrying the
/// inference-side uncertainty admits the 5.852e-9 same-condition pair
/// through `calibrate validate`, while the field stays optional (contexts
/// without it keep the exact historical gate) and 2.5% still refuses.
#[test]
fn validate_with_apply_uncertainty_admits_ppb_pair() {
    let dir = temp_dir("validate-u");
    let blocks = 10;
    let block_len = 12_800;
    let waveform = dir.join("wave.csv");
    synthetic_waveform_csv(&waveform, blocks, block_len);
    let total = (blocks * block_len) as u64;
    let mid = (4 * block_len) as u64;
    let training_end = (8 * block_len) as u64;
    let request_path = dir.join("request.toml");
    std::fs::write(
        &request_path,
        build_request_text(
            "wave.csv",
            "model",
            &[(0, mid), (mid, training_end)],
            training_end,
            total,
        ),
    )
    .unwrap();
    run_build(&request_path, None).unwrap();
    let artifact = dir.join("model/calibration.json");

    // ppb-deviated context without the uncertainty field: refuses
    // (historical gate, floor 1e-9).
    let plain = dir.join("plain.toml");
    std::fs::write(
        &plain,
        r#"channel = 3
sample_interval_s = 0.00001
reference_frequency_hz = 1000.000005852
voltage_unit = "V"
phase_convention = "phi=2*pi*f*t-reference_phase_rad"

[acquisition]
"#,
    )
    .unwrap();
    assert!(run_validate(&artifact, &plain).is_err());

    // Same pair with the inference-side uncertainty recorded: validates.
    let with_u = dir.join("with-u.toml");
    std::fs::write(
        &with_u,
        r#"channel = 3
sample_interval_s = 0.00001
reference_frequency_hz = 1000.000005852
voltage_unit = "V"
phase_convention = "phi=2*pi*f*t-reference_phase_rad"
reference_frequency_rel_uncertainty = 2e-9

[acquisition]
"#,
    )
    .unwrap();
    run_validate(&artifact, &with_u).unwrap();

    // A true 2.5% mismatch refuses even with the uncertainty recorded.
    let far = dir.join("far.toml");
    std::fs::write(
        &far,
        r#"channel = 3
sample_interval_s = 0.00001
reference_frequency_hz = 1025.0
voltage_unit = "V"
phase_convention = "phi=2*pi*f*t-reference_phase_rad"
reference_frequency_rel_uncertainty = 2e-9

[acquisition]
"#,
    )
    .unwrap();
    assert!(run_validate(&artifact, &far).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn build_rejects_insufficient_and_existing_output() {
    let dir = temp_dir("rejects");
    // Two blocks only: below the minimum contributing-block policy.
    let waveform = dir.join("short.csv");
    synthetic_waveform_csv(&waveform, 2, 12800);
    let request_path = dir.join("request.toml");
    std::fs::write(
        &request_path,
        build_request_text("short.csv", "model", &[(0, 25600)], 0, 25600),
    )
    .unwrap();
    assert!(run_build(&request_path, None).is_err());
    assert!(!dir.join("model").exists());

    // Existing output is never overwritten.
    let waveform = dir.join("wave.csv");
    synthetic_waveform_csv(&waveform, 10, 12800);
    std::fs::write(
        &request_path,
        build_request_text(
            "wave.csv",
            "model",
            &[(0, 51200), (51200, 102400)],
            102400,
            128000,
        ),
    )
    .unwrap();
    run_build(&request_path, None).unwrap();
    assert!(run_build(&request_path, None).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn inspect_rejects_non_artifact() {
    let dir = temp_dir("inspect");
    let bad = dir.join("bad.json");
    std::fs::write(&bad, b"{\"schema_version\": 999}").unwrap();
    assert!(run_inspect(&bad).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

// ---------------------------------------------------------------------------
// Direct RAW build (Issue #270) and background-interval contract (#269).
// ---------------------------------------------------------------------------

/// Direct-build fixture geometry: 128 000 samples at dt=1e-5, f_ref=1 kHz.
const DIRECT_SAMPLES: usize = 128_000;
const DIRECT_DT_TEXT: &str = "0.00001";
const DIRECT_Y_INC: f64 = 1.0e-4;
const DIRECT_Y_ORIGIN: f64 = 20_000.0;

/// Writes a recorded RAW run: legacy v1 acquisition manifest, quantized
/// target (ch2) plus reference (ch3) channels, and a v7 configuration
/// snapshot with `reference.channel = 3` and `lockin.channels = [2]`.
fn write_direct_raw_run(dir: &Path) {
    let acquisition = dir.join("acquisition");
    let waveforms = acquisition.join("waveforms");
    std::fs::create_dir_all(&waveforms).unwrap();
    for (channel, seed) in [
        (2u8, 0x1111_2222_3333_4444u64),
        (3u8, 0x5555_6666_7777_8888u64),
    ] {
        let mut state = seed;
        let mut rand = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state as f64 / u64::MAX as f64) * 2.0 - 1.0
        };
        let mut bytes = Vec::with_capacity(DIRECT_SAMPLES * 2);
        for i in 0..DIRECT_SAMPLES {
            let time = i as f64 * 1.0e-5;
            let tone = (2.0 * std::f64::consts::PI * 1_000.0 * time + 0.3).sin();
            let value = tone + 0.03 * rand();
            let word = ((value / DIRECT_Y_INC) + DIRECT_Y_ORIGIN)
                .round()
                .clamp(0.0, 65535.0) as u16;
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        std::fs::write(waveforms.join(format!("ch{channel}.u16le")), bytes).unwrap();
    }
    let manifest = format!(
        "version = 1\nchannels = [\n  {{ file = \"waveforms/ch2.u16le\", sample_count = {DIRECT_SAMPLES}, \
         x_increment = {DIRECT_DT_TEXT}, x_origin = 0.0, x_reference = 0.0, \
         y_increment = 0.0001, y_origin = 20000.0, y_reference = 0.0 }},\n  {{ file = \"waveforms/ch3.u16le\", sample_count = {DIRECT_SAMPLES}, \
         x_increment = {DIRECT_DT_TEXT}, x_origin = 0.0, x_reference = 0.0, \
         y_increment = 0.0001, y_origin = 20000.0, y_reference = 0.0 }},\n]\n\n\
         [oscilloscope]\nwaveform_format = \"WORD\"\nbyte_order = \"little-endian\"\n"
    );
    std::fs::write(acquisition.join("manifest.toml"), manifest).unwrap();
    let snapshot = r#"version = 7
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
channel = 3
fft_window = { start = 0.0, end = 0.05 }
stride_samples = 2000
window_samples = 4000
[lockin]
channels = [2]
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
    std::fs::write(dir.join("config.resolved.toml"), snapshot).unwrap();
}

/// Sorted `(relative path, sha256)` view of a directory tree.
fn dir_snapshot(dir: &Path) -> Vec<(String, String)> {
    fn walk(base: &Path, current: &Path, out: &mut Vec<(String, String)>) {
        let mut entries: Vec<_> = std::fs::read_dir(current)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                let relative = path
                    .strip_prefix(base)
                    .unwrap()
                    .display()
                    .to_string()
                    .replace('\\', "/");
                let digest = crate::utils::checksum::file_sha256(&path).unwrap();
                out.push((relative, digest));
            }
        }
    }
    let mut snapshot = Vec::new();
    walk(dir, dir, &mut snapshot);
    snapshot
}

#[test]
fn direct_role_intervals_split_70_15_15_with_two_training_intervals() {
    let intervals = direct_role_intervals(128_000).unwrap();
    let spans: Vec<(&str, u64, u64)> = intervals
        .iter()
        .map(|interval| match interval.role {
            CalibrationRole::Training => ("training", interval.start, interval.end),
            CalibrationRole::TuningValidation => {
                ("tuning_validation", interval.start, interval.end)
            }
            CalibrationRole::Evaluation => ("evaluation", interval.start, interval.end),
        })
        .collect();
    // skip = 2560; usable = 125440; train = 87808; tune = 18816; eval = 18816.
    assert_eq!(
        spans,
        vec![
            ("training", 2560, 46_464),
            ("training", 46_464, 90_368),
            ("tuning_validation", 90_368, 109_184),
            ("evaluation", 109_184, 128_000),
        ]
    );
    assert!(direct_role_intervals(0).is_err());
    assert!(direct_role_intervals(3).is_err());
}

#[test]
fn resolve_direct_run_dir_defaults_to_cwd() {
    assert_eq!(resolve_direct_run_dir(None), PathBuf::from("."));
    assert_eq!(
        resolve_direct_run_dir(Some(Path::new("shot13"))),
        PathBuf::from("shot13")
    );
}

#[test]
fn default_output_inside_run_is_rejected() {
    let dir = temp_dir("guard");
    let run = dir.join("shot13");
    std::fs::create_dir_all(&run).unwrap();
    let inside = lexical_absolute(&run.join("calibration"));
    let error = reject_default_output_inside_run(&inside, &run)
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("pass --output"), "{error:#}");
    let sibling = lexical_absolute(&dir.join("calibration"));
    reject_default_output_inside_run(&sibling, &run).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn cli_build_parses_direct_and_request_modes() {
    use crate::cli::{CalibrateCommand, Cli, Command};
    use clap::Parser;
    // Bare `--channel 2` is a direct build resolving `--run` to `.`.
    let cli = Cli::try_parse_from(["pmoke", "calibrate", "build", "--channel", "2"]).unwrap();
    match cli.command {
        Some(Command::Calibrate {
            command:
                CalibrateCommand::Build {
                    request,
                    run,
                    channel,
                    ..
                },
        }) => {
            assert!(request.is_none());
            assert!(run.is_none());
            assert_eq!(resolve_direct_run_dir(run.as_deref()), PathBuf::from("."));
            assert_eq!(channel, vec![2]);
        }
        other => panic!("unexpected command: {other:?}"),
    }
    let cli = Cli::try_parse_from([
        "pmoke",
        "calibrate",
        "build",
        "--run",
        "shot13",
        "--channel",
        "2",
        "--channel",
        "3",
    ])
    .unwrap();
    match cli.command {
        Some(Command::Calibrate {
            command: CalibrateCommand::Build { run, channel, .. },
        }) => {
            assert_eq!(run, Some(PathBuf::from("shot13")));
            assert_eq!(channel, vec![2, 3]);
        }
        other => panic!("unexpected command: {other:?}"),
    }
    // Legacy `--request` conflicts with direct-build flags.
    assert!(
        Cli::try_parse_from([
            "pmoke",
            "calibrate",
            "build",
            "--request",
            "r.toml",
            "--channel",
            "2"
        ])
        .is_err()
    );
    let cli = Cli::try_parse_from(["pmoke", "calibrate", "build", "--request", "r.toml"]).unwrap();
    match cli.command {
        Some(Command::Calibrate {
            command: CalibrateCommand::Build { request, .. },
        }) => {
            assert_eq!(request, Some(PathBuf::from("r.toml")));
        }
        other => panic!("unexpected command: {other:?}"),
    }
}

#[test]
fn direct_build_raw_green_and_source_run_immutable() {
    let dir = temp_dir("direct");
    let run = dir.join("shot13");
    std::fs::create_dir_all(&run).unwrap();
    write_direct_raw_run(&run);
    let before = dir_snapshot(&run);
    assert!(!before.is_empty());

    let output = dir.join("out");
    run_build_direct(&DirectBuildOptions {
        run: Some(run.clone()),
        channels: vec![2],
        output: Some(output.clone()),
        ..DirectBuildOptions::default()
    })
    .unwrap();

    // Per-channel artifact plus report under the output base.
    let artifact = output.join("ch2.json");
    assert!(artifact.is_file());
    let report: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(output.join("calibrate-report-ch2.json")).unwrap(),
    )
    .unwrap();
    assert!(report["model_id"].as_str().unwrap().ends_with("-ch2"));
    assert_eq!(report["resolved_request"]["source_kind"], "raw");
    assert_eq!(report["adequate"], true);
    assert!(report["training_blocks"].as_u64().unwrap() >= 8);
    let requested = report["requested_intervals"].as_array().unwrap();
    assert_eq!(requested.len(), 4);
    let roles: Vec<&str> = requested
        .iter()
        .map(|interval| interval["role"].as_str().unwrap())
        .collect();
    assert_eq!(
        roles,
        vec!["training", "training", "tuning_validation", "evaluation"]
    );
    assert!(!report["effective_blocks"].as_array().unwrap().is_empty());
    // Trailing partial blocks warn and record instead of truncating silently.
    assert_eq!(
        report["warnings"].as_array().unwrap().len(),
        report["exclusions"].as_array().unwrap().len()
    );
    assert!(
        report["resolved_request"]["reference_provenance"]
            .as_str()
            .unwrap()
            .starts_with("same-run-ch3-fit")
    );
    assert_eq!(
        report["sha256"],
        crate::utils::checksum::file_sha256(&artifact).unwrap()
    );

    // The source run is byte-identical before and after.
    assert_eq!(dir_snapshot(&run), before);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn raw_csv_digest_equivalence() {
    let dir = temp_dir("equivalence");
    let run = dir.join("shot13");
    std::fs::create_dir_all(&run).unwrap();
    write_direct_raw_run(&run);
    let before = dir_snapshot(&run);

    // Direct RAW build first; the TOML mirror reuses its resolved values.
    let direct_out = dir.join("direct");
    run_build_direct(&DirectBuildOptions {
        run: Some(run.clone()),
        channels: vec![2],
        output: Some(direct_out.clone()),
        ..DirectBuildOptions::default()
    })
    .unwrap();
    let direct_report: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(direct_out.join("calibrate-report-ch2.json")).unwrap(),
    )
    .unwrap();

    // Bit-exact CSV export of the same RAW acquisition (Display round-trip).
    let acquisition = run.join("acquisition");
    let csv_path = dir.join("wave.csv");
    crate::utils::waveform::export_raw_waveform_csv(&acquisition, &csv_path).unwrap();

    let intervals = direct_report["requested_intervals"].as_array().unwrap();
    // The TOML mirror reuses the direct fit's reference values AND its
    // recorded build-side uncertainty, so identical samples plus
    // identical parameters yield identical digests (Issue #274 FR-01).
    let u_build = direct_report["tolerance_basis"]["u_build"]
        .as_f64()
        .map(|value| format!("reference_frequency_rel_uncertainty = {value:.17e}\n"))
        .unwrap_or_default();
    let mut request = format!(
        "schema_version = 1\nmodel_id = \"{}\"\nwaveform_csv = \"wave.csv\"\n\
         channel_column = 1\ntime_column = 0\nsample_interval_s = {DIRECT_DT_TEXT}\n\
         reference_frequency_hz = {:.17e}\nreference_phase_rad = {:.17e}\n\
         channel = 2\nseed = 0\nblock_len = 3200\nmin_reference_cycles_per_block = 2.0\n{u_build}\
         output = \"model\"\n\
         [phase_recipe]\nbins = 8\nmin_samples_per_bin = 8\nmin_cycles_per_bin = 2\n\
         min_contributing_blocks = {}\n",
        direct_report["model_id"].as_str().unwrap(),
        direct_report["resolved_request"]["reference_frequency_hz"]
            .as_f64()
            .unwrap(),
        direct_report["resolved_request"]["reference_phase_rad"]
            .as_f64()
            .unwrap(),
        direct_report["training_blocks"].as_u64().unwrap(),
    );
    for interval in intervals {
        request.push_str(&format!(
            "\n[[intervals]]\nrole = \"{}\"\nstart = {}\nend = {}\n",
            interval["role"].as_str().unwrap(),
            interval["start"].as_u64().unwrap(),
            interval["end"].as_u64().unwrap(),
        ));
    }
    let request_path = dir.join("request.toml");
    std::fs::write(&request_path, request).unwrap();
    run_build(&request_path, None).unwrap();

    // Identical samples plus identical parameters yield identical digests.
    assert_eq!(
        std::fs::read(direct_out.join("ch2.json")).unwrap(),
        std::fs::read(dir.join("model/calibration.json")).unwrap()
    );
    assert_eq!(dir_snapshot(&run), before);
    std::fs::remove_dir_all(&dir).unwrap();
}

fn build_request_with_block_len(
    waveform: &str,
    output: &str,
    block_len: usize,
    training_ranges: &[(u64, u64)],
    tuning_start: u64,
    total: u64,
) -> String {
    let mut text = format!(
        "schema_version = 1\nmodel_id = \"r1-test-model\"\nwaveform_csv = \"{waveform}\"\n\
         channel_column = 1\nsample_interval_s = 0.00001\nreference_frequency_hz = 1000.0\n\
         reference_phase_rad = 0.0\nchannel = 3\nseed = 7\nblock_len = {block_len}\noutput = \"{output}\"\n"
    );
    for (start, end) in training_ranges {
        text.push_str(&format!(
            "\n[[intervals]]\nrole = \"training\"\nstart = {start}\nend = {end}\n"
        ));
    }
    text.push_str(&format!(
        "\n[[intervals]]\nrole = \"tuning_validation\"\nstart = {tuning_start}\nend = {total}\n"
    ));
    text
}

#[test]
fn trailing_exclusion_warns_and_records_requested_and_effective() {
    let dir = temp_dir("truncation");
    // Three training intervals at block_len 12800: each leaves a trailing
    // exclusion (1600, 1600, and 9600 samples) while keeping 8 training
    // blocks at the planning minimum; the tuning interval stays
    // block-aligned so only the three training trailers record.
    let waveform = dir.join("wave.csv");
    synthetic_waveform_csv(&waveform, 10, 12_800);
    let request_path = dir.join("request.toml");
    std::fs::write(
        &request_path,
        build_request_with_block_len(
            "wave.csv",
            "model",
            12_800,
            &[(0, 40_000), (40_000, 80_000), (80_000, 115_200)],
            115_200,
            128_000,
        ),
    )
    .unwrap();
    run_build(&request_path, None).unwrap();
    let report: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("model/calibrate-report.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["requested_intervals"].as_array().unwrap().len(), 4);
    assert_eq!(report["training_blocks"], 8);
    let exclusions = report["exclusions"].as_array().unwrap();
    assert_eq!(exclusions.len(), 3);
    assert_eq!(
        report["warnings"].as_array().unwrap().len(),
        exclusions.len()
    );
    assert!(!report["effective_blocks"].as_array().unwrap().is_empty());
    assert_eq!(
        report["resolved_request"]["reference_provenance"],
        "toml-request"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}
