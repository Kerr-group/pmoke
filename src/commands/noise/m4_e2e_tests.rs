//! PN-M4 end-to-end acceptance tests (PN-AT-018/019/020/022, PN-NFR-006).
//!
//! Deterministic inline synthetic fixtures only: no private data, no
//! network, no hardware.

use super::run_compare;
use std::fs;
use std::path::{Path, PathBuf};

fn unique_test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pmoke_noise_m4_{name}_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn xorshift(state: &mut u64) -> f64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    (*state as f64 / u64::MAX as f64) * 2.0 - 1.0
}

/// Stationary synthetic recorded CSV: 1 kHz tone plus noise at dt=1e-5.
fn synthetic_csv(path: &Path, samples: usize, seed: u64) {
    let dt = 1.0e-5;
    let mut text = String::from("time (s),ch3\n");
    let mut state = seed;
    for i in 0..samples {
        let time = i as f64 * dt;
        let tone = (2.0 * std::f64::consts::PI * 1_000.0 * time + 0.3).sin();
        text.push_str(&format!(
            "{time:.10e},{:.10e}\n",
            tone + 0.05 * xorshift(&mut state)
        ));
    }
    fs::write(path, text).unwrap();
}

/// Synthetic recorded CSV with a known amplitude envelope: steady 1.0 before
/// `ramp_start`, a linear rise to 1.5 over `ramp_len` samples, steady after.
fn amplitude_rise_csv_at(
    path: &Path,
    samples: usize,
    seed: u64,
    ramp_start: usize,
    ramp_len: usize,
) {
    let dt = 1.0e-5;
    let mut text = String::from("time (s),ch3\n");
    let mut state = seed;
    for i in 0..samples {
        let time = i as f64 * dt;
        let amplitude = if i < ramp_start {
            1.0
        } else if i < ramp_start + ramp_len {
            1.0 + 0.5 * (i - ramp_start) as f64 / ramp_len as f64
        } else {
            1.5
        };
        let tone = amplitude * (2.0 * std::f64::consts::PI * 1_000.0 * time + 0.3).sin();
        text.push_str(&format!(
            "{time:.10e},{:.10e}\n",
            tone + 0.01 * xorshift(&mut state)
        ));
    }
    fs::write(path, text).unwrap();
}

const SMALL_CALIBRATION: &str = "phase_bins = 8\nmin_samples_per_bin = 8\nmin_cycles_per_bin = 1\nmin_contributing_blocks = 2\nmin_training_blocks = 2\nmin_training_intervals = 1\n";

/// Reduced geometry: 102,400 samples at the same block_len 12800 (8 blocks
/// instead of 15.625) keeps every calibration minimum, role fraction, and
/// per-block cost model intact at roughly half the compute.
const SMALL_TOTAL: usize = 102_400;
const FULL_BLOCK: u64 = 12_800;

/// Compare request over a recorded CSV with one evaluation region.
fn request_text(
    csv: &str,
    output: &str,
    total: u64,
    block_len: u64,
    extra_calibration: &str,
    statistics: &str,
    bank: &str,
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
path = "{csv}"

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
{extra_calibration}

[context]
rotation_rad = 0.0
modulation_depth = 1.0
field_factor = 1.0

[candidates]
include_boxcar_baseline = true
joint_modes = ["identity", "phase_diagonal"]
{statistics}
{bank}"#
    )
}

fn run_request(dir: &Path, text: &str, name: &str) -> anyhow::Result<PathBuf> {
    let request_path = dir.join(format!("request_{name}.toml"));
    fs::write(&request_path, text).unwrap();
    run_compare(&request_path, None, None)?;
    Ok(dir.join(format!("comparison_{name}")))
}

fn comparison_json(destination: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(destination.join("comparison.json")).unwrap()).unwrap()
}

