//! PN-M3 bank tests (Issue #246): frozen schedule, identical-model
//! reduction, boundary diagnostics, retained closure, worker/chunk
//! determinism and staged replay over synthetic recorded CSV.
//!
//! All fixtures are deterministic synthetics generated inline; no private
//! data, no network, no hardware.

use crate::commands::noise::compare::run_compare;
use crate::commands::noise::replay::run_replay;
use std::fs;
use std::path::{Path, PathBuf};

fn unique_test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pmoke_noise_bank_{name}_{}_{}",
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

/// Synthetic recorded CSV with two noise levels: the first half is quiet,
/// the second half louder, so regime-specific estimations differ.
fn synthetic_csv(path: &Path, samples: usize, seed: u64) {
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
        let gain = if i < samples / 2 { 0.03 } else { 0.25 };
        text.push_str(&format!("{time:.10e},{:.10e}\n", tone + gain * rand()));
    }
    fs::write(path, text).unwrap();
}

/// Compare request with an appended [bank] section. `bank` is empty for the
/// frozen-global lane.
fn request_text(
    csv: &str,
    output: &str,
    total: u64,
    bank_section: &str,
    covariance: &str,
    extra_calibration: &str,
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
modulation_depth = 0.92
field_factor = 1.0

[candidates]
include_boxcar_baseline = true
joint_modes = ["identity"]
covariance_output = "{covariance}"

{bank_section}"#
    )
}

const SMALL_CALIBRATION: &str = "phase_bins = 8\nmin_samples_per_bin = 8\nmin_cycles_per_bin = 1\nmin_contributing_blocks = 2\nmin_training_blocks = 2\nmin_training_intervals = 1\n";

fn run_request(dir: &Path, text: &str, name: &str) -> anyhow::Result<PathBuf> {
    let request_path = dir.join(format!("request_{name}.toml"));
    fs::write(&request_path, text).unwrap();
    run_compare(&request_path, None, None)?;
    Ok(dir.join(format!("comparison_{name}")))
}

/// Bank with two regimes trained on the same intervals (identical estimated
/// content) and a split schedule covering the evaluation centers.
fn identical_bank(total: u64) -> String {
    let split = total - total / 8;
    format!(
        r#"[bank]
schema_version = 1
selection_provenance = "external_schedule"
workers = 1
chunk_size = 8

[[bank.regimes]]
name = "pre"
condition = "before_split"
intervals = [{{ start = 0, end = {} }}, {{ start = {}, end = {} }}]

[[bank.regimes]]
name = "post"
condition = "after_split"
intervals = [{{ start = 0, end = {} }}, {{ start = {}, end = {} }}]

[[bank.schedule]]
regime = "pre"
start = 0
end = {split}

[[bank.schedule]]
regime = "post"
start = {split}
end = {total}
"#,
        total / 4,
        total / 4,
        total / 2,
        total / 4,
        total / 4,
        total / 2,
    )
}

