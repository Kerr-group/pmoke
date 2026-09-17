use crate::commands::noise::diagnose::run_diagnose;
use crate::commands::noise::plan::{GuardSpec, resolve_role_plan};
use pmoke_analysis_core::calibration::{BlockPlanRequest, CalibrationRole, RoleInterval};
use std::fs;
use std::path::PathBuf;

fn unique_test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmoke_noise_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Synthetic recorded CSV: 4 blocks of a noisy 1 kHz tone at dt=1e-5 with a
/// time column and detector ch3. Deterministic xorshift, no fixtures.
fn synthetic_csv(path: &std::path::Path, blocks: usize, block_len: usize) {
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
    fs::write(path, text).unwrap();
}

fn diagnose_request_text(
    path: &str,
    output: &str,
    training: &[(u64, u64)],
    evaluation: Option<(u64, u64)>,
    extra: &str,
) -> String {
    let mut text = format!(
        r#"schema_version = 1
operation = "diagnose"
reference_frequency_hz = 1000.0
reference_phase_rad = 0.0
sample_interval_s = 0.00001
block_len = 12800
output = "{output}"

[source]
kind = "recorded_csv"
path = "{path}"

[source.channels]
detector = 3

[source.grid]
stride = 100

[study]
classification = "exploratory"
"#
    );
    for (start, end) in training {
        text.push_str(&format!(
            "\n[[roles]]\nrole = \"training\"\nstart = {start}\nend = {end}\n"
        ));
    }
    if let Some((start, end)) = evaluation {
        text.push_str(&format!(
            "\n[[roles]]\nrole = \"evaluation\"\nstart = {start}\nend = {end}\n"
        ));
    }
    text.push_str(extra);
    text
}

#[test]
fn leakage_guard_rejects_halo_overlap_but_keeps_distant_blocks() {
    // block_len 100 over [0,300) training + [300,600) evaluation:
    // evaluation block [300,400) touches the training footprint expanded by
    // the halo (2) and is ineligible; [400,500) and [500,600) stay eligible.
    let request = BlockPlanRequest {
        intervals: vec![
            RoleInterval {
                role: CalibrationRole::Training,
                start: 0,
                end: 300,
            },
            RoleInterval {
                role: CalibrationRole::Evaluation,
                start: 300,
                end: 600,
            },
        ],
        block_len: 100,
        total_samples: 600,
        reference_frequency_hz: 1000.0,
        sample_interval_s: 1.0e-5,
        ..BlockPlanRequest::default()
    };
    // Loosen the shared kernel minimums for this unit-scale geometry.
    let request = BlockPlanRequest {
        min_training_blocks: 1,
        min_training_intervals: 1,
        min_reference_cycles_per_block: 0.0,
        ..request
    };
    let guard = GuardSpec {
        guard_samples: 0,
        interpolation_halo_samples: 2,
    };
    let resolved = resolve_role_plan(&request, guard, 1.0e-5).unwrap();
    assert_eq!(resolved.role_plan.training_block_count, 3);
    assert_eq!(resolved.role_plan.evaluation_block_count, 3);
    assert_eq!(resolved.leakage.ineligible_evaluation_blocks.len(), 1);
    assert_eq!(
        (
            resolved.leakage.ineligible_evaluation_blocks[0].start,
            resolved.leakage.ineligible_evaluation_blocks[0].end
        ),
        (300, 400)
    );
    assert_eq!(resolved.leakage.eligible_evaluation_blocks.len(), 2);
}

#[test]
fn overlapping_role_intervals_are_rejected() {
    let request = BlockPlanRequest {
        intervals: vec![
            RoleInterval {
                role: CalibrationRole::Training,
                start: 0,
                end: 200,
            },
            RoleInterval {
                role: CalibrationRole::Evaluation,
                start: 100,
                end: 300,
            },
        ],
        block_len: 100,
        total_samples: 300,
        reference_frequency_hz: 1000.0,
        sample_interval_s: 1.0e-5,
        min_training_blocks: 1,
        min_training_intervals: 1,
        min_reference_cycles_per_block: 0.0,
    };
    let guard = GuardSpec {
        guard_samples: 0,
        interpolation_halo_samples: 2,
    };
    assert!(resolve_role_plan(&request, guard, 1.0e-5).is_err());
}

