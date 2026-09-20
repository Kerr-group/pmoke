//! S4 canonical-frame snapshots + cell-budget tests (Issue #278, FR-02/FR-03).
//!
//! Phase 1 covered S1/S2 screens: the idle dashboard at all three size
//! classes, the `?` help overlay, the S2 runs-browser focus, and the S1
//! inspector tab cycle. Phase 2 adds the S3 REPORTS tab (empty state plus a
//! selected run whose probes all degrade to explicit markers).
//!
//! Frames render through the same `TestBackend` path as `view.rs` tests, but
//! pinned with `insta` so any chrome drift fails loudly. The dashboard app
//! is the harness idle app (fixed cwd, no running command), so frames are
//! deterministic: `push_output` stamps no elapsed time and the header clock
//! only ticks whole minutes.

use super::harness::*;
use super::*;
use unicode_width::UnicodeWidthStr;

/// Canonical 10-action workflow fixture for snapshots (Issue #278).
///
/// `monitor_actions()` is feature-gated: `--no-default-features` yields 10
/// actions (12 WORKFLOW entries with group headers) while `hw-core` (default
/// and `--all-features` CI) adds Single/Trigger/Autoshot/Fetch/Screenshot/
/// Automeasure/Process/Auto for 18 actions (22 entries). The committed
/// `.snap` files were authored against the 12-entry registry, so CI rendered
/// `WORKFLOW 02/22` plus an `ACQUISITION` block where snapshots expect
/// `WORKFLOW 02/12`. Pinning the harness to this fixed list makes all 8
/// frames render identically in every feature leg with no `.snap` changes.
fn canonical_snapshot_actions() -> Vec<MonitorAction> {
    vec![
        MonitorAction::Show,
        MonitorAction::Reference,
        MonitorAction::Sensor,
        MonitorAction::Li,
        MonitorAction::Signal,
        MonitorAction::Phase,
        MonitorAction::Moke,
        MonitorAction::Analyze,
        MonitorAction::Doctor,
        MonitorAction::RawVerify,
    ]
}

/// Pin the workflow registry to the canonical fixture and reset workflow
/// navigation to the safe Config action (row 1), clearing any search or
/// collapsed-group state so the WORKFLOW panel is fully deterministic.
fn pin_canonical_workflow(app: &mut MonitorApp) {
    app.workflow_fixture = Some(canonical_snapshot_actions());
    app.collapsed_groups.clear();
    app.action_query.clear();
    app.search_mode = false;
    app.workflow_cursor = 1;
}

/// Idle dashboard with one probe line in the activity panel.
fn probe_app() -> MonitorApp {
    let mut app = harness_app();
    pin_canonical_workflow(&mut app);
    app.push_output(OutputStream::System, "snapshot probe line");
    app
}

fn assert_fits_width(text: &str, size: (u16, u16)) {
    let (width, height) = size;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), height as usize, "frame height drifted");
    for (row, line) in lines.iter().enumerate() {
        let cells = UnicodeWidthStr::width(*line);
        assert!(
            cells <= width as usize,
            "row {row} is {cells} cells, wider than {width}: {line:?}"
        );
    }
}

#[test]
fn snapshot_dashboard_wide() {
    let mut app = probe_app();
    let rendered = render_text(&mut app, WIDE);
    assert_fits_width(&rendered, WIDE);
    insta::assert_snapshot!("dashboard_wide", rendered);
}

#[test]
fn snapshot_dashboard_compact() {
    let mut app = probe_app();
    let rendered = render_text(&mut app, COMPACT);
    assert_fits_width(&rendered, COMPACT);
    insta::assert_snapshot!("dashboard_compact", rendered);
}

#[test]
fn snapshot_dashboard_tiny() {
    let mut app = probe_app();
    let rendered = render_text(&mut app, TINY);
    assert_fits_width(&rendered, TINY);
    assert!(rendered.contains("WORKFLOW"));
    assert!(rendered.contains("ACTIVITY"));
    assert!(!rendered.contains("INSPECTOR"));
    insta::assert_snapshot!("dashboard_tiny", rendered);
}

#[test]
fn snapshot_help_overlay() {
    let mut app = probe_app();
    let outcomes = drive(&mut app, area_for(WIDE), &[ch('?')]);
    assert_continue(&outcomes);
    assert!(app.show_help);
    let rendered = render_text(&mut app, WIDE);
    assert_fits_width(&rendered, WIDE);
    assert!(rendered.contains("GLOBAL"));
    assert!(rendered.contains("RUNS"));
    insta::assert_snapshot!("help_overlay", rendered);
}