/// Bank whose regimes train on disjoint (different-noise) halves, so the
/// estimated models differ.
fn distinct_bank(total: u64, workers: usize, chunk_size: usize) -> String {
    let split = total - total / 8;
    format!(
        r#"[bank]
schema_version = 1
selection_provenance = "declared_training_context"
workers = {workers}
chunk_size = {chunk_size}

[[bank.regimes]]
name = "quiet"
condition = "quiet_half"
intervals = [{{ start = 0, end = {} }}]

[[bank.regimes]]
name = "loud"
condition = "loud_half"
intervals = [{{ start = {}, end = {} }}]

[[bank.schedule]]
regime = "quiet"
start = 0
end = {split}

[[bank.schedule]]
regime = "loud"
start = {split}
end = {total}
"#,
        total / 2,
        total / 2,
        total * 3 / 4,
    )
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

fn leg_values(destination: &Path, leg: &str) -> Vec<f64> {
    read_json(&destination.join(format!("legs/{leg}.json")))["fundamental_magnitude"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_f64().unwrap())
        .collect()
}

fn leg_centers(destination: &Path, leg: &str) -> Vec<u64> {
    read_json(&destination.join(format!("legs/{leg}.json")))["centers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_u64().unwrap())
        .collect()
}

// PN-AT-013/021: identical regime models reduce to one distinct model and
// the bank reproduces the frozen global path exactly.
#[test]
fn bank_identical_models_reduce_and_reproduce_global_path() {
    let dir = unique_test_dir("identical");
    let total = 200_000u64;
    synthetic_csv(&dir.join("wave.csv"), total as usize, 0x1234_5678_9abc_def1);
    let global = run_request(
        &dir,
        &request_text(
            "wave.csv",
            "comparison_global",
            total,
            "",
            "full",
            SMALL_CALIBRATION,
        ),
        "global",
    )
    .unwrap();
    let bank = run_request(
        &dir,
        &request_text(
            "wave.csv",
            "comparison_bank",
            total,
            &identical_bank(total),
            "full",
            SMALL_CALIBRATION,
        ),
        "bank",
    )
    .unwrap();

    for file in [
        "model-bank.json",
        "bank-evidence.json",
        "calibrations/ch3/identity/bank-pre/model.json",
    ] {
        assert!(bank.join(file).is_file(), "missing bank artifact {file}");
    }
    let model_bank = read_json(&bank.join("model-bank.json"));
    assert_eq!(model_bank["schema_version"], 1);
    assert_eq!(model_bank["selection_provenance"], "external_schedule");
    assert_eq!(model_bank["reduction"]["identity"], "identical_model");
    assert_eq!(model_bank["models"].as_array().unwrap().len(), 1);

    let comparison = read_json(&bank.join("comparison.json"));
    let bank_report = &comparison["bank"];
    assert_eq!(bank_report["reduction"]["identity"], "identical_model");
    assert_eq!(bank_report["distinct_models"]["identity"], 1);
    assert_eq!(bank_report["row_mapping_ok"], true);

    // The identical-model bank reproduces the frozen global path exactly.
    assert_eq!(
        leg_values(&global, "identity_candidate"),
        leg_values(&bank, "identity_candidate"),
        "identical-model bank must equal the frozen global values"
    );
    assert_eq!(
        leg_centers(&global, "identity_candidate"),
        leg_centers(&bank, "identity_candidate")
    );
    // Every schedule center appears once with one model ID per row.
    let schedule = read_json(&bank.join("schedule.json"));
    let rows = schedule["bank"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), leg_centers(&bank, "identity_candidate").len());
    let leg = read_json(&bank.join("legs/identity_candidate.json"));
    let leg_rows = leg["rows"].as_array().unwrap();
    assert_eq!(leg_rows.len(), rows.len());
    let first_model = leg_rows[0]["model_id"].as_str().unwrap().to_string();
    for (schedule_row, leg_row) in rows.iter().zip(leg_rows.iter()) {
        assert_eq!(schedule_row["center"], leg_row["center"]);
        assert_eq!(schedule_row["regime"], leg_row["regime"]);
        assert_eq!(schedule_row["boundary"], leg_row["boundary"]);
        assert_eq!(leg_row["model_id"].as_str().unwrap(), first_model);
        // Retained XY is present and finite for replay/attribution.
        assert_eq!(leg_row["xy"].as_array().unwrap().len(), 12);
    }
    // Boundary windows exist and are flagged (cross-regime support).
    let boundary = &leg["boundary"];
    assert!(boundary["boundary_windows"].as_u64().unwrap() > 0);
    assert!(boundary["cross_regime_windows"].as_u64().unwrap() > 0);
    fs::remove_dir_all(&dir).unwrap();
}

// PN-AT-013/014: distinct regime models run per center on the frozen
// schedule with boundary flags and exact row mapping.
#[test]
fn bank_distinct_models_map_every_center_once() {
    let dir = unique_test_dir("distinct");
    let total = 240_000u64;
    synthetic_csv(&dir.join("wave.csv"), total as usize, 0xfeed_face_dead_beef);
    let bank = run_request(
        &dir,
        &request_text(
            "wave.csv",
            "comparison_bank",
            total,
            &distinct_bank(total, 1, 64),
            "diagonal",
            SMALL_CALIBRATION,
        ),
        "bank",
    )
    .unwrap();
    let comparison = read_json(&bank.join("comparison.json"));
    assert_eq!(
        comparison["bank"]["reduction"]["identity"],
        "distinct_models"
    );
    assert_eq!(comparison["bank"]["distinct_models"]["identity"], 2);
    let leg = read_json(&bank.join("legs/identity_candidate.json"));
    let rows = leg["rows"].as_array().unwrap();
    let schedule = read_json(&bank.join("schedule.json"));
    let schedule_rows = schedule["bank"]["rows"].as_array().unwrap();
    assert_eq!(rows.len(), schedule_rows.len());
    let mut previous: Option<u64> = None;
    let mut regimes: Vec<&str> = Vec::new();
    for (leg_row, schedule_row) in rows.iter().zip(schedule_rows.iter()) {
        let center = leg_row["center"].as_u64().unwrap();
        assert_eq!(center, schedule_row["center"].as_u64().unwrap());
        if let Some(previous) = previous {
            assert!(center > previous, "centers must be strictly increasing");
        }
        previous = Some(center);
        let regime = leg_row["regime"].as_str().unwrap();
        if regimes.last() != Some(&regime) {
            regimes.push(regime);
        }
    }
    assert_eq!(regimes, vec!["quiet", "loud"]);
    // Distinct models appear; each center carries exactly one model ID.
    let quiet_model = rows.iter().find(|row| row["regime"] == "quiet").unwrap()["model_id"]
        .as_str()
        .unwrap()
        .to_string();
    let loud_model = rows.iter().find(|row| row["regime"] == "loud").unwrap()["model_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(quiet_model, loud_model);
    // Boundary windows are flagged and counted in the report.
    assert!(comparison["bank"]["boundary_windows"].as_u64().unwrap() > 0);
    // Diagonally serialized covariance retains 12 entries per row.
    assert!(rows.iter().all(|row| {
        row["covariance"]
            .as_array()
            .map(|values| values.len() == 12)
            .unwrap_or(false)
    }));
    fs::remove_dir_all(&dir).unwrap();
}

// PN-FR-037: worker/chunk settings preserve row order and exact values.
#[test]
fn bank_workers_and_chunks_preserve_rows_and_values() {
    let dir = unique_test_dir("workers");
    let total = 240_000u64;
    synthetic_csv(&dir.join("wave.csv"), total as usize, 0x0bad_c0de_1234_5678);
    let single = run_request(
        &dir,
        &request_text(
            "wave.csv",
            "comparison_single",
            total,
            &distinct_bank(total, 1, 3),
            "full",
            SMALL_CALIBRATION,
        ),
        "single",
    )
    .unwrap();
    let parallel = run_request(
        &dir,
        &request_text(
            "wave.csv",
            "comparison_parallel",
            total,
            &distinct_bank(total, 4, 7),
            "full",
            SMALL_CALIBRATION,
        ),
        "parallel",
    )
    .unwrap();
    for leg in ["boxcar_baseline", "identity_candidate"] {
        assert_eq!(
            leg_centers(&single, leg),
            leg_centers(&parallel, leg),
            "{leg} centers must not depend on workers/chunks"
        );
        assert_eq!(
            leg_values(&single, leg),
            leg_values(&parallel, leg),
            "{leg} values must not depend on workers/chunks"
        );
    }
    let single_rows = read_json(&single.join("legs/identity_candidate.json"))["rows"].clone();
    let parallel_rows = read_json(&parallel.join("legs/identity_candidate.json"))["rows"].clone();
    assert_eq!(single_rows, parallel_rows, "row records must be identical");
    // Measured acceleration evidence is retained and bounded.
    let evidence = read_json(&single.join("bank-evidence.json"));
    assert_eq!(evidence["workers"], 1);
    assert_eq!(evidence["chunk_size"], 3);
    let acceleration = &evidence["acceleration"];
    assert!(acceleration["prepared_plans"].as_u64().unwrap() >= 1);
    assert!(acceleration["direct_reference_rows"].as_u64().unwrap() >= 1);
    // PN-NFR-004: the conditional-bank apply is measured against the
    // equivalent fixed-global cost on the same grid, with the cache state
    // and the reuse count reported.
    let reuse = acceleration["plan_reuse_windows"].as_u64().unwrap();
    assert!(reuse >= acceleration["prepared_plans"].as_u64().unwrap());
    let projection = acceleration["fixed_global_projection_ms_total"]
        .as_f64()
        .unwrap();
    assert!(projection.is_finite() && projection > 0.0);
    let ratio = acceleration["accelerated_vs_projection_ratio"]
        .as_f64()
        .unwrap();
    assert!(ratio.is_finite() && ratio > 0.0);
    assert!(
        acceleration["cache_state"]
            .as_str()
            .unwrap()
            .contains("cold prepare")
    );
    let leg_evidence = &evidence["legs"][0];
    assert!(leg_evidence["direct_reference_ms"].as_f64().unwrap() >= 0.0);
    assert!(
        leg_evidence["fixed_global_projection_ms"]
            .as_f64()
            .unwrap()
            .is_finite()
    );
    let max_error = acceleration["max_abs_error_v"].as_f64().unwrap();
    assert!(
        max_error <= 1.0e-12,
        "accelerated error {max_error} above bound"
    );
    assert!(
        evidence["retained_rows_cap"].as_u64().unwrap()
            >= evidence["retained_rows"].as_u64().unwrap()
    );
    fs::remove_dir_all(&dir).unwrap();
}

// PN-AT-016/024: replay verifies the moved generation and replays
// phase -> MOKE -> NPY from the retained artifacts alone; tampering and
// overwrite attempts fail closed.
#[test]
fn bank_replay_after_move_verifies_digests_and_replays() {
    let dir = unique_test_dir("replay");
    let total = 200_000u64;
    synthetic_csv(&dir.join("wave.csv"), total as usize, 0x5eed_5eed_5eed_5eed);
    let destination = run_request(
        &dir,
        &request_text(
            "wave.csv",
            "comparison_bank",
            total,
            &identical_bank(total),
            "full",
            SMALL_CALIBRATION,
        ),
        "bank",
    )
    .unwrap();
    // Move the destination: retained artifacts are authoritative, no
    // external model path or cwd is required.
    let moved = dir.join("moved_generation");
    fs::rename(&destination, &moved).unwrap();
    let replay_target = dir.join("replay_out");
    run_replay(&moved, Some(&replay_target), false).unwrap();
    for file in [
        "phase/identity_candidate.csv",
        "moke/identity_candidate.csv",
        "npy/identity_candidate_angle_variance.npy",
        "replay-manifest.json",
    ] {
        assert!(replay_target.join(file).is_file(), "missing replay {file}");
    }
    let replay_manifest = read_json(&replay_target.join("replay-manifest.json"));
    let leg_entry = &replay_manifest["legs"][0];
    let schedule_rows = read_json(&moved.join("schedule.json"))["bank"]["rows"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(leg_entry["rows"].as_u64().unwrap() as usize, schedule_rows);
    // Full covariance policy yields conditional MOKE variance derived from
    // the retained (X1, X2) marginal: cross-check the replayed value against
    // the core delta-method contract on the same row.
    let moke = fs::read_to_string(replay_target.join("moke/identity_candidate.csv")).unwrap();
    assert!(moke.lines().next().unwrap().contains("angle_variance_v2"));
    assert!(
        moke.lines()
            .nth(1)
            .unwrap()
            .contains("conditional_full_marginal")
    );
    let first_row = &read_json(&moved.join("legs/identity_candidate.json"))["rows"][0];
    let covariance: Vec<f64> = first_row["covariance"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_f64().unwrap())
        .collect();
    let xy: Vec<f64> = first_row["xy"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_f64().unwrap())
        .collect();
    let expected_variance = pmoke_analysis_core::moke_uncertainty::moke_standard_angle_variance(
        xy[0],
        xy[2],
        0.92,
        covariance[0],
        covariance[2],
        covariance[23],
    )
    .unwrap();
    let replayed_variance: f64 = moke
        .lines()
        .nth(1)
        .unwrap()
        .split(',')
        .nth(3)
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        (replayed_variance - expected_variance).abs() <= 1.0e-15 * expected_variance.abs(),
        "replayed MOKE variance {replayed_variance} disagrees with the retained marginal {expected_variance}"
    );
    // Verification-only passes without writing, and refuses overwrite.
    run_replay(&moved, Some(&replay_target), true).unwrap();
    assert!(run_replay(&moved, Some(&replay_target), false).is_err());
    // Tampered retained bytes fail the digest closure.
    let model_path = moved.join("calibrations/ch3/identity/bank-pre/model.json");
    let mut bytes = fs::read(&model_path).unwrap();
    bytes[10] ^= 0x01;
    fs::write(&model_path, bytes).unwrap();
    let error = format!(
        "{:?}",
        run_replay(&moved, Some(&dir.join("replay_bad")), true).unwrap_err()
    );
    assert!(
        error.contains("staging_mismatch") || error.contains("digest mismatch"),
        "tampered model must fail the closure check: {error}"
    );
    fs::remove_dir_all(&dir).unwrap();
}

// A `none` covariance policy replays without inventing variance (never
// zero) and reports the reason explicitly (PN-FR-025).
#[test]
fn bank_replay_none_policy_reports_unavailable_variance() {
    let dir = unique_test_dir("none_policy");
    let total = 200_000u64;
    synthetic_csv(&dir.join("wave.csv"), total as usize, 0x1111_2222_3333_4444);
    let destination = run_request(
        &dir,
        &request_text(
            "wave.csv",
            "comparison_bank",
            total,
            &identical_bank(total),
            "none",
            SMALL_CALIBRATION,
        ),
        "bank",
    )
    .unwrap();
    let leg = read_json(&destination.join("legs/identity_candidate.json"));
    assert!(leg["rows"][0].get("covariance").is_none());
    let replay_target = dir.join("replay_out");
    run_replay(&destination, Some(&replay_target), false).unwrap();
    let moke = fs::read_to_string(replay_target.join("moke/identity_candidate.csv")).unwrap();
    let header = moke.lines().next().unwrap();
    assert!(!header.contains("angle_variance_v2"));
    assert!(
        moke.lines()
            .nth(1)
            .unwrap()
            .contains("unavailable_covariance_output_none")
    );
    assert!(
        !replay_target
            .join("npy/identity_candidate_angle_variance.npy")
            .exists()
    );
    fs::remove_dir_all(&dir).unwrap();
}

// PN-AT-013: invalid bank requests fail closed before inference.
#[test]
fn bank_rejects_gaps_overlaps_unknown_regimes_and_channel_mismatch() {
    let dir = unique_test_dir("reject");
    let total = 200_000u64;
    synthetic_csv(&dir.join("wave.csv"), total as usize, 0xaaaa_bbbb_cccc_dddd);
    let cases: Vec<(&str, String, &str)> = vec![
        (
            "gap",
            r#"[bank]
schema_version = 1
selection_provenance = "external_schedule"

[[bank.regimes]]
name = "pre"
condition = "a"
intervals = [{ start = 0, end = 50000 }]

[[bank.schedule]]
regime = "pre"
start = 0
end = 160000
"#
            .to_string(),
            "unsupported_regime",
        ),
        (
            "overlap",
            r#"[bank]
schema_version = 1
selection_provenance = "external_schedule"

[[bank.regimes]]
name = "pre"
condition = "a"
intervals = [{ start = 0, end = 50000 }]

[[bank.regimes]]
name = "post"
condition = "b"
intervals = [{ start = 50000, end = 100000 }]

[[bank.schedule]]
regime = "pre"
start = 0
end = 150000

[[bank.schedule]]
regime = "post"
start = 100000
end = 200000
"#
            .to_string(),
            "schedule_overlap",
        ),
        (
            "unknown_regime",
            r#"[bank]
schema_version = 1
selection_provenance = "external_schedule"

[[bank.regimes]]
name = "pre"
condition = "a"
intervals = [{ start = 0, end = 50000 }]

[[bank.schedule]]
regime = "missing"
start = 0
end = 200000
"#
            .to_string(),
            "unknown_model_reference",
        ),
        (
            "channel_mismatch",
            r#"[bank]
schema_version = 1
selection_provenance = "external_schedule"

[[bank.regimes]]
name = "pre"
condition = "a"
channel = 4
intervals = [{ start = 0, end = 50000 }]

[[bank.schedule]]
regime = "pre"
start = 0
end = 200000
"#
            .to_string(),
            "channel_mismatch",
        ),
        (
            "overflow",
            r#"[bank]
schema_version = 1
selection_provenance = "external_schedule"

[[bank.regimes]]
name = "pre"
condition = "a"
intervals = [{ start = 0, end = 18446744073709551615 }]

[[bank.schedule]]
regime = "pre"
start = 0
end = 200000
"#
            .to_string(),
            "exceeds recorded length",
        ),
    ];
    for (name, section, expected) in cases {
        let text = request_text(
            "wave.csv",
            &format!("comparison_{name}"),
            total,
            &section,
            "diagonal",
            SMALL_CALIBRATION,
        );
        let request_path = dir.join(format!("request_{name}.toml"));
        fs::write(&request_path, &text).unwrap();
        let error = format!("{:?}", run_compare(&request_path, None, None).unwrap_err());
        assert!(
            error.contains(expected),
            "case {name}: expected '{expected}' in: {error}"
        );
        assert!(
            !dir.join(format!("comparison_{name}")).exists(),
            "case {name} must not publish a destination"
        );
    }
    fs::remove_dir_all(&dir).unwrap();
}

// PN-FR-023 addendum: a stale unreferenced staging file is refused at
// commit instead of being published.
#[test]
fn bank_refuses_unreferenced_staging_files() {
    let dir = unique_test_dir("stale");
    let total = 200_000u64;
    synthetic_csv(&dir.join("wave.csv"), total as usize, 0x9999_8888_7777_6666);
    let text = request_text(
        "wave.csv",
        "comparison_bank",
        total,
        &identical_bank(total),
        "full",
        SMALL_CALIBRATION,
    );
    let request_path = dir.join("request.toml");
    fs::write(&request_path, &text).unwrap();
    // Craft a stale generation: a manifest owning its files plus a stray leg
    // file left behind by an earlier attempt.
    let staging = dir.join("comparison_bank.staging");
    fs::create_dir_all(staging.join("legs")).unwrap();
    let context_bytes = b"{}\n";
    fs::write(staging.join("context.json"), context_bytes).unwrap();
    fs::write(staging.join("legs/stale_leg.json"), "{}\n").unwrap();
    let manifest = serde_json::json!({
        "schema_version": 2,
        "request_sha256": crate::utils::checksum::sha256_hex(text.as_bytes()),
        "source_digest": "stub",
        "context_sha256": "stub",
        "model_digests": {},
        "phase": "staged",
        "completed_phases": [],
        "file_digests": {
            "context.json": crate::utils::checksum::sha256_hex(context_bytes),
        },
    });
    fs::write(
        staging.join("staging-manifest.json"),
        format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
    )
    .unwrap();
    let error = format!("{:?}", run_compare(&request_path, None, None).unwrap_err());
    assert!(
        error.contains("unreferenced"),
        "stale staging must fail closed on the unreferenced file: {error}"
    );
    assert!(!dir.join("comparison_bank/comparison.json").exists());
    fs::remove_dir_all(&dir).unwrap();
}