#[test]
fn diagnose_end_to_end_publishes_new_destination() {
    let dir = unique_test_dir("e2e");
    let blocks = 10;
    let block_len = 12_800;
    let wave = dir.join("wave.csv");
    synthetic_csv(&wave, blocks, block_len);
    let total = (blocks * block_len) as u64;
    let mid = (4 * block_len) as u64;
    let training_end = (8 * block_len) as u64;
    let request_path = dir.join("request.toml");
    fs::write(
        &request_path,
        diagnose_request_text(
            "wave.csv",
            "diagnosis",
            &[(0, mid), (mid, training_end)],
            Some((training_end, total)),
            "",
        ),
    )
    .unwrap();
    run_diagnose(&request_path, None).unwrap();

    let destination = dir.join("diagnosis");
    for file in [
        "request.source.toml",
        "request.resolved.json",
        "inputs.json",
        "diagnostics.json",
        "report.md",
    ] {
        assert!(destination.join(file).is_file(), "missing {file}");
    }
    let resolved: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(destination.join("request.resolved.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(resolved["operation"], "diagnose");
    assert_eq!(resolved["study_classification"], "exploratory");
    assert_eq!(resolved["detector_channel"], 3);
    assert_eq!(resolved["stride"], 100);
    let diagnostics: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(destination.join("diagnostics.json")).unwrap())
            .unwrap();
    // M1b computes real measurements: the clean tone must read
    // no_detectable_change (or unknown), with measurements present.
    let finding = diagnostics["diagnostic_finding"].as_str().unwrap();
    assert!(
        finding == "no_detectable_change" || finding == "unknown",
        "clean e2e synthetic must not read supported_structure, got {finding}"
    );
    assert_eq!(diagnostics["measurements_available"], true);
    assert!(destination.join("report.md").is_file(), "missing report.md");

    // No overwrite: a second run into the same destination fails.
    assert!(run_diagnose(&request_path, None).is_err());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn diagnose_rejects_unknown_fields_and_bad_operation() {
    let dir = unique_test_dir("reject");
    let wave = dir.join("wave.csv");
    synthetic_csv(&wave, 10, 12_800);
    let request_path = dir.join("request.toml");
    fs::write(
        &request_path,
        diagnose_request_text(
            "wave.csv",
            "diagnosis",
            &[(0, 102_400)],
            Some((102_400, 128_000)),
            "\nunknown_field = 1\n",
        ),
    )
    .unwrap();
    assert!(run_diagnose(&request_path, None).is_err());

    fs::write(
        &request_path,
        diagnose_request_text(
            "wave.csv",
            "diagnosis",
            &[(0, 102_400)],
            Some((102_400, 128_000)),
            "",
        )
        .replace("operation = \"diagnose\"", "operation = \"compare\""),
    )
    .unwrap();
    assert!(run_diagnose(&request_path, None).is_err());
    fs::remove_dir_all(&dir).unwrap();
}

/// M1b synthetic: phase-modulated 1 kHz tone (structured) vs clean tone
/// (flat). Both deterministic xorshift; no fixtures, no network.
fn synthetic_structured_csv(path: &std::path::Path, blocks: usize, block_len: usize) {
    let dt = 1.0e-5;
    let mut text = String::from("time (s),ch3\n");
    let mut state = 0x9e37_79b9_7f4a_7c15u64;
    let mut rand = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state as f64 / u64::MAX as f64) * 2.0 - 1.0
    };
    for i in 0..blocks * block_len {
        let time = i as f64 * dt;
        // Phase-modulated carrier: residual variance peaks at some phases.
        let carrier = (2.0 * std::f64::consts::PI * 1_000.0 * time + 0.3).sin();
        let modulator = (2.0 * std::f64::consts::PI * 1_000.0 * time).sin();
        let value = carrier + 0.35 * modulator * rand() + 0.02 * rand();
        text.push_str(&format!("{time:.10e},{value:.10e}\n"));
    }
    fs::write(path, text).unwrap();
}

fn flat_request_text(path: &str, output: &str, total: u64) -> String {
    diagnose_request_text(
        path,
        output,
        &[(0, total / 2), (total / 2, total)],
        None,
        "",
    )
}

#[test]
fn diagnostics_report_supported_structure_on_modulated_tone() {
    let dir = unique_test_dir("m1b_structured");
    let blocks = 16usize;
    let block_len = 12800usize;
    let wave = dir.join("wave.csv");
    synthetic_structured_csv(&wave, blocks, block_len);
    let total = (blocks * block_len) as u64;
    let request_path = dir.join("request.toml");
    fs::write(
        &request_path,
        flat_request_text("wave.csv", "diagnosis", total),
    )
    .unwrap();
    let output = dir.join("diagnosis");
    run_diagnose(&request_path, Some(&output)).unwrap();
    let text = fs::read_to_string(output.join("diagnostics.json")).unwrap();
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    let finding = report["diagnostic_finding"].as_str().unwrap();
    assert!(
        finding == "supported_structure" || finding == "unknown",
        "structured synthetic must not read no_detectable_change, got {finding}"
    );
    assert_eq!(report["measurements_available"], true);
    let measurements = &report["measurements"];
    assert!(measurements["phase_variance"]["v0"].as_f64().unwrap() > 0.0);
    assert!(
        measurements["correlation"]["spd_validated"]
            .as_bool()
            .unwrap()
    );
    let md = fs::read_to_string(output.join("report.md")).unwrap();
    assert!(md.contains("finding:"));
    assert!(!md.to_lowercase().contains("caused by"));
}