#[test]
fn snapshot_runs_focused() {
    let mut app = probe_app();
    let outcomes = drive(&mut app, area_for(WIDE), &[ch('4')]);
    assert_continue(&outcomes);
    assert_eq!(app.focus, FocusPane::Runs);
    let rendered = render_text(&mut app, WIDE);
    assert_fits_width(&rendered, WIDE);
    assert!(rendered.contains("RUNS"));
    insta::assert_snapshot!("runs_focused", rendered);
}

#[test]
fn snapshot_inspector_artifacts_tab() {
    let mut app = ready_probe_app();
    let outcomes = drive(&mut app, area_for(WIDE), &[ch('2'), ch('i'), ch('i')]);
    assert_continue(&outcomes);
    assert_eq!(app.focus, FocusPane::Inspector);
    let rendered = render_text(&mut app, WIDE);
    assert_fits_width(&rendered, WIDE);
    insta::assert_snapshot!("inspector_tabbed", rendered);
}

/// Ready-config app (RUNNABLE header) with a probe line, for frames that
/// differ by load state. Same determinism pins as [`harness_app`].
fn ready_probe_app() -> MonitorApp {
    let mut app = ready_test_app(2);
    app.current_dir = "/workspace/pmoke".to_string();
    app.run_root = "/workspace/pmoke".to_string();
    app.motion_mode = MotionMode::Full;
    app.mouse_capture = true;
    app.quit_confirm_pending = false;
    pin_canonical_workflow(&mut app);
    app.push_output(OutputStream::System, "snapshot probe line");
    app
}

/// CJK/wide-character cell budget: truncation keeps display width within
/// budget (CJK = 2 cells) and padding reaches exact width, so callers fit
/// first and pad second.
#[test]
fn cjk_probe_stays_within_budget() {
    let probe = "あいうえお RUNS 01/20 ◆ cwd";
    for width in [8, 12, 20, 24] {
        let fitted = fit_text(probe, width);
        assert!(
            UnicodeWidthStr::width(fitted.as_str()) <= width,
            "fit_text overflow at {width}: {fitted:?}"
        );
    }
    // Padding a short mixed-width string reaches exact width (pad never
    // truncates: over-wide input is the caller's `fit_text` duty).
    let padded = pad_display_width("あい RUNS", 12);
    assert_eq!(UnicodeWidthStr::width(padded.as_str()), 12);
    // Full-width input fitted to a narrow budget stays within budget too.
    let fitted = fit_text("ＡＢＣＤＥＦＧＨＩＪＫＬＭ", 10);
    assert!(UnicodeWidthStr::width(fitted.as_str()) <= 10);
}

/// S3 REPORTS tab over a selected run whose directory carries no manifest
/// or reports. Every probe degrades to its explicit `unknown` / `not found`
/// marker, so the frame is fully deterministic: the run path is a fixed
/// fixture string (no temp dirs, whose PID-scoped names would poison the
/// snapshot) and the repo root carries no `./calibration/` fallback for the
/// calibrate lane to find when tests run with the package root as cwd.
fn reports_probe_app() -> MonitorApp {
    let mut app = ready_probe_app();
    app.run_entries = vec![RunDirEntry {
        name: "run-alpha".to_string(),
        path: "/workspace/pmoke/runs/run-alpha".to_string(),
        status: "complete".to_string(),
        stage: "analysis".to_string(),
        updated: "2026-01-01".to_string(),
        has_acquisition: true,
        has_analysis: true,
    }];
    app.run_cursor = 0;
    app
}

#[test]
fn snapshot_reports_tab_empty() {
    let mut app = probe_app();
    let outcomes = drive(&mut app, area_for(WIDE), &[ch('2'), ch('5')]);
    assert_continue(&outcomes);
    assert_eq!(app.focus, FocusPane::Inspector);
    assert_eq!(app.inspector_view, InspectorView::Reports);
    let rendered = render_text(&mut app, WIDE);
    assert_fits_width(&rendered, WIDE);
    assert!(rendered.contains("REPORTS"));
    assert!(rendered.contains("No recorded run selected."));
    insta::assert_snapshot!("reports_empty", rendered);
}

#[test]
fn snapshot_reports_tab_populated() {
    let mut app = reports_probe_app();
    let outcomes = drive(&mut app, area_for(WIDE), &[ch('2'), ch('5')]);
    assert_continue(&outcomes);
    assert_eq!(app.focus, FocusPane::Inspector);
    assert_eq!(app.inspector_view, InspectorView::Reports);
    let rendered = render_text(&mut app, WIDE);
    assert_fits_width(&rendered, WIDE);
    assert!(rendered.contains("run-alpha"));
    assert!(rendered.contains("Stages"));
    insta::assert_snapshot!("reports_populated", rendered);
}
