//! S3 calibration/analysis inspector tests (Issue #277).
//!
//! - FR-01: each report lane summarizes its JSON shape; missing,
//!   oversized, symlinked, or unparsable files degrade to explicit
//!   markers instead of failing.
//! - FR-02: per-stage rows derive from the run manifest lifecycle fields,
//!   with `"—"` until a milestone is reached and failure/attempt rows only
//!   when the manifest carries them.
//! - Read-only: probing never creates, writes, or follows symlinks
//!   (tree snapshot before/after is byte-identical).
//! - The REPORTS tab renders the combined table and scrolls it; the empty
//!   state names the scan root and budgets.

use super::*;
use ratatui::backend::TestBackend;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static INSPECT_TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_root(name: &str) -> PathBuf {
    let id = INSPECT_TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let root =
        std::env::temp_dir().join(format!("pmoke_tui_s3_{name}_{}_{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn finish_temp_root(root: &Path) {
    let _ = std::fs::remove_dir_all(root);
}

/// Recursive `(relative path, bytes)` snapshot proving read-only probes.
/// Local copy of the S2 helper (that one is private to `runs.rs`); kept
/// independent so this test cannot pass trivially.
fn snapshot_tree(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut children: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .collect();
        children.sort();
        for child in children {
            if child.is_dir() && !child.is_symlink() {
                stack.push(child);
            } else if child.is_file() {
                let relative = child
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let bytes = std::fs::read(&child).unwrap();
                out.push((relative, bytes));
            }
        }
    }
    out.sort();
    out
}

fn write_run_manifest(dir: &Path, body: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join("run.toml"), body).unwrap();
}

fn full_manifest() -> String {
    [
        "schema_version = 2",
        "status = \"complete\"",
        "stage = \"analysis\"",
        "pmoke_version = \"test\"",
        "started_at = \"2026-01-01T00:00:00Z\"",
        "acquired_at = \"2026-01-01T01:00:00Z\"",
        "analysis_started_at = \"2026-01-01T02:00:00Z\"",
        "analyzed_at = \"2026-01-01T03:00:00Z\"",
        "updated_at = \"2026-01-01T04:00:00Z\"",
        "completed_at = \"2026-01-01T05:00:00Z\"",
        "[analysis]",
        "published_through = \"moke\"",
        "published_generation = 3",
        "[analysis.last_attempt]",
        "generation = 3",
        "command = \"moke\"",
        "status = \"complete\"",
        "started_at = \"2026-01-01T02:00:00Z\"",
        "completed_at = \"2026-01-01T03:00:00Z\"",
        "",
    ]
    .join("\n")
}

fn minimal_manifest() -> String {
    [
        "schema_version = 2",
        "status = \"acquired\"",
        "stage = \"fetch\"",
        "pmoke_version = \"test\"",
        "started_at = \"2026-01-01T00:00:00Z\"",
        "updated_at = \"2026-01-01T01:00:00Z\"",
        "",
    ]
    .join("\n")
}

fn row_value(rows: &[Vec<String>], item: &str) -> Option<String> {
    rows.iter()
        .find(|row| row.first().is_some_and(|first| first == item))
        .and_then(|row| row.get(1))
        .cloned()
}

fn reports_app_with_run(run_dir: &Path) -> MonitorApp {
    let mut app = test_app();
    app.run_entries = vec![RunDirEntry {
        name: "run-alpha".to_string(),
        path: run_dir.to_string_lossy().into_owned(),
        status: "complete".to_string(),
        stage: "analysis".to_string(),
        updated: "2026-01-01".to_string(),
        has_acquisition: true,
        has_analysis: true,
    }];
    app.run_cursor = 0;
    app.inspector_view = InspectorView::Reports;
    app
}