#[test]
fn compare_reports_separate_gates_fidelity_and_complete_accounting() {
    // PN-AT-018/022: every requested method x channel x region is enumerated,
    // and computed / benefit / adequacy / fidelity / scientific /
    // default-promotion are reported as separate gates.
    let dir = unique_test_dir("gates");
    let total = SMALL_TOTAL;
    synthetic_csv(&dir.join("wave.csv"), total, 0x1234_5678_9abc_def1);
    let text = request_text(
        "wave.csv",
        "comparison_gates",
        total as u64,
        FULL_BLOCK,
        SMALL_CALIBRATION,
        "",
        "",
    );
    let destination = run_request(&dir, &text, "gates").unwrap();
    let report = comparison_json(&destination);

    assert_eq!(report["accounting_complete"], true);
    let accounting = report["accounting"].as_array().unwrap();
    // Baseline plus two requested modes, one row per eligible evaluation
    // region for complete legs.
    let mut regions: Vec<&str> = accounting
        .iter()
        .map(|row| row["region"].as_str().unwrap())
        .collect();
    regions.sort_unstable();
    regions.dedup();
    assert!(!regions.is_empty());
    assert_eq!(accounting.len(), 3 * regions.len());
    for row in accounting {
        assert_eq!(row["channel"], 3);
        let outcome = row["outcome"].as_str().unwrap();
        assert!(
            outcome == "computed" || outcome == "baseline_anchor",
            "complete legs must be accounted per region, got {outcome}"
        );
        assert!(row["evidence_recorded"].as_bool().unwrap());
        assert!(
            row["region"]
                .as_str()
                .unwrap()
                .starts_with("eligible_evaluation_")
        );
    }
    let gates = &report["gates"];
    assert!(
        gates["acquisition_validity"] == "pass" || gates["acquisition_validity"] == "unverified",
        "unexpected acquisition state {}",
        gates["acquisition_validity"]
    );
    assert_eq!(gates["computation"], "complete");
    assert_eq!(gates["diagnostic_qualification"], "qualified");
    assert_eq!(gates["model_adequacy"], "adequate");
    assert!(
        gates["numeric_benefit"] == "fail"
            || gates["numeric_benefit"] == "inconclusive"
            || gates["numeric_benefit"] == "pass",
        "unexpected benefit state {}",
        gates["numeric_benefit"]
    );
    if gates["numeric_benefit"] == "pass" {
        // A numeric pass must not become a scientific pass while the fidelity
        // and applicability controls stay unverified (PN-FR-032, PN-AT-022).
        assert_eq!(gates["dynamic_fidelity"], "unverified");
        assert_ne!(gates["scientific"], "pass");
    }
    assert_eq!(gates["dynamic_fidelity"], "unverified");
    assert_ne!(gates["scientific"], "pass");
    assert_eq!(gates["default_promotion"], "not_authorized");
    let fidelity = &report["fidelity"];
    assert_eq!(fidelity["status"], "unverified");
    assert!(fidelity["tolerance_authority"].is_null());
    assert!(fidelity["reason"].as_str().unwrap().contains("PN-D-007"));
    let statistics = &report["statistics"];
    assert_eq!(
        statistics["primary_statistic_id"],
        "pooled_residual_scatter_ratio_v1"
    );
    assert_eq!(
        statistics["secondary_statistic_id"],
        "mean_block_sd_ratio_v1"
    );
    assert_eq!(
        statistics["evaluate_lockin_statistic_id"],
        "mean_block_sd_ratio_v1"
    );
    assert_eq!(
        statistics["resampling_unit"],
        "whole_disjoint_evaluation_blocks"
    );
    assert_eq!(statistics["rng_id"], "xorshift64/v1");
    assert_eq!(statistics["selection_performed"], false);
    assert_eq!(statistics["candidates_requested"], 2);
    // Every evidence record carries its statistic identity, the secondary
    // statistic and the stability gate.
    let evidence = report["evidence"].as_array().unwrap();
    assert!(!evidence.is_empty());
    for item in evidence {
        assert_eq!(item["statistic_id"], "pooled_residual_scatter_ratio_v1");
        assert_eq!(item["secondary_statistic_id"], "mean_block_sd_ratio_v1");
        assert!(item["stability_gate"].is_string());
        assert!(item["detrend_ratio_none"].is_number());
        assert!(item["detrend_ratio_block_mean"].is_number());
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compare_replays_frozen_inputs_to_identical_reports() {
    // PN-NFR-006: the same frozen inputs and recorded RNG reproduce the
    // report byte for byte.
    let dir = unique_test_dir("repro");
    let total = SMALL_TOTAL;
    synthetic_csv(&dir.join("wave.csv"), total, 0x1234_5678_9abc_def1);
    let text = request_text(
        "wave.csv",
        "comparison_repro",
        total as u64,
        FULL_BLOCK,
        SMALL_CALIBRATION,
        "",
        "",
    );
    let destination = run_request(&dir, &text, "repro").unwrap();
    let first = fs::read(destination.join("comparison.json")).unwrap();
    fs::remove_dir_all(&destination).unwrap();
    fs::remove_dir_all(dir.join("comparison_repro.staging")).unwrap_or(());
    let destination = run_request(&dir, &text, "repro").unwrap();
    let second = fs::read(destination.join("comparison.json")).unwrap();
    assert_eq!(
        first, second,
        "the frozen-input comparison report must be byte reproducible"
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compare_insufficient_evidence_is_inconclusive_not_a_pass() {
    // PN-AT-022: an insufficient-evidence input produces a complete
    // computation whose benefit state is explicitly inconclusive.
    let dir = unique_test_dir("insufficient");
    let total = SMALL_TOTAL;
    synthetic_csv(&dir.join("wave.csv"), total, 0x1234_5678_9abc_def1);
    let statistics = "[statistics]\nmin_independent_blocks = 99\n";
    let text = request_text(
        "wave.csv",
        "comparison_insufficient",
        total as u64,
        FULL_BLOCK,
        SMALL_CALIBRATION,
        statistics,
        "",
    );
    let destination = run_request(&dir, &text, "insufficient").unwrap();
    let report = comparison_json(&destination);
    assert_eq!(report["gates"]["computation"], "complete");
    assert_eq!(report["gates"]["numeric_benefit"], "inconclusive");
    assert_ne!(report["gates"]["scientific"], "pass");
    assert_eq!(report["gates"]["default_promotion"], "not_authorized");
    let evidence = report["evidence"].as_array().unwrap();
    assert!(evidence.iter().all(|item| item["paired_ci"].is_null()));
    assert!(
        evidence
            .iter()
            .all(|item| item["benefit_gate"] == "inconclusive")
    );
    assert!(evidence.iter().any(|item| {
        item["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note.as_str().unwrap().contains("independent blocks"))
    }));
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compare_does_not_mutate_sources_or_local_defaults() {
    // PN-AT-022: a completed (no-benefit) run must not touch the recorded
    // source, the request or any local default.
    let dir = unique_test_dir("mutation");
    let total = SMALL_TOTAL;
    let csv = dir.join("wave.csv");
    synthetic_csv(&csv, total, 0x1234_5678_9abc_def1);
    let text = request_text(
        "wave.csv",
        "comparison_mutation",
        total as u64,
        FULL_BLOCK,
        SMALL_CALIBRATION,
        "",
        "",
    );
    let request_path = dir.join("request_mutation.toml");
    fs::write(&request_path, &text).unwrap();
    let before: Vec<(String, u64, Vec<u8>)> = vec![
        (
            "wave.csv".to_string(),
            fs::metadata(&csv).unwrap().len(),
            fs::read(&csv).unwrap(),
        ),
        (
            "request_mutation.toml".to_string(),
            fs::metadata(&request_path).unwrap().len(),
            fs::read(&request_path).unwrap(),
        ),
    ];
    run_compare(&request_path, None, None).unwrap();
    assert_eq!(fs::read(&csv).unwrap(), before[0].2);
    assert_eq!(fs::read(&request_path).unwrap(), before[1].2);
    // The destination is the only new entry; staging is cleaned.
    let mut entries: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .collect();
    entries.sort();
    assert_eq!(
        entries,
        vec!["comparison_mutation", "request_mutation.toml", "wave.csv"]
    );
    let report = comparison_json(&dir.join("comparison_mutation"));
    assert!(
        !report["gates"]["default_promotion"]
            .as_str()
            .unwrap()
            .is_empty()
    );
    assert_eq!(report["gates"]["default_promotion"], "not_authorized");
    fs::remove_dir_all(&dir).unwrap();
}

/// Request whose evaluation region contains an injected rise at `ramp_start`.
fn rise_request_text(csv: &str, output: &str, total: u64, bank: &str) -> String {
    format!(
        r#"schema_version = 1
operation = "compare"
reference_frequency_hz = 1000.0
reference_phase_rad = 0.0
sample_interval_s = 0.00001
block_len = 12800
output = "{output}"

[source]
kind = "recorded_csv"
path = "{csv}"

[source.channels]
detector = 3

[source.grid]
stride = 10

[study]
classification = "exploratory"

[[roles]]
role = "training"
start = 0
end = 40000

[[roles]]
role = "training"
start = 40000
end = 60000

[[roles]]
role = "validation"
start = 60000
end = 70000

[[roles]]
role = "evaluation"
start = 70000
end = {total}

[calibration]
{SMALL_CALIBRATION}
[context]
rotation_rad = 0.0
modulation_depth = 1.0
field_factor = 1.0

[candidates]
include_boxcar_baseline = true
joint_modes = ["identity"]
{bank}"#
    )
}

/// The injected envelope evaluated at an output center (the boxcar leg
/// reports the fundamental peak amplitude).
fn truth_at(center: u64, ramp_start: usize, ramp_len: usize) -> f64 {
    let index = center as usize;
    if index < ramp_start {
        1.0
    } else if index < ramp_start + ramp_len {
        1.0 + 0.5 * (index - ramp_start) as f64 / ramp_len as f64
    } else {
        1.5
    }
}

#[test]
fn known_amplitude_rise_is_recovered_at_output_resolution() {
    // PN-AT-020: an injected rise is measured against known truth — gain,
    // latency, overshoot and final-value/field errors at the actual output
    // resolution — while the fidelity gate stays unverified (PN-D-007).
    let dir = unique_test_dir("fidelity");
    // Full-size geometry: the three-window fidelity analysis needs ~10
    // evaluation centers, so this test keeps the 200k fixture (see the
    // reduced-geometry note on SMALL_TOTAL).
    let total = 200_000usize;
    let (ramp_start, ramp_len) = (100_000usize, 2_000usize);
    amplitude_rise_csv_at(
        &dir.join("wave.csv"),
        total,
        0x0f1d_2e3d_4c5b_6a79,
        ramp_start,
        ramp_len,
    );
    let text = rise_request_text("wave.csv", "comparison_fidelity", total as u64, "");
    let destination = run_request(&dir, &text, "fidelity").unwrap();
    let leg: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(destination.join("legs/boxcar_baseline.json")).unwrap(),
    )
    .unwrap();
    let centers: Vec<u64> = leg["centers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_u64().unwrap())
        .collect();
    let recovered: Vec<(u64, f64)> = centers
        .iter()
        .zip(leg["fundamental_magnitude"].as_array().unwrap())
        .map(|(center, value)| (*center, value.as_f64().unwrap()))
        .collect();
    let truth: Vec<(u64, f64)> = centers
        .iter()
        .map(|center| (*center, truth_at(*center, ramp_start, ramp_len)))
        .collect();
    let report = comparison_json(&destination);
    let field_factor = report["context"]["field_factor"].as_f64().unwrap();
    let metrics = crate::commands::noise::fidelity::assess(
        &truth,
        &recovered,
        (centers[0], ramp_start as u64 - 1_000),
        (
            ramp_start as u64 + ramp_len as u64 + 10_000,
            centers[centers.len() - 1] + 1,
        ),
        1.0e-5,
        field_factor,
    )
    .unwrap();
    let gain = metrics.gain.unwrap();
    assert!((gain - 1.0).abs() < 0.05, "recovered gain {gain}");
    let latency = metrics.latency_centers.unwrap();
    assert!(
        latency.abs() <= 300.0,
        "latency must stay within about three reference cycles at output resolution, got {latency} samples"
    );
    assert!(metrics.final_value_error.unwrap().abs() < 0.05);
    assert!(metrics.overshoot_fraction.unwrap().abs() < 0.2);
    // Fidelity is unverified because no physical tolerance is specified, and
    // the scientific verdict cannot pass on numeric evidence alone.
    assert_eq!(report["fidelity"]["status"], "unverified");
    assert!(report["fidelity"]["tolerance_authority"].is_null());
    assert_ne!(report["gates"]["scientific"], "pass");
    assert_eq!(report["gates"]["default_promotion"], "not_authorized");
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn bank_boundary_rise_keeps_every_row_and_reports_seam_evidence() {
    // PN-AT-020: a rise crossing a bank regime boundary is kept (no row
    // dropping, no implicit crossfade) and the measured seam evidence is
    // reported as unverified physical fidelity.
    let dir = unique_test_dir("bankrise");
    // Full-size geometry: the seam statistics need several evaluation
    // centers on each side of the boundary (see SMALL_TOTAL note).
    let total = 200_000usize;
    let split = total - total / 8; // the bank schedule boundary at 175k
    amplitude_rise_csv_at(
        &dir.join("wave.csv"),
        total,
        0x5a5a_1234_9876_0f0f,
        split - 2_000,
        4_000,
    );
    let bank = format!(
        r#"[bank]
schema_version = 1
selection_provenance = "declared_training_context"
workers = 1
chunk_size = 8

[[bank.regimes]]
name = "pre"
condition = "before_boundary"
intervals = [{{ start = 0, end = 50000 }}]

[[bank.regimes]]
name = "post"
condition = "after_boundary"
intervals = [{{ start = 50000, end = 100000 }}]

[[bank.schedule]]
regime = "pre"
start = 0
end = {split}

[[bank.schedule]]
regime = "post"
start = {split}
end = {total}
"#
    );
    let text = request_text(
        "wave.csv",
        "comparison_bankrise",
        total as u64,
        FULL_BLOCK,
        SMALL_CALIBRATION,
        "",
        &bank,
    )
    .replace(
        "joint_modes = [\"identity\", \"phase_diagonal\"]",
        "joint_modes = [\"identity\"]",
    );
    let destination = run_request(&dir, &text, "bankrise").unwrap();
    let report = comparison_json(&destination);
    assert_eq!(report["gates"]["computation"], "complete");
    let fidelity = &report["fidelity"];
    assert_eq!(fidelity["status"], "unverified");
    let boundary_windows = fidelity["boundary_windows"].as_u64().unwrap();
    assert!(
        boundary_windows > 0,
        "the boundary windows must be flagged, got {boundary_windows}"
    );
    assert!(fidelity["switch_jump_rms"].is_number());
    let bank_report = &report["bank"];
    assert!(bank_report["cross_regime_windows"].as_u64().unwrap() > 0);
    // Every schedule row keeps its record: the retained series length equals
    // the number of shared output centers.
    let leg: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(destination.join("legs/boxcar_baseline.json")).unwrap(),
    )
    .unwrap();
    let centers = leg["centers"].as_array().unwrap().len();
    let magnitudes = leg["fundamental_magnitude"].as_array().unwrap().len();
    assert_eq!(centers, magnitudes);
    assert!(centers > 0);
    // The boundary windows sit around the injected rise, so the seam evidence
    // is measured where the physical feature is (not dropped or averaged away).
    let report_rows = report["bank"]["boundary_windows"].as_u64().unwrap();
    assert_eq!(report_rows, boundary_windows);
    fs::remove_dir_all(&dir).unwrap();
}
