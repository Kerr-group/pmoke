//! S4 canonical-frame snapshots + cell-budget tests (Issue #278, FR-02/FR-03).
//!
//! Phase 1 covers S1/S2 screens only: the idle dashboard at all three size
//! classes, the `?` help overlay, the S2 runs-browser focus, and the S1
//! inspector tab cycle. S3 report screens get their own snapshots in Phase 2.
//!
//! Frames render through the same `TestBackend` path as `view.rs` tests, but
//! pinned with `insta` so any chrome drift fails loudly. The dashboard app
//! is the harness idle app (fixed cwd, no running command), so frames are
//! deterministic: `push_output` stamps no elapsed time and the header clock
//! only ticks whole minutes.

use super::harness::*;
use super::*;
use unicode_width::UnicodeWidthStr;

/// Idle dashboard with one probe line in the activity panel.
fn probe_app() -> MonitorApp {
    let mut app = harness_app();
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