fn render_text(width: u16, height: u16, app: &mut MonitorApp) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn stage_state_reads_full_lifecycle_and_attempt() {
    let root = temp_root("stages_full");
    let run = root.join("run-a");
    write_run_manifest(&run, &full_manifest());

    let state = read_stage_state(&run);

    assert_eq!(state.status, "complete");
    assert_eq!(state.stage, "analysis");
    assert_eq!(state.acquired_at, "2026-01-01T01:00:00Z");
    assert_eq!(state.completed_at, "2026-01-01T05:00:00Z");
    assert_eq!(state.analysis_command, "moke");
    assert_eq!(state.analysis_status, "complete");
    assert_eq!(state.analysis_generation, "3");
    assert_eq!(state.published_through, "moke");
    assert_eq!(state.published_generation, "3");
    assert_eq!(state.failed_stage, "—");
    assert_eq!(state.error_summary, "—");

    let rows = stage_rows(&state);
    assert_eq!(row_value(&rows, "Status"), Some("complete".to_string()));
    assert_eq!(
        row_value(&rows, "Acquired"),
        Some("2026-01-01T01:00:00Z".to_string())
    );
    assert_eq!(
        row_value(&rows, "Attempt"),
        Some("moke complete (gen 3)".to_string())
    );
    assert_eq!(
        row_value(&rows, "Published"),
        Some("through moke (gen 3)".to_string())
    );
    assert!(row_value(&rows, "Failed stage").is_none());

    finish_temp_root(&root);
}

#[test]
fn stage_state_degrades_gracefully_before_milestones() {
    let root = temp_root("stages_minimal");
    let run = root.join("run-a");
    write_run_manifest(&run, &minimal_manifest());

    let state = read_stage_state(&run);

    assert_eq!(state.status, "acquired");
    assert_eq!(state.acquired_at, "—");
    assert_eq!(state.completed_at, "—");
    assert_eq!(state.analysis_command, "—");
    assert_eq!(state.published_through, "—");

    let rows = stage_rows(&state);
    assert_eq!(row_value(&rows, "Acquired"), Some("—".to_string()));
    assert!(row_value(&rows, "Attempt").is_none());
    assert!(row_value(&rows, "Published").is_none());
    assert!(row_value(&rows, "Failed stage").is_none());

    finish_temp_root(&root);
}

#[test]
fn stage_state_reports_failure_detail_when_failed() {
    let root = temp_root("stages_failed");
    let run = root.join("run-a");
    write_run_manifest(
        &run,
        &[
            "schema_version = 2",
            "status = \"failed\"",
            "stage = \"li\"",
            "pmoke_version = \"test\"",
            "started_at = \"2026-01-01T00:00:00Z\"",
            "updated_at = \"2026-01-01T01:00:00Z\"",
            "failed_stage = \"li\"",
            "error_summary = \"lock-in blew up\"",
            "",
        ]
        .join("\n"),
    );

    let rows = stage_rows(&read_stage_state(&run));

    assert_eq!(row_value(&rows, "Status"), Some("failed".to_string()));
    assert_eq!(row_value(&rows, "Failed stage"), Some("li".to_string()));
    assert_eq!(
        row_value(&rows, "Error"),
        Some("lock-in blew up".to_string())
    );

    finish_temp_root(&root);
}

#[test]
fn stage_state_unknown_without_or_beyond_manifest() {
    let root = temp_root("stages_unknown");
    let run = root.join("run-a");
    std::fs::create_dir_all(&run).unwrap();

    let missing = read_stage_state(&run);
    assert_eq!(missing.status, "unknown");
    assert_eq!(missing.started_at, "—");

    std::fs::write(run.join("run.toml"), "not = [valid").unwrap();
    let broken = read_stage_state(&run);
    assert_eq!(broken.status, "unknown");

    // Oversized manifests stay unread (bounded probe, never parsed).
    let big = "status = \"complete\"\n".repeat(8 * 1024);
    std::fs::write(run.join("run.toml"), big).unwrap();
    let oversized = read_stage_state(&run);
    assert_eq!(oversized.status, "unknown");

    finish_temp_root(&root);
}

#[test]
fn summarize_calibrate_extracts_binding_fields() {
    let value: serde_json::Value = serde_json::from_str(
        r#"{
            "model_id": "m1",
            "adequate": true,
            "training_blocks": 12,
            "modes": ["boxcar", "gls"],
            "tolerance_basis": {"final_tol": 0.0000123},
            "warnings": ["w1", "w2"]
        }"#,
    )
    .unwrap();

    let summary = ReportKind::Calibrate.summarize(&value);
    let get = |key: &str| {
        summary
            .iter()
            .find(|(item, _)| item == key)
            .map(|(_, val)| val.clone())
            .unwrap()
    };

    assert_eq!(get("model"), "m1");
    assert_eq!(get("adequate"), "true");
    assert_eq!(get("blocks"), "12");
    assert_eq!(get("modes"), "boxcar,gls");
    assert_eq!(get("tolerance"), "1.230000e-5");
    assert_eq!(get("warnings"), "2");
}

