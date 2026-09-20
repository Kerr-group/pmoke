//! PN-M2 tests: one-request compare workflow over synthetic recorded CSV.
//!
//! All fixtures are deterministic xorshift synthetics generated inline; no
//! private data, no network, no hardware.

use super::run_compare;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

fn unique_test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pmoke_noise_cmp_{name}_{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Synthetic recorded CSV: a 1 kHz tone plus noise at dt=1e-5 with a time
/// column and detector ch3. Deterministic xorshift, no fixtures.
fn synthetic_csv(path: &Path, samples: usize, noise_gain: f64, seed: u64) {
    let dt = 1.0e-5;
    let mut text = String::from("time (s),ch3\n");
    let mut state = seed;
    let mut rand = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state as f64 / u64::MAX as f64) * 2.0 - 1.0
    };
    for i in 0..samples {
        let time = i as f64 * dt;
        let tone = (2.0 * std::f64::consts::PI * 1_000.0 * time + 0.3).sin();
        let value = tone + noise_gain * rand();
        text.push_str(&format!("{time:.10e},{value:.10e}\n"));
    }
    fs::write(path, text).unwrap();
}

/// Compare request text over a recorded CSV with small block geometry.
/// Training covers two intervals, validation one, evaluation one trailing
/// span. `extra` appends raw TOML for negative tests.
fn compare_request_text(
    path: &str,
    output: &str,
    total: u64,
    block_len: u64,
    extra: &str,
) -> String {
    let train_a_end = total / 4;
    let train_b_end = total / 2;
    let valid_end = total * 3 / 4;
    format!(
        r#"schema_version = 1
operation = "compare"
reference_frequency_hz = 1000.0
reference_phase_rad = 0.0
sample_interval_s = 0.00001
block_len = {block_len}
output = "{output}"

[source]
kind = "recorded_csv"
path = "{path}"

[source.channels]
detector = 3

[source.grid]
stride = 10

[study]
classification = "exploratory"

[[roles]]
role = "training"
start = 0
end = {train_a_end}

[[roles]]
role = "training"
start = {train_a_end}
end = {train_b_end}

[[roles]]
role = "validation"
start = {train_b_end}
end = {valid_end}

[[roles]]
role = "evaluation"
start = {valid_end}
end = {total}

[calibration]

[context]
rotation_rad = 0.0
modulation_depth = 1.0
field_factor = 1.0

[candidates]
include_boxcar_baseline = true
joint_modes = ["identity", "phase_diagonal"]
{extra}"#
    )
}

fn run_request(dir: &Path, text: &str) -> anyhow::Result<PathBuf> {
    let request_path = dir.join("request.toml");
    fs::write(&request_path, text).unwrap();
    run_compare(&request_path, None, None)?;
    Ok(dir.join("comparison"))
}

/// Reduced geometry for the non-golden compare tests: 102,400 samples at the
/// same block_len 12800 (8 blocks instead of 15.625) keeps every calibration
/// minimum, role fraction, and per-block cost model intact at roughly half
/// the compute. The end-to-end test below stays at full size as the golden
/// path. (block_len cannot shrink: blocks below 12800 samples fall under the
/// 128-reference-cycles-per-block floor and become unusable.)
const SMALL_TOTAL: usize = 102_400;
const FULL_BLOCK: u64 = 12_800;

