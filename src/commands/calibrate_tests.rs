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
