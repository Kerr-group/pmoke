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
    assert_eq!(diagnostics["diagnostic_finding"], "unknown");
    assert_eq!(diagnostics["measurements_available"], false);

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
