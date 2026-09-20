//! S4 headless key-event harness (Issue #278, FR-03, Phase 1).
//!
//! Drives the real dispatch path (`handle_key` -> `resolve_key` ->
//! `apply_key_action`) with scripted crossterm key events and renders the
//! result through the ratatui `TestBackend`. No terminal, no PTY, no child
//! processes: scripts must stay inside the safe-key discipline below.
//!
//! Safe-key discipline (harness scripts must avoid):
//! - `Enter` while Commands is focused (`RunSelected` spawns the selected
//!   workflow command as a child process);
//! - `y` / `Enter` on Output (`CopySelection` touches the OS clipboard);
//! - `q` on an idle app (`RequestQuit` quits immediately without a running
//!   command; the armed confirm path with a running command is covered by
//!   the `quit_confirmation_*` unit tests in `keymap.rs`);
//! - `r` (`Refresh` re-probes the filesystem; read-only but environment
//!   dependent, so refresh coverage stays in `app` unit tests).
//!
//! S3 REPORTS tab (Issue #277) is covered like every other inspector tab:
//! `5` selects it while the inspector is focused and `i` cycles through it,
//! so the Phase 2 snapshots drive those keys. No new `TuiAction` was needed
//! (scrolling reuses the inspector actions).

use super::*;
use ratatui::backend::TestBackend;

/// Canonical snapshot size classes (see `strings.md`).
pub(super) const WIDE: (u16, u16) = (120, 30);
pub(super) const COMPACT: (u16, u16) = (72, 24);
pub(super) const TINY: (u16, u16) = (40, 14);

/// Full-frame area for a size class, passed to `handle_key` for the
/// scroll/clamp paths that need a viewport.
pub(super) fn area_for(size: (u16, u16)) -> Rect {
    Rect::new(0, 0, size.0, size.1)
}

/// Idle dashboard app with deterministic chrome: fixed working directory
/// and scan root (`MonitorApp::new` reads the real cwd otherwise), pinned
/// motion/mouse modes (both resolve from `PMOKE_*` env), no running
/// command, no pending quit confirmation.
pub(super) fn harness_app() -> MonitorApp {
    let mut app = test_app();
    app.current_dir = "/workspace/pmoke".to_string();
    app.run_root = "/workspace/pmoke".to_string();
    app.motion_mode = MotionMode::Full;
    app.mouse_capture = true;
    app.quit_confirm_pending = false;
    app
}

pub(super) fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

pub(super) fn ch(char: char) -> KeyEvent {
    key(KeyCode::Char(char))
}

/// Feed scripted key events through the real dispatch path and return one
/// outcome per event. Panics on dispatch errors; `Quit` outcomes are
/// returned (not swallowed) so tests must assert them explicitly.
pub(super) fn drive(app: &mut MonitorApp, area: Rect, events: &[KeyEvent]) -> Vec<KeyOutcome> {
    events
        .iter()
        .map(|event| handle_key(app, *event, area).expect("headless dispatch"))
        .collect()
}

/// Assert every outcome kept looping (the script never confirmed quit).
pub(super) fn assert_continue(outcomes: &[KeyOutcome]) {
    assert!(
        outcomes
            .iter()
            .all(|outcome| *outcome == KeyOutcome::Continue),
        "harness script unexpectedly quit: {outcomes:?}"
    );
}

/// Render the dashboard to plain text, one line per row.
pub(super) fn render_text(app: &mut MonitorApp, size: (u16, u16)) -> String {
    let (width, height) = size;
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
fn harness_drives_focus_ring_without_quitting() {
    let mut app = harness_app();
    let area = area_for(WIDE);
    assert_eq!(app.focus, FocusPane::Commands);
    // 1/2/3/4 + b walk the four-pane ring (S2 order: Commands Runs
    // Inspector Output); Tab steps forward from any pane.
    let outcomes = drive(&mut app, area, &[ch('4')]);
    assert_continue(&outcomes);
    assert_eq!(app.focus, FocusPane::Runs);
    let outcomes = drive(&mut app, area, &[ch('b'), ch('1')]);
    assert_continue(&outcomes);
    assert_eq!(app.focus, FocusPane::Commands);
    let outcomes = drive(&mut app, area, &[ch('2'), ch('i')]);
    assert_continue(&outcomes);
    assert_eq!(app.focus, FocusPane::Inspector);
    // Inspector-local `3` selects the Diagnostics tab instead of moving
    // focus (pane-specific precedes the global fallback in REGISTRY order).
    let outcomes = drive(&mut app, area, &[ch('3'), key(KeyCode::Tab)]);
    assert_continue(&outcomes);
    assert_eq!(app.focus, FocusPane::Output);
}

#[test]
fn harness_help_overlay_opens_and_closes() {
    let mut app = harness_app();
    let area = area_for(WIDE);
    let outcomes = drive(&mut app, area, &[ch('?')]);
    assert_continue(&outcomes);
    assert!(app.show_help);
    // While open, unrelated keys are swallowed by the modal.
    let focus_before = app.focus;
    let outcomes = drive(&mut app, area, &[ch('4'), key(KeyCode::Tab)]);
    assert_continue(&outcomes);
    assert_eq!(app.focus, focus_before);
    let outcomes = drive(&mut app, area, &[key(KeyCode::Esc)]);
    assert_continue(&outcomes);
    assert!(!app.show_help);
}

#[test]
fn harness_search_types_clears_and_commits() {
    let mut app = harness_app();
    let area = area_for(WIDE);
    let outcomes = drive(
        &mut app,
        area,
        &[ch('/'), ch('a'), ch('b'), key(KeyCode::Backspace)],
    );
    assert_continue(&outcomes);
    assert!(app.search_mode);
    assert_eq!(app.action_query, "a");
    let outcomes = drive(&mut app, area, &[key(KeyCode::Enter)]);
    assert_continue(&outcomes);
    assert!(!app.search_mode);
    assert_eq!(app.action_query, "a");
}