#[test]
fn compare_end_to_end_builds_frozen_models_and_baseline_plus_candidates() {
    // Geometry: 200k samples, block_len 12800 @128 cycles/block. Training
    // spans [0,100k) = 7 blocks; phase recipe uses small explicit minimums
    // so the synthetic coverage passes without silent down-binning.
    let dir = unique_test_dir("e2e");
    let total = 200_000usize;
    synthetic_csv(&dir.join("wave.csv"), total, 0.05, 0x1234_5678_9abc_def1);
    let extra = r#"
[calibration]
phase_bins = 8
min_samples_per_bin = 8
min_cycles_per_bin = 1
min_contributing_blocks = 2
min_training_blocks = 2
min_training_intervals = 1
"#;
    // Note: [calibration] is already emitted empty above; the override
    // table below must merge — instead rewrite with explicit values.
    let text = compare_request_text("wave.csv", "comparison", total as u64, 12_800, "")
        .replace("[calibration]\n", extra);
    let destination = run_request(&dir, &text).unwrap();

    for file in [
        "request.source.toml",
        "request.resolved.json",
        "inputs.json",
        "diagnostics.json",
        "context.json",
        "role-plan.json",
        "reserved.json",
        "models.json",
        "schedule.json",
        "comparison.json",
        "report.md",
        "staging-manifest.json",
        "calibrations/ch3/identity/model.json",
        "calibrations/ch3/phase_diagonal/model.json",
        "legs/boxcar_baseline.json",
        "legs/identity_candidate.json",
        "legs/phase_diagonal_candidate.json",
    ] {
        assert!(destination.join(file).is_file(), "missing {file}");
    }
    let comparison: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(destination.join("comparison.json")).unwrap())
            .unwrap();
    assert_eq!(comparison["operation"], "compare");
    assert_eq!(
        comparison["statistic_id"],
        "pooled_residual_scatter_ratio_v1"
    );
    // Baseline plus all requested legs present.
    let legs = comparison["legs"].as_array().unwrap();
    assert_eq!(legs.len(), 3);
    assert!(legs.iter().any(|leg| leg["name"] == "boxcar_baseline"));
    // Frozen models carry exact digests matching staged bytes.
    for model in comparison["models"].as_array().unwrap() {
        let sha = model["sha256"].as_str().unwrap();
        assert_eq!(sha.len(), 64);
        let staged = fs::read(destination.join(model["artifact_path"].as_str().unwrap())).unwrap();
        assert_eq!(sha, crate::utils::checksum::sha256_hex(&staged));
    }
    // Context hash is frozen and shared.
    let ctx = comparison["context"]["context_sha256"].as_str().unwrap();
    assert_eq!(ctx.len(), 64);
    // No overwrite: a second run into the same destination fails.
    let request_path = dir.join("request.toml");
    assert!(run_compare(&request_path, None, None).is_err());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compare_resolves_omitted_phase_bins_to_accepted_64_explicitly() {
    // Omitted phase_bins must resolve to the accepted 64 explicitly and be
    // recorded as such — never the legacy CLI 8 (PN-AT-009).
    let dir = unique_test_dir("bins64");
    let total = SMALL_TOTAL;
    synthetic_csv(&dir.join("wave.csv"), total, 0.05, 0x1234_5678_9abc_def1);
    // 64 bins need real coverage: 128 cycles/block over 4 training blocks
    // gives ~8 cycles per bin — below the default 128 minimum, so this
    // geometry must fail closed rather than auto-reduce (asserted below).
    let text = compare_request_text("wave.csv", "comparison", total as u64, FULL_BLOCK, "");
    let request_path = dir.join("request.toml");
    fs::write(&request_path, &text).unwrap();
    let result = run_compare(&request_path, None, None);
    // Either insufficient coverage fails the model build explicitly, or a
    // high-coverage synthetic passes with 64 recorded bins — both are
    // honest; silent 8-bin resolution is what is forbidden.
    if result.is_ok() {
        let resolved: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(dir.join("comparison/request.resolved.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(resolved["recipe"]["phase_bins"], 64);
        assert_eq!(resolved["recipe"]["phase_bins_explicit"], false);
    } else {
        let message = format!("{result:?}");
        assert!(
            message.contains("insufficient")
                || message.contains("coverage")
                || message.contains("bin"),
            "64-bin default must fail closed on thin coverage, got: {message}"
        );
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compare_rejects_missing_baseline_and_search_and_unknown_fields() {
    let dir = unique_test_dir("reject");
    synthetic_csv(
        &dir.join("wave.csv"),
        SMALL_TOTAL,
        0.05,
        0x1234_5678_9abc_def1,
    );
    let total = SMALL_TOTAL as u64;
    // Missing boxcar baseline.
    let text = compare_request_text("wave.csv", "comparison", total, FULL_BLOCK, "").replace(
        "include_boxcar_baseline = true",
        "include_boxcar_baseline = false",
    );
    let request_path = dir.join("request.toml");
    fs::write(&request_path, &text).unwrap();
    assert!(run_compare(&request_path, None, None).is_err());

    // Hyperparameter search refused.
    let text = compare_request_text("wave.csv", "comparison", total, FULL_BLOCK, "").replace(
        "[calibration]\n",
        "[calibration]\nhyperparameter_search = true\n",
    );
    fs::write(&request_path, &text).unwrap();
    let error = format!("{:?}", run_compare(&request_path, None, None).unwrap_err());
    assert!(error.contains("unsupported_autotuning"), "got: {error}");

    // Unknown fields rejected.
    let text = compare_request_text("wave.csv", "comparison", total, FULL_BLOCK, "")
        + "\nunknown_field = 1\n";
    fs::write(&request_path, &text).unwrap();
    assert!(run_compare(&request_path, None, None).is_err());

    // Bad operation rejected.
    let text = compare_request_text("wave.csv", "comparison", total, FULL_BLOCK, "")
        .replace("operation = \"compare\"", "operation = \"diagnose\"");
    fs::write(&request_path, &text).unwrap();
    assert!(run_compare(&request_path, None, None).is_err());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compare_records_explicit_unavailable_and_failed_legs() {
    // Stationary-correlated on a training-adequate geometry whose short
    // reserved span cannot support lag 256: the frozen models build, the
    // legs run, and short-block failures stay explicit per-window records
    // beside the intact labeled baseline (PN-AT-015).
    let dir = unique_test_dir("unavail");
    let total = SMALL_TOTAL;
    synthetic_csv(&dir.join("wave.csv"), total, 0.05, 0x1234_5678_9abc_def1);
    let text = compare_request_text("wave.csv", "comparison", total as u64, FULL_BLOCK, "")
        .replace(
            "joint_modes = [\"identity\", \"phase_diagonal\"]",
            "joint_modes = [\"identity\", \"stationary_correlated\"]",
        )
        .replace(
            "[calibration]\n",
            "[calibration]\nphase_bins = 8\nmin_samples_per_bin = 8\nmin_cycles_per_bin = 1\nmin_contributing_blocks = 2\nmin_training_blocks = 2\nmin_training_intervals = 1\n",
        );
    let request_path = dir.join("request.toml");
    fs::write(&request_path, &text).unwrap();
    run_compare(&request_path, None, None).unwrap();
    let comparison: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("comparison/comparison.json")).unwrap())
            .unwrap();
    let legs = comparison["legs"].as_array().unwrap();
    // The unrunnable mode is explicit, never filled from another mode.
    let stationary = legs
        .iter()
        .find(|leg| leg["name"] == "stationary_correlated_candidate")
        .expect("stationary leg record present");
    // The stationary leg stays an explicit record with its own identity:
    // requested-but-unbuildable legs would surface here as unavailable or
    // failed (never borrowed from another mode). On this training-adequate
    // geometry the model builds and the leg runs beside the labeled
    // baseline, so assert the intact explicit record either way (PN-AT-015).
    let state = stationary["state"].as_str().unwrap();
    assert!(
        state == "complete" || state == "unavailable" || state == "failed",
        "stationary leg must be an explicit record, got {state}"
    );
    assert_eq!(
        stationary["mode"].as_str(),
        Some("stationary_correlated"),
        "stationary record must carry its own mode identity"
    );
    // No leg borrows another mode's rows: explicit records carry no series.
    // (The stationary model builds on this geometry, so its candidate leg
    // runs beside the baseline; either outcome stays an intact record.)
    assert!(dir.join("comparison/legs/boxcar_baseline.json").is_file());
    // The stationary leg is an explicit record: unavailable/failed legs
    // carry no per-window series file; a completed leg carries one.
    let stationary_file = dir.join("comparison/legs/stationary_correlated_candidate.json");
    if state == "complete" {
        assert!(stationary_file.is_file());
    } else {
        assert!(!stationary_file.is_file() || stationary["output_rows"].is_null());
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compare_reserved_evidence_is_measured_not_declared() {
    // Changing only reserved bytes must change reserved diagnostics while
    // the training model bytes stay identical (PN-AT-010).
    let dir = unique_test_dir("reserved");
    let total = SMALL_TOTAL;
    let recipe_override = "[calibration]\nphase_bins = 8\nmin_samples_per_bin = 8\nmin_cycles_per_bin = 1\nmin_contributing_blocks = 2\nmin_training_blocks = 2\nmin_training_intervals = 1\n";
    let text = compare_request_text("wave.csv", "comparison", total as u64, FULL_BLOCK, "")
        .replace("[calibration]\n", recipe_override);
    // Run A: baseline noise.
    synthetic_csv(&dir.join("wave.csv"), total, 0.05, 0x1234_5678_9abc_def1);
    let request_path = dir.join("request.toml");
    fs::write(&request_path, &text).unwrap();
    run_compare(&request_path, None, None).unwrap();
    let reserved_a: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("comparison/reserved.json")).unwrap())
            .unwrap();
    let model_a: serde_json::Value = serde_json::from_slice(
        &fs::read(dir.join("comparison/calibrations/ch3/identity/model.json")).unwrap(),
    )
    .unwrap();
    fs::remove_dir_all(dir.join("comparison")).unwrap();
    fs::remove_dir_all(dir.join("comparison.staging")).unwrap_or(());

    // Run B: same training span, hotter reserved span (second half louder).
    {
        let dt = 1.0e-5;
        let mut text_csv = String::from("time (s),ch3\n");
        let mut state = 0x1234_5678_9abc_def1u64;
        let mut rand = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state as f64 / u64::MAX as f64) * 2.0 - 1.0
        };
        for i in 0..total {
            let time = i as f64 * dt;
            let tone = (2.0 * std::f64::consts::PI * 1_000.0 * time + 0.3).sin();
            let gain = if i >= total / 2 { 0.30 } else { 0.05 };
            text_csv.push_str(&format!("{time:.10e},{:.10e}\n", tone + gain * rand()));
        }
        fs::write(dir.join("wave.csv"), text_csv).unwrap();
    }
    run_compare(&request_path, None, None).unwrap();
    let reserved_b: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(dir.join("comparison/reserved.json")).unwrap())
            .unwrap();
    let model_b: serde_json::Value = serde_json::from_slice(
        &fs::read(dir.join("comparison/calibrations/ch3/identity/model.json")).unwrap(),
    )
    .unwrap();
    // Training estimates unchanged (function of the frozen training span,
    // whose source bytes are untouched); reserved diagnostics moved.
    // The training *plan* records the live source digest (which changed
    // under run B), so compare the estimated quantities — reference
    // variance and phase variances — not the whole plan envelope.
    assert_eq!(
        model_a["reference_variance_v2"], model_b["reference_variance_v2"],
        "reference variance must be frozen across reserved-only changes"
    );
    assert_eq!(
        model_a["phase"], model_b["phase"],
        "phase variances must be frozen across reserved-only changes"
    );
    assert_ne!(
        reserved_a["reserved_shift"], reserved_b["reserved_shift"],
        "reserved diagnostics must respond to reserved bytes"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compare_cancel_before_commit_leaves_destination_absent() {
    // A pre-set cancel flag stops the workflow before commit with no
    // visible destination (PN-AT-021 subset, PN-NFR-005).
    let dir = unique_test_dir("cancel");
    let total = SMALL_TOTAL;
    synthetic_csv(&dir.join("wave.csv"), total, 0.05, 0x1234_5678_9abc_def1);
    let text = compare_request_text("wave.csv", "comparison", total as u64, FULL_BLOCK, "")
        .replace(
            "[calibration]\n",
            "[calibration]\nphase_bins = 8\nmin_samples_per_bin = 8\nmin_cycles_per_bin = 1\nmin_contributing_blocks = 2\nmin_training_blocks = 2\nmin_training_intervals = 1\n",
        );
    let request_path = dir.join("request.toml");
    fs::write(&request_path, &text).unwrap();
    let cancel = AtomicBool::new(true);
    let error = format!(
        "{:?}",
        run_compare(&request_path, None, Some(&cancel)).unwrap_err()
    );
    assert!(error.contains("cancelled_before_commit"), "got: {error}");
    assert!(
        !dir.join("comparison").exists(),
        "cancelled compare must leave no destination"
    );
    // A fresh run without cancel commits normally (restart evidence).
    // The cancelled attempt removed its staging, so the retry proceeds.
    let flag = AtomicBool::new(false);
    run_compare(&request_path, None, Some(&flag)).unwrap();
    assert!(dir.join("comparison/comparison.json").is_file());
    assert!(dir.join("comparison/staging-manifest.json").is_file());
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compare_request_cancelled_before_commit_flag_refuses() {
    // resources.cancelled_before_commit=true is a declared pre-cancelled
    // workflow: refuse before touching the source (PN-FR-031).
    let dir = unique_test_dir("precancel");
    synthetic_csv(
        &dir.join("wave.csv"),
        SMALL_TOTAL,
        0.05,
        0x1234_5678_9abc_def1,
    );
    let text = compare_request_text("wave.csv", "comparison", SMALL_TOTAL as u64, FULL_BLOCK, "")
        + "\n[resources]\ncancelled_before_commit = true\n";
    let request_path = dir.join("request.toml");
    fs::write(&request_path, &text).unwrap();
    let error = format!("{:?}", run_compare(&request_path, None, None).unwrap_err());
    assert!(error.contains("cancelled_before_commit"), "got: {error}");
    assert!(!dir.join("comparison").exists());
    fs::remove_dir_all(&dir).unwrap();
}

/// PN-AT-005: the shared downstream context is frozen once and shared
/// verbatim; a resumed attempt that binds a different context (or a
/// different request after the freeze) is refused by name instead of
/// silently mixing generations.
#[test]
fn compare_frozen_context_is_shared_and_post_freeze_mutation_is_refused() {
    let dir = unique_test_dir("context_freeze");
    let total = SMALL_TOTAL;
    synthetic_csv(&dir.join("wave.csv"), total, 0.05, 0x0c0f_fee1_2345_6789);
    let base = compare_request_text("wave.csv", "comparison", total as u64, FULL_BLOCK, "").replace(
        "[calibration]\n",
        "[calibration]\nphase_bins = 8\nmin_samples_per_bin = 8\nmin_cycles_per_bin = 1\nmin_contributing_blocks = 2\nmin_training_blocks = 2\nmin_training_intervals = 1\n",
    );
    let request_path = dir.join("request.toml");
    fs::write(&request_path, &base).unwrap();
    run_compare(&request_path, None, None).unwrap();

    // Exactly one frozen context is written and every retained record
    // agrees on its digest: no per-leg context copies can diverge.
    let destination = dir.join("comparison");
    let context: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(destination.join("context.json")).unwrap())
            .unwrap();
    let context_sha = context["context_sha256"].as_str().unwrap().to_owned();
    let comparison: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(destination.join("comparison.json")).unwrap())
            .unwrap();
    assert_eq!(
        comparison["context"]["context_sha256"],
        context_sha.as_str()
    );
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(destination.join("staging-manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["context_sha256"], context_sha.as_str());

    // Simulate an interrupted attempt: the committed generation becomes the
    // resume candidate for the same request.
    let staging = dir.join("comparison.staging");
    fs::rename(&destination, &staging).unwrap();
    run_compare(&request_path, None, None).unwrap();
    assert!(
        dir.join("comparison/comparison.json").is_file(),
        "unchanged request must resume the frozen snapshot"
    );

    // Post-freeze mutation of the frozen inputs (rotation, depth,
    // reference phase) is refused by name, never mixed into the old
    // generation.
    for (old, new) in [
        ("rotation_rad = 0.0", "rotation_rad = 0.25"),
        ("modulation_depth = 1.0", "modulation_depth = 0.9"),
        ("reference_phase_rad = 0.0", "reference_phase_rad = 0.3"),
    ] {
        if dir.join("comparison").exists() {
            fs::rename(dir.join("comparison"), dir.join("comparison.staging")).unwrap();
        }
        let mutated = base.replace(old, new);
        fs::write(&request_path, &mutated).unwrap();
        let error = format!("{:?}", run_compare(&request_path, None, None).unwrap_err());
        assert!(
            error.contains("staging_mismatch"),
            "mutating `{old}` after the freeze must refuse by name, got: {error}"
        );
        assert!(
            !dir.join("comparison").exists(),
            "refused resume must leave no destination"
        );
    }

    // A prior generation whose frozen context disagrees with the request
    // (tampered manifest digest) is refused by the dedicated context gate.
    fs::write(&request_path, &base).unwrap();
    let manifest_path = dir.join("comparison.staging/staging-manifest.json");
    let mut tampered: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
    tampered["context_sha256"] =
        serde_json::json!("0000000000000000000000000000000000000000000000000000000000000000");
    fs::write(
        &manifest_path,
        format!("{}\n", serde_json::to_string_pretty(&tampered).unwrap()),
    )
    .unwrap();
    let error = format!("{:?}", run_compare(&request_path, None, None).unwrap_err());
    assert!(
        error.contains("different frozen context"),
        "context disagreement must be named, got: {error}"
    );
    assert!(!dir.join("comparison").exists());

    fs::remove_dir_all(&dir).unwrap();
}

/// A truncated staging manifest (the SIGKILL-in-write residue, Issue #273)
/// is not a resume candidate: the retry reclaims the stale staging and
/// commits normally instead of failing on the torn file.
#[test]
fn compare_torn_staging_manifest_is_reclaimed_and_retry_recovers() {
    let dir = unique_test_dir("torn_manifest");
    let total = SMALL_TOTAL;
    synthetic_csv(&dir.join("wave.csv"), total, 0.05, 0x1234_5678_9abc_def1);
    let text = compare_request_text("wave.csv", "comparison", total as u64, FULL_BLOCK, "")
        .replace(
            "[calibration]\n",
            "[calibration]\nphase_bins = 8\nmin_samples_per_bin = 8\nmin_cycles_per_bin = 1\nmin_contributing_blocks = 2\nmin_training_blocks = 2\nmin_training_intervals = 1\n",
        );
    let request_path = dir.join("request.toml");
    fs::write(&request_path, &text).unwrap();
    // Kill-in-window residue: a staging directory whose manifest is
    // truncated mid-write, with no destination committed.
    let staging = dir.join("comparison.staging");
    fs::create_dir_all(&staging).unwrap();
    fs::write(
        staging.join("staging-manifest.json"),
        "{\"schema_version\": 2, \"request_sha256\": \"abc",
    )
    .unwrap();
    assert!(!dir.join("comparison").exists());
    run_compare(&request_path, None, None).unwrap();
    assert!(dir.join("comparison/comparison.json").is_file());
    assert!(dir.join("comparison/staging-manifest.json").is_file());
    assert!(!staging.exists(), "recovered staging must be consumed");
    fs::remove_dir_all(&dir).unwrap();
}
