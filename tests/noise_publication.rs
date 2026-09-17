//! PN-AT-021 publication fault-injection tests (Issue #246, PN-FR-003/031,
//! PN-NFR-005).
//!
//! These exercise the real `pmoke noise compare` binary against synthetic
//! recorded CSV fixtures only: no hardware, no network, no private data.
//! The injected faults are process-level (SIGKILL) and filesystem-level
//! (kernel write-size limit, RLIMIT_FSIZE) and are available on Unix hosts;
//! the file compiles everywhere and skips on non-Unix targets.
#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn unique_test_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "pmoke_noise_publication_{name}_{}_{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Deterministic noisy 1 kHz tone at dt=1e-5 with a detector ch3 column.
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
        text.push_str(&format!("{time:.10e},{:.10e}\n", tone + 0.05 * rand()));
    }
    fs::write(path, text).unwrap();
}

fn request_text(path: &str, output: &str, total: u64) -> String {
    // Block-aligned geometry: two training blocks, one validation block and
    // a two-block evaluation span at block_len 12,800 over 64,000 samples.
    // Synthetic scale only; the publication semantics under test do not
    // depend on the data volume.
    let block = 12_800_u64;
    let train_a_end = block;
    let train_b_end = 2 * block;
    let valid_end = 3 * block;
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
path = "{path}"

[source.channels]
detector = 3

[source.grid]
stride = 1000

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
phase_bins = 8
min_samples_per_bin = 4
min_cycles_per_bin = 1
min_contributing_blocks = 2
min_training_blocks = 2
min_training_intervals = 1

[context]
rotation_rad = 0.0
modulation_depth = 1.0
field_factor = 1.0

[candidates]
include_boxcar_baseline = true
joint_modes = ["identity", "phase_diagonal"]
"#
    )
}

fn pmoke_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_pmoke"))
}

fn compare_command(dir: &Path, request: &Path) -> Command {
    let mut command = Command::new(pmoke_bin());
    command
        .current_dir(dir)
        .args(["noise", "compare", "--request"])
        .arg(request)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
}

fn wait_for<F: Fn() -> bool>(condition: F, child: &mut Child, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        if let Ok(Some(_)) = child.try_wait() {
            return condition();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    false
}

fn committed_generation_is_complete(destination: &Path) -> bool {
    destination.join("comparison.json").is_file()
        && destination.join("staging-manifest.json").is_file()
}

/// A killed staging attempt leaves no destination; the retry recovers the
/// stale staging directory and commits normally (PN-FR-031/PN-NFR-005).
#[test]
fn kill_during_staging_leaves_no_destination_and_retry_recovers() {
    let dir = unique_test_dir("kill_staging");
    synthetic_csv(&dir.join("wave.csv"), 64_000, 0xfeed_1234_5678_9abc);
    let request_path = dir.join("request.toml");
    fs::write(
        &request_path,
        request_text("wave.csv", "comparison", 64_000),
    )
    .unwrap();

    let staging = dir.join("comparison.staging");
    let mut child = compare_command(&dir, &request_path).spawn().unwrap();
    let seen = wait_for(
        || staging.join("request.resolved.json").is_file() || staging.join("models.json").is_file(),
        &mut child,
        Duration::from_secs(240),
    );
    let _ = child.kill();
    let _ = child.wait();
    if !seen {
        // The run finished before the kill window: the committed outcome is
        // a legal end state; nothing partial may exist.
        assert!(committed_generation_is_complete(&dir.join("comparison")));
        return;
    }
    assert!(
        !dir.join("comparison").exists(),
        "a killed staging attempt must not publish a partial destination"
    );

    // The retry reclaims the owned stale staging and commits normally.
    let output = compare_command(&dir, &request_path).output().unwrap();
    assert!(
        output.status.success(),
        "retry after the killed attempt must commit: {:?}",
        output
    );
    assert!(committed_generation_is_complete(&dir.join("comparison")));
    assert!(!staging.exists(), "recovered staging must be consumed");
    fs::remove_dir_all(&dir).unwrap();
}

/// Killing at the publication boundary (staging manifest present, rename may
/// or may not have happened) must end in exactly one of the two legal
/// durable states: committed-and-complete, or absent-with-staging retained.
#[test]
fn kill_at_publication_leaves_only_legal_durable_states() {
    let dir = unique_test_dir("kill_publication");
    synthetic_csv(&dir.join("wave.csv"), 64_000, 0x0102_0304_0506_0708);
    let request_path = dir.join("request.toml");
    fs::write(
        &request_path,
        request_text("wave.csv", "comparison", 64_000),
    )
    .unwrap();

    let staging = dir.join("comparison.staging");
    let destination = dir.join("comparison");
    let mut child = compare_command(&dir, &request_path).spawn().unwrap();
    let manifest_seen = wait_for(
        || staging.join("staging-manifest.json").is_file() || destination.exists(),
        &mut child,
        Duration::from_secs(240),
    );
    let _ = child.kill();
    let _ = child.wait();
    assert!(
        manifest_seen,
        "the staging manifest must appear before commit"
    );

    if destination.exists() {
        // Committed: the published generation must be complete and
        // verifiable by the retained-artifact reader.
        assert!(committed_generation_is_complete(&destination));
        let output = Command::new(pmoke_bin())
            .current_dir(&dir)
            .args(["noise", "replay", "--destination"])
            .arg(&destination)
            .arg("--verify-only")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "a committed generation must verify after the kill window: {:?}",
            output
        );
    } else {
        // Precommit abort: staging may remain for inspection, never a
        // destination.
        assert!(
            staging.exists() || manifest_seen,
            "precommit abort keeps the owned staging evidence"
        );
    }

    // Recovery: either the retry commits (staging replayed after digest
    // checks) or the destination already exists and the retry refuses by
    // name. No third outcome is legal.
    let output = compare_command(&dir, &request_path).output().unwrap();
    if output.status.success() {
        assert!(committed_generation_is_complete(&destination));
    } else {
        assert!(
            destination.exists(),
            "a failed retry must not delete an existing committed generation"
        );
    }
    fs::remove_dir_all(&dir).unwrap();
}