#[test]
fn summarize_noise_compare_evaluate_extract_verdicts() {
    let noise: serde_json::Value = serde_json::from_str(
        r#"{
            "operation": "diagnose",
            "diagnostic_finding": "supported",
            "finding_reason": "clean background",
            "measurements_available": true
        }"#,
    )
    .unwrap();
    let noise_summary = ReportKind::Noise.summarize(&noise);
    assert!(noise_summary.contains(&("finding".to_string(), "supported".to_string())));

    let compare: serde_json::Value = serde_json::from_str(
        r#"{
            "gate_version": "g1",
            "legs": [
                {"name": "a", "status": "complete"},
                {"name": "b", "status": "failed"}
            ],
            "shared_grid_equal": true,
            "shared_reference_equal": false,
            "computation_complete": true
        }"#,
    )
    .unwrap();
    let compare_summary = ReportKind::Compare.summarize(&compare);
    let get = |key: &str| {
        compare_summary
            .iter()
            .find(|(item, _)| item == key)
            .map(|(_, val)| val.clone())
            .unwrap()
    };
    assert_eq!(get("legs"), "1/2 complete");
    assert_eq!(get("reference"), "false");
    assert_eq!(get("complete"), "true");

    let evaluate: serde_json::Value = serde_json::from_str(
        r#"{
            "method_label": "m6",
            "channel": 2,
            "benefit_gate": "pass",
            "scientific_verdict": "fail",
            "sd_ratio": 1.23456,
            "computation_complete": true
        }"#,
    )
    .unwrap();
    let evaluate_summary = ReportKind::Evaluate.summarize(&evaluate);
    assert!(evaluate_summary.contains(&("verdict".to_string(), "fail".to_string())));
    assert!(evaluate_summary.contains(&("sd_ratio".to_string(), "1.2346".to_string())));
}

#[test]
fn summarize_unknown_shapes_degrade_to_unknown() {
    // A staging `diagnostics.json` from another lane carries none of the
    // diagnose keys; the inspector must show unknowns, never fail.
    let value: serde_json::Value = serde_json::from_str(r#"{"staging": true}"#).unwrap();
    let summary = ReportKind::Noise.summarize(&value);
    assert!(
        summary
            .iter()
            .all(|(_, val)| val == "unknown" || val == "—" || val.len() <= 100)
    );
}

#[test]
fn report_rows_probe_run_dir_with_explicit_markers() {
    let root = temp_root("report_rows");
    let run = root.join("run-a");
    std::fs::create_dir_all(&run).unwrap();
    write_run_manifest(&run, &minimal_manifest());
    std::fs::write(
        run.join("compare-report.json"),
        r#"{"gate_version": "g9", "legs": [], "shared_grid_equal": true,
            "shared_reference_equal": true, "computation_complete": false}"#,
    )
    .unwrap();
    std::fs::create_dir_all(run.join("analysis")).unwrap();
    std::fs::write(
        run.join("analysis").join("manifest.toml"),
        "published_through = \"signal\"\npublished_generation = 2\n",
    )
    .unwrap();
    let before = snapshot_tree(&root);

    let rows = report_table_rows(&run);

    let after = snapshot_tree(&root);
    assert_eq!(before, after, "report probes must not write");

    let compare = row_value(&rows, "compare").expect("compare row");
    assert!(compare.contains("gate=g9"), "{compare}");
    assert!(compare.contains("complete=false"), "{compare}");
    let noise = row_value(&rows, "noise").expect("noise row");
    assert!(noise.contains("not found"), "{noise}");
    let analysis = row_value(&rows, "analysis results").expect("analysis row");
    assert!(analysis.contains("signal"), "{analysis}");
    assert!(analysis.contains("gen 2"), "{analysis}");

    finish_temp_root(&root);
}