#[test]
fn diagnostics_report_no_change_on_clean_tone() {
    let dir = unique_test_dir("m1b_clean");
    let blocks = 16usize;
    let block_len = 12800usize;
    let wave = dir.join("wave.csv");
    synthetic_csv(&wave, blocks, block_len);
    let total = (blocks * block_len) as u64;
    let request_path = dir.join("request.toml");
    fs::write(
        &request_path,
        flat_request_text("wave.csv", "diagnosis", total),
    )
    .unwrap();
    let output = dir.join("diagnosis");
    run_diagnose(&request_path, Some(&output)).unwrap();
    let text = fs::read_to_string(output.join("diagnostics.json")).unwrap();
    let report: serde_json::Value = serde_json::from_str(&text).unwrap();
    let finding = report["diagnostic_finding"].as_str().unwrap();
    assert!(
        finding == "no_detectable_change" || finding == "unknown",
        "clean synthetic must not read supported_structure, got {finding}"
    );
    assert_eq!(report["measurements_available"], true);
}

#[test]
fn diagnostics_report_insufficient_on_single_block() {
    // One training block cannot satisfy the shared kernel minimums: the
    // run must report unknown/insufficient, never a pass.
    let dir = unique_test_dir("m1b_thin");
    let wave = dir.join("wave.csv");
    synthetic_csv(&wave, 2, 12800);
    let request_path = dir.join("request.toml");
    fs::write(
        &request_path,
        diagnose_request_text("wave.csv", "diagnosis", &[(0, 12800)], None, ""),
    )
    .unwrap();
    let output = dir.join("diagnosis");
    let result = run_diagnose(&request_path, Some(&output));
    if let Ok(()) = result {
        let text = fs::read_to_string(output.join("diagnostics.json")).unwrap();
        let report: serde_json::Value = serde_json::from_str(&text).unwrap();
        let finding = report["diagnostic_finding"].as_str().unwrap();
        assert!(
            finding == "unknown" || finding == "insufficient_evidence",
            "thin coverage must stay non-pass, got {finding}"
        );
    }
}

/// PN-AT-001 rail control on the recorded RAW route: saturating ADC rail
/// codes are reported in the acquisition QC and the source bytes stay
/// untouched (no clipping, no repair).
#[test]
fn diagnose_reports_rail_saturation_on_recorded_raw() {
    let dir = unique_test_dir("rails_raw");
    let source_dir = dir.join("rawsrc");
    fs::create_dir_all(&source_dir).unwrap();
    let blocks = 10_usize;
    let block_len = 12_800_usize;
    let total = blocks * block_len;
    let mut bytes = Vec::with_capacity(total * 2);
    for index in 0..total {
        let word: u16 = match index {
            100 => 0,
            200 => u16::MAX,
            12_900 => 0,
            _ => {
                let time = index as f64 * 1.0e-5;
                let value = 8_000.0 * (2.0 * std::f64::consts::PI * 1_000.0 * time + 0.3).sin();
                (32_768.0 + value).round().clamp(1.0, 65_534.0) as u16
            }
        };
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    fs::write(source_dir.join("ch3.u16le"), &bytes).unwrap();
    fs::write(
        source_dir.join("manifest.toml"),
        format!(
            r#"
version = 1

[oscilloscope]
waveform_format = "WORD"
byte_order = "little-endian"

[channels.ch3]
file = "ch3.u16le"
sample_count = {total}
x_increment = 0.00001
x_origin = 0.0
x_reference = 0.0
y_increment = 0.001
y_origin = 32768.0
y_reference = 0.0
"#
        ),
    )
    .unwrap();
    let frozen_before = fs::read(source_dir.join("ch3.u16le")).unwrap();
    let request_path = dir.join("request.toml");
    fs::write(
        &request_path,
        r#"schema_version = 1
operation = "diagnose"
reference_frequency_hz = 1000.0
reference_phase_rad = 0.0
sample_interval_s = 0.00001
block_len = 12800
output = "diagnosis"

[source]
kind = "recorded_raw"
path = "rawsrc"

[source.channels]
detector = 3

[source.grid]
stride = 100

[study]
classification = "exploratory"

[[roles]]
role = "training"
start = 0
end = 51200

[[roles]]
role = "training"
start = 51200
end = 102400

[[roles]]
role = "evaluation"
start = 102400
end = 128000
"#,
    )
    .unwrap();
    run_diagnose(&request_path, None).unwrap();

    let report: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("diagnosis/diagnostics.json")).unwrap())
            .unwrap();
    assert_eq!(report["measurements_available"], true);
    let qc = &report["measurements"]["acquisition_qc"];
    assert_eq!(
        qc["rail_saturation"], true,
        "rail saturation must be reported"
    );
    assert_eq!(qc["detector_rail_low"], 2);
    assert_eq!(qc["detector_rail_high"], 1);
    assert_eq!(qc["reference_rail_low"], 0);
    assert_eq!(qc["reference_rail_high"], 0);
    let reason = report["finding_reason"].as_str().unwrap();
    assert!(
        reason.contains("rail saturation reported"),
        "rail evidence must be named in the finding reason: {reason}"
    );
    // The recorded source bytes are preserved exactly: no clipping or
    // repair of the saturated samples.
    assert_eq!(
        fs::read(source_dir.join("ch3.u16le")).unwrap(),
        frozen_before
    );
    fs::remove_dir_all(&dir).unwrap();
}