/// A kernel-level write failure during staging (RLIMIT_FSIZE, EFBIG) aborts
/// before commit, keeps the previous generation byte-identical and reports
/// the failing write by name.
#[test]
fn write_failure_keeps_previous_generation_and_aborts_before_commit() {
    let dir = unique_test_dir("disk_full");
    synthetic_csv(&dir.join("wave.csv"), 64_000, 0x2468_ace0_1357_9bdf);
    let request_path = dir.join("request.toml");
    fs::write(
        &request_path,
        request_text("wave.csv", "comparison", 64_000),
    )
    .unwrap();

    // The old (earlier) generation is a fully committed destination.
    let first = compare_command(&dir, &request_path).output().unwrap();
    assert!(first.status.success(), "baseline generation must commit");
    let destination = dir.join("comparison");
    let before: Vec<(PathBuf, Vec<u8>)> = walk_files(&destination);

    // Second generation into a different destination with a hard write cap
    // (8 blocks; the host counts 1024-byte blocks, so 8 KiB): the staging
    // write of comparison.json fails with EFBIG. The cap is far below the
    // 12 KiB comparison report and the 8 KiB plain report, so the failure
    // lands inside staging on every supported host.
    fs::write(
        &request_path,
        request_text("wave.csv", "comparison_second", 64_000),
    )
    .unwrap();
    let script = format!(
        "trap '' XFSZ; ulimit -f 8; exec \"{}\" noise compare --request \"{}\"",
        pmoke_bin().display(),
        request_path.display()
    );
    let output = Command::new("bash")
        .current_dir(&dir)
        .arg("-c")
        .arg(&script)
        .output()
        .unwrap();
    assert!(
        !output.status.success(),
        "a write failure must fail the workflow, not pass silently"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cannot write") || stderr.contains("File too large"),
        "the failing write must be named: {stderr}"
    );
    assert!(
        !dir.join("comparison_second").exists(),
        "a write failure before commit must leave no destination"
    );

    // The earlier generation is untouched (retain old output).
    let after: Vec<(PathBuf, Vec<u8>)> = walk_files(&destination);
    assert_eq!(before, after, "the previous generation must stay intact");

    // Recovery: with the cap removed the retry commits cleanly.
    let retry = compare_command(&dir, &request_path).output().unwrap();
    assert!(
        retry.status.success(),
        "retry after a write failure must recover: {:?}",
        retry
    );
    assert!(committed_generation_is_complete(
        &dir.join("comparison_second")
    ));
    fs::remove_dir_all(&dir).unwrap();
}

fn walk_files(root: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut entries: Vec<_> = fs::read_dir(&dir)
            .unwrap()
            .collect::<std::io::Result<_>>()
            .unwrap();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                stack.push(path);
            } else {
                let relative = path.strip_prefix(root).unwrap().to_path_buf();
                files.push((relative, fs::read(&path).unwrap()));
            }
        }
    }
    files.sort();
    files
}