#[test]
#[cfg(unix)]
fn report_probes_refuse_symlinks() {
    let root = temp_root("report_symlink");
    let run = root.join("run-a");
    std::fs::create_dir_all(&run).unwrap();
    write_run_manifest(&run, &minimal_manifest());
    let outside = root.join("outside.json");
    std::fs::write(&outside, r#"{"gate_version": "sneaky"}"#).unwrap();
    std::os::unix::fs::symlink(&outside, run.join("compare-report.json")).unwrap();

    let rows = report_table_rows(&run);

    let compare = row_value(&rows, "compare").expect("compare row");
    assert!(
        compare.contains("not found"),
        "symlinked report must not be followed: {compare}"
    );

    finish_temp_root(&root);
}

#[test]
fn reports_tab_renders_stages_and_reports_sections() {
    let root = temp_root("reports_render");
    let run = root.join("run-a");
    std::fs::create_dir_all(&run).unwrap();
    write_run_manifest(&run, &full_manifest());
    std::fs::write(
        run.join("m6-evaluation-report.json"),
        r#"{"method_label": "m6", "channel": 1, "benefit_gate": "pass",
            "scientific_verdict": "pass", "computation_complete": true}"#,
    )
    .unwrap();
    let mut app = reports_app_with_run(&run);
    let area = Rect::new(0, 0, 100, 40);

    // Wide dashboards pin the inspector pane to 9 rows, so the combined
    // table scrolls: the first window shows the run header + STAGES head.
    let rendered = render_text(100, 40, &mut app);
    assert!(rendered.contains("REPORTS"), "tab title with scroll range");
    assert!(rendered.contains("Stages"), "stages section marker");

    scroll_inspector_down(&mut app, area, 9);
    let mid = render_text(100, 40, &mut app);
    assert!(mid.contains("Acquired"), "lifecycle row");
    assert!(mid.contains("2026-01-01T01:00:00Z"), "lifecycle timestamp");
    assert!(mid.contains("moke complete (gen 3)"), "attempt row");

    scroll_inspector_down(&mut app, area, 10_000);
    let bottom = render_text(100, 40, &mut app);
    assert!(bottom.contains("evaluate"), "report lane row");
    assert!(bottom.contains("verdict=pass"), "report summary");
    // (Item cells truncate to 14 chars, so pin the value text instead.)
    assert!(bottom.contains("no analysis manifest"), "analysis manifest row");
    // The section marker sits one row above the clamped last window; its
    // presence is pinned on the row list itself.
    assert!(
        reports_table_rows(&app)
            .iter()
            .any(|row| row.first().is_some_and(|first| first == "Reports")),
        "reports section marker"
    );

    // Small narrow panes must clip, never panic (the inspector needs a
    // 19+ row body, i.e. a 22+ row terminal in narrow layouts).
    let tiny = render_text(60, 24, &mut app);
    assert!(tiny.contains("REPORTS"));

    finish_temp_root(&root);
}

#[test]
fn reports_tab_empty_state_without_selection() {
    let mut app = test_app();
    app.inspector_view = InspectorView::Reports;

    let rendered = render_text(100, 30, &mut app);

    assert!(rendered.contains("No recorded run selected."));
    assert!(rendered.contains("budgets 512 dirs"));
}

#[test]
fn reports_scroll_windows_the_combined_table() {
    let root = temp_root("reports_scroll");
    let run = root.join("run-a");
    std::fs::create_dir_all(&run).unwrap();
    write_run_manifest(&run, &full_manifest());
    let mut app = reports_app_with_run(&run);
    let area = Rect::new(0, 0, 100, 40);
    let total = reports_table_rows(&app).len();
    assert!(total > 10, "combined table exceeds one pane: {total}");

    scroll_inspector_down(&mut app, area, 3);
    assert_eq!(app.reports_scroll, 3);
    scroll_inspector_up(&mut app, area, 1);
    assert_eq!(app.reports_scroll, 2);

    // Clamp pins to the last window; other tabs are untouched.
    scroll_inspector_down(&mut app, area, 10_000);
    assert!(app.reports_scroll <= total);
    assert_eq!(app.config_scroll, 0);
    assert_eq!(app.files_scroll, 0);

    app.refresh();
    assert_eq!(app.reports_scroll, 0);

    finish_temp_root(&root);
}
