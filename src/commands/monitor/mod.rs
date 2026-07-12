use crate::config::{
    self, Config, ConfigDiagnostics, ConfigLoad, ConfigWarning, FetchAnalysisInput, FetchOutput,
};
use crate::ui::{EventKind, EventLevel, UiEvent};
use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal, TerminalOptions, Viewport,
    backend::CrosstermBackend,
    buffer::{Buffer, CellWidth},
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    prelude::{Color, Line, Modifier, Span, Style},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Row, Table, Wrap},
};
use std::{
    env, fs,
    io::{self, Read, Stdout},
    process::{Child, Command as ProcessCommand, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError},
    thread,
    time::{Duration, Instant, SystemTime},
};
use tachyonfx::{CellFilter, Duration as FxDuration, EffectManager, Interpolation, Motion, fx};
use tui_spinner::FluxFrames;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

mod actions;
mod app;
mod clipboard;
mod formatting;
mod layout;
mod output;
mod panels;
mod timeline;
mod view;

use actions::{
    ActionGroup, MonitorAction, WorkflowEntry, action_readiness, action_runnable, monitor_actions,
};
use app::*;
#[cfg(test)]
use clipboard::base64_encode;
use clipboard::{ClipboardMethod, copy_text_to_clipboard};
use formatting::{
    bordered_inner, centered_rect, centered_text, contains, fit_path, fit_text, format_age,
    format_duration, format_live_duration, pad_display_width, percent_width, strip_ansi_codes,
};
#[cfg(test)]
use layout::workflow_panel_width;
use layout::{
    UiLayout, config_panel_layout, latest_event_feed_effect_area, output_inner_layout,
    output_visible_rows, workflow_layout,
};
use output::*;
use panels::*;
#[cfg(test)]
use timeline::{
    StageProgressState, TimelineStep, TimelineStepState, timeline_badge_cell, timeline_for_action,
    timeline_separator, timeline_step_lines, timeline_step_spans,
};
use timeline::{render_run_timeline, spinner_frame, timeline_motion_frame};
use view::*;

const TUI_IDLE_TICK: Duration = Duration::from_millis(150);
const TUI_ANIMATION_TICK: Duration = Duration::from_millis(16);
const CONTEXT_DETAILS_MIN_WIDTH: usize = 60;
const OUTPUT_PREFIX_WIDTH: u16 = 12;
const EVENT_BADGE_WIDTH: usize = 6;
const TIMELINE_BADGE_WIDTH: usize = 5;
const EVENT_FEED_EFFECT_MS: u32 = 520;

struct TerminalGuard<'a> {
    terminal: &'a mut Terminal<CrosstermBackend<Stdout>>,
    armed: bool,
}

impl<'a> TerminalGuard<'a> {
    fn new(terminal: &'a mut Terminal<CrosstermBackend<Stdout>>) -> Self {
        Self {
            terminal,
            armed: true,
        }
    }

    fn terminal(&mut self) -> &mut Terminal<CrosstermBackend<Stdout>> {
        self.terminal
    }

    fn restore(mut self) -> Result<()> {
        self.armed = false;
        restore_terminal(self.terminal)
    }
}

impl Drop for TerminalGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            let _ = restore_terminal(self.terminal);
        }
    }
}

#[derive(Default)]
struct TerminalSetupGuard {
    raw_mode: bool,
    alternate_screen: bool,
    mouse_capture: bool,
    armed: bool,
}

impl TerminalSetupGuard {
    fn new() -> Self {
        Self {
            armed: true,
            ..Self::default()
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TerminalSetupGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut stdout = io::stdout();
        if self.mouse_capture {
            let _ = execute!(stdout, DisableMouseCapture);
        }
        if self.alternate_screen {
            let _ = execute!(stdout, LeaveAlternateScreen);
        }
        if self.raw_mode {
            let _ = disable_raw_mode();
        }
    }
}

pub fn monitor(config_path: &str, load: ConfigLoad) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let mut guard = TerminalGuard::new(&mut terminal);
    let mut app = MonitorApp::new(config_path.to_string(), load);
    let run_result = run(guard.terminal(), &mut app);
    let restore_result = guard.restore();
    match run_result {
        Err(error) => Err(error),
        Ok(()) => restore_result,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
enum MonitorEffect {
    #[default]
    EventFeedLatest,
}

fn fx_duration(elapsed: Duration) -> FxDuration {
    let millis = elapsed.as_millis().min(u128::from(u32::MAX)) as u32;
    FxDuration::from_millis(millis)
}

#[derive(Clone)]
struct RunRecord {
    action: MonitorAction,
    label: &'static str,
    elapsed: Duration,
    result: String,
    ok: bool,
}

#[derive(Clone)]
struct RunSnapshot {
    record: RunRecord,
    output: Vec<LogEntry>,
}

struct ActiveRun {
    action: MonitorAction,
    label: &'static str,
    started_at: Instant,
    receiver: Receiver<RunEvent>,
    cancel: Sender<CancelReason>,
    cancel_requested: bool,
}

struct RunHandle {
    receiver: Receiver<RunEvent>,
    cancel: Sender<CancelReason>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CancelReason {
    CtrlC,
    Closed,
}

impl CancelReason {
    fn label(self) -> &'static str {
        match self {
            Self::CtrlC => "Ctrl+C",
            Self::Closed => "control channel closed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FocusPane {
    Commands,
    Inspector,
    Output,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum InspectorView {
    #[default]
    Summary,
    Config,
    Diagnostics,
    Artifacts,
}

impl InspectorView {
    fn next(self) -> Self {
        match self {
            Self::Summary => Self::Config,
            Self::Config => Self::Diagnostics,
            Self::Diagnostics => Self::Artifacts,
            Self::Artifacts => Self::Summary,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Summary => "SUMMARY",
            Self::Config => "CONFIG",
            Self::Diagnostics => "DIAGNOSTICS",
            Self::Artifacts => "ARTIFACTS",
        }
    }
}

#[derive(Clone)]
struct LogEntry {
    text: String,
    kind: LogKind,
    sequence: Option<u64>,
    elapsed_ms: Option<u64>,
    event_head: bool,
    stream: OutputStream,
    transient: bool,
}

impl LogEntry {
    #[cfg(test)]
    fn new(stream: OutputStream, text: impl Into<String>) -> Self {
        let text = text.into();
        let kind = classify_log_entry(stream, &strip_ansi_codes(&text));
        Self {
            text,
            kind,
            sequence: None,
            elapsed_ms: None,
            event_head: false,
            stream,
            transient: false,
        }
    }

    fn with_kind(text: String, kind: LogKind, stream: OutputStream) -> Self {
        Self {
            text,
            kind,
            sequence: None,
            elapsed_ms: None,
            event_head: false,
            stream,
            transient: false,
        }
    }

    fn from_event(event: &UiEvent) -> Vec<Self> {
        let kind = log_kind_for_event(event);
        let text = if event.kind == EventKind::Section {
            format!("╭─ {}", event.message)
        } else {
            event.message.clone()
        };
        let mut entries = vec![Self {
            text,
            kind,
            sequence: Some(event.sequence),
            elapsed_ms: Some(event.elapsed_ms),
            event_head: true,
            stream: OutputStream::Stdout,
            transient: event.kind == EventKind::Progress,
        }];
        entries.extend(event.fields.iter().map(|(key, value)| Self {
            text: format!("│ {key}  {value}"),
            kind: LogKind::Metric,
            sequence: Some(event.sequence),
            elapsed_ms: Some(event.elapsed_ms),
            event_head: false,
            stream: OutputStream::Stdout,
            transient: false,
        }));
        entries
    }
}

fn log_kind_for_event(event: &UiEvent) -> LogKind {
    match event.level {
        EventLevel::Error => LogKind::Error,
        EventLevel::Warning => LogKind::Warning,
        EventLevel::Success if event.kind == EventKind::Save => LogKind::Save,
        EventLevel::Success => LogKind::Success,
        EventLevel::Info => match event.kind {
            EventKind::Read => LogKind::Read,
            EventKind::Save => LogKind::Save,
            EventKind::Skip => LogKind::Skipped,
            EventKind::Section => LogKind::Section,
            EventKind::Metric => LogKind::Metric,
            EventKind::System => LogKind::System,
            _ => LogKind::Info,
        },
    }
}

enum RunEvent {
    Output(OutputStream, String),
    Progress(OutputStream, String),
    Structured(UiEvent),
    Finished { ok: bool, status: String },
    Failed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputStream {
    Stdout,
    Stderr,
    System,
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    let mut guard = TerminalSetupGuard::new();
    enable_raw_mode()?;
    guard.raw_mode = true;
    let mut stdout = io::stdout();
    // Mark each transition before attempting it. If a write fails after the
    // terminal consumed the escape sequence, rollback still sends its inverse.
    guard.alternate_screen = true;
    execute!(stdout, EnterAlternateScreen)?;
    guard.mouse_capture = true;
    execute!(stdout, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Fullscreen,
        },
    )?;
    guard.disarm();
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    let mut first_error = disable_raw_mode().err().map(anyhow::Error::from);
    if let Err(error) = execute!(terminal.backend_mut(), DisableMouseCapture) {
        first_error.get_or_insert_with(|| error.into());
    }
    if let Err(error) = execute!(terminal.backend_mut(), LeaveAlternateScreen) {
        first_error.get_or_insert_with(|| error.into());
    }
    if let Err(error) = terminal.show_cursor() {
        first_error.get_or_insert_with(|| error.into());
    }
    first_error.map_or(Ok(()), Err)
}

fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut MonitorApp) -> Result<()> {
    loop {
        app.poll_command();
        let effect_delta = app.effect_delta();
        terminal.draw(|frame| render(frame, app, effect_delta))?;

        let tick = tui_frame_tick(app);
        if event::poll(tick)? {
            match event::read()? {
                Event::Key(key) => {
                    if key.kind != KeyEventKind::Press {
                        continue;
                    }

                    match key.code {
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.interrupt_current_operation();
                        }
                        KeyCode::Char('?') if !app.search_mode => app.show_help = !app.show_help,
                        KeyCode::Esc if app.search_mode || !app.action_query.is_empty() => {
                            app.clear_action_search()
                        }
                        KeyCode::Esc => app.escape_current_mode(),
                        KeyCode::Char('q') if app.show_help => {
                            app.show_help = false;
                        }
                        _ if app.show_help => {}
                        KeyCode::Char('/') if !app.search_mode => app.begin_action_search(),
                        KeyCode::Backspace if app.search_mode => app.pop_action_query(),
                        KeyCode::Enter if app.search_mode => app.search_mode = false,
                        KeyCode::Char(ch) if app.search_mode => app.push_action_query(ch),
                        KeyCode::PageUp => {
                            scroll_focused_up(app, terminal.size()?.into(), 12);
                        }
                        KeyCode::PageDown => {
                            scroll_focused_down(app, terminal.size()?.into(), 12);
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            scroll_focused_up(app, terminal.size()?.into(), 6);
                        }
                        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            scroll_focused_down(app, terminal.size()?.into(), 6);
                        }
                        KeyCode::End => app.follow_output(),
                        KeyCode::Char('q') if app.command_running() => {
                            app.push_output(
                                OutputStream::System,
                                "A command is running. Press Ctrl+C to stop it before quitting.",
                            );
                        }
                        KeyCode::Char('q') => return Ok(()),
                        KeyCode::Char('r') => app.refresh(),
                        KeyCode::Char('[') => app.show_previous_run(),
                        KeyCode::Char(']') => app.show_next_run(),
                        KeyCode::Char('a') => app.focus_actions(),
                        KeyCode::Char('o') => app.focus_output(),
                        KeyCode::Char('i') => app.cycle_inspector(),
                        KeyCode::Char('m') => app.focus_messages(),
                        KeyCode::Char('f') => app.focus_files(),
                        KeyCode::Char('s') => app.focus_status(),
                        KeyCode::Char('y') => app.copy_selected_output(),
                        KeyCode::Char('v') | KeyCode::Char('V')
                            if app.focus == FocusPane::Output =>
                        {
                            app.enter_output_line_visual_mode();
                        }
                        KeyCode::Enter if app.focus == FocusPane::Output => {
                            app.copy_selected_output();
                        }
                        KeyCode::Enter => run_selected_action(app, terminal.size()?.into())?,
                        KeyCode::Char('K') if app.focus == FocusPane::Output => {
                            app.select_previous_output(true);
                            ensure_selected_output_visible(app, terminal.size()?.into());
                        }
                        KeyCode::Char('J') if app.focus == FocusPane::Output => {
                            app.select_next_output(true);
                            ensure_selected_output_visible(app, terminal.size()?.into());
                        }
                        KeyCode::Up | KeyCode::Char('k') if app.focus == FocusPane::Output => {
                            app.select_previous_output(
                                key.modifiers.contains(KeyModifiers::SHIFT)
                                    || app.output_selection_anchor.is_some(),
                            );
                            ensure_selected_output_visible(app, terminal.size()?.into());
                        }
                        KeyCode::Down | KeyCode::Char('j') if app.focus == FocusPane::Output => {
                            app.select_next_output(
                                key.modifiers.contains(KeyModifiers::SHIFT)
                                    || app.output_selection_anchor.is_some(),
                            );
                            ensure_selected_output_visible(app, terminal.size()?.into());
                        }
                        KeyCode::Char('g') if app.focus == FocusPane::Output => {
                            app.select_first_output(app.output_selection_anchor.is_some());
                            ensure_selected_output_visible(app, terminal.size()?.into());
                        }
                        KeyCode::Char('G') if app.focus == FocusPane::Output => {
                            app.select_last_output(app.output_selection_anchor.is_some());
                            ensure_selected_output_visible(app, terminal.size()?.into());
                        }
                        KeyCode::Up | KeyCode::Char('k') if app.focus == FocusPane::Inspector => {
                            scroll_inspector_up(app, terminal.size()?.into(), 1)
                        }
                        KeyCode::Down | KeyCode::Char('j') if app.focus == FocusPane::Inspector => {
                            scroll_inspector_down(app, terminal.size()?.into(), 1)
                        }
                        KeyCode::Up | KeyCode::Char('k') => select_previous_action(app),
                        KeyCode::Down | KeyCode::Char('j') => select_next_action(app),
                        KeyCode::Char('g') => select_first_action(app),
                        KeyCode::Char('G') => select_last_action(app),
                        KeyCode::Left | KeyCode::BackTab | KeyCode::Char('h') => {
                            focus_previous_pane(app)
                        }
                        KeyCode::Right | KeyCode::Tab | KeyCode::Char('l') => focus_next_pane(app),
                        _ => {}
                    }
                }
                Event::Mouse(mouse) if app.show_help => {
                    if matches!(mouse.kind, MouseEventKind::Down(_)) {
                        app.show_help = false;
                    }
                    if matches!(mouse.kind, MouseEventKind::Down(_) | MouseEventKind::Up(_)) {
                        app.output_mouse_drag_active = false;
                    }
                }
                Event::Mouse(mouse) => handle_mouse(app, terminal.size()?.into(), mouse)?,
                _ => {}
            }
        }
    }
}

fn tui_frame_tick(app: &MonitorApp) -> Duration {
    if app.command_running() || app.effects.is_running() {
        TUI_ANIMATION_TICK
    } else {
        TUI_IDLE_TICK
    }
}

fn select_previous_action(app: &mut MonitorApp) {
    app.focus_commands();
    app.workflow_cursor = app.workflow_cursor.saturating_sub(1);
}

fn select_next_action(app: &mut MonitorApp) {
    app.focus_commands();
    let last = app.workflow_entries().len().saturating_sub(1);
    app.workflow_cursor = (app.workflow_cursor + 1).min(last);
}

fn select_first_action(app: &mut MonitorApp) {
    app.focus_commands();
    app.workflow_cursor = 0;
}

fn select_last_action(app: &mut MonitorApp) {
    app.focus_commands();
    app.workflow_cursor = app.workflow_entries().len().saturating_sub(1);
}

fn focus_previous_pane(app: &mut MonitorApp) {
    app.focus = match app.focus {
        FocusPane::Commands => FocusPane::Output,
        FocusPane::Inspector => FocusPane::Commands,
        FocusPane::Output => FocusPane::Inspector,
    };
}

fn focus_next_pane(app: &mut MonitorApp) {
    app.focus = match app.focus {
        FocusPane::Commands => FocusPane::Inspector,
        FocusPane::Inspector => FocusPane::Output,
        FocusPane::Output => FocusPane::Commands,
    };
}

fn handle_mouse(app: &mut MonitorApp, area: Rect, mouse: MouseEvent) -> Result<()> {
    let layout = dashboard_layout(area);
    if matches!(mouse.kind, MouseEventKind::Down(_) | MouseEventKind::Up(_)) {
        app.output_mouse_drag_active = false;
    }
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            if contains(layout.activity, mouse.column, mouse.row) {
                app.scroll_output_up(4);
                clamp_output_scroll(app, area);
            } else if contains(layout.workflow, mouse.column, mouse.row) {
                select_previous_action(app);
            } else if contains(layout.inspector, mouse.column, mouse.row) {
                scroll_inspector_up(app, area, 3);
            }
        }
        MouseEventKind::ScrollDown => {
            if contains(layout.activity, mouse.column, mouse.row) {
                app.scroll_output_down(4);
                clamp_output_scroll(app, area);
            } else if contains(layout.workflow, mouse.column, mouse.row) {
                select_next_action(app);
            } else if contains(layout.inspector, mouse.column, mouse.row) {
                scroll_inspector_down(app, area, 3);
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if contains(layout.workflow, mouse.column, mouse.row) {
                select_action_at(app, layout.workflow, mouse.column, mouse.row);
            } else if contains(layout.inspector, mouse.column, mouse.row) {
                app.focus_inspector();
            } else if contains(layout.activity, mouse.column, mouse.row) {
                app.output_mouse_drag_active = select_output_at(
                    app,
                    layout.activity,
                    mouse.column,
                    mouse.row,
                    mouse.modifiers.contains(KeyModifiers::SHIFT),
                );
            }
        }
        MouseEventKind::Drag(MouseButton::Left)
            if app.output_mouse_drag_active
                && contains(layout.activity, mouse.column, mouse.row) =>
        {
            select_output_at(app, layout.activity, mouse.column, mouse.row, true);
        }
        MouseEventKind::Down(MouseButton::Right) | MouseEventKind::Down(MouseButton::Middle)
            if contains(layout.activity, mouse.column, mouse.row) =>
        {
            app.follow_output();
        }
        _ => {}
    }
    Ok(())
}

fn dashboard_layout(area: Rect) -> UiLayout {
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(area);
    UiLayout::new(body[1])
}

fn scroll_focused_up(app: &mut MonitorApp, area: Rect, lines: usize) {
    match app.focus {
        FocusPane::Output => {
            app.scroll_output_up(lines);
            clamp_output_scroll(app, area);
        }
        FocusPane::Inspector => scroll_inspector_up(app, area, lines),
        _ => {}
    }
}

fn scroll_focused_down(app: &mut MonitorApp, area: Rect, lines: usize) {
    match app.focus {
        FocusPane::Output => {
            app.scroll_output_down(lines);
            clamp_output_scroll(app, area);
        }
        FocusPane::Inspector => scroll_inspector_down(app, area, lines),
        _ => {}
    }
}

fn scroll_inspector_up(app: &mut MonitorApp, area: Rect, lines: usize) {
    match app.inspector_view {
        InspectorView::Config => app.config_scroll = app.config_scroll.saturating_sub(lines),
        InspectorView::Diagnostics => {
            app.messages_scroll = app.messages_scroll.saturating_sub(lines)
        }
        InspectorView::Artifacts => app.files_scroll = app.files_scroll.saturating_sub(lines),
        InspectorView::Summary => {}
    }
    clamp_inspector_scroll(app, area);
}

fn scroll_inspector_down(app: &mut MonitorApp, area: Rect, lines: usize) {
    match app.inspector_view {
        InspectorView::Config => app.config_scroll = app.config_scroll.saturating_add(lines),
        InspectorView::Diagnostics => {
            app.messages_scroll = app.messages_scroll.saturating_add(lines)
        }
        InspectorView::Artifacts => app.files_scroll = app.files_scroll.saturating_add(lines),
        InspectorView::Summary => {}
    }
    clamp_inspector_scroll(app, area);
}

fn clamp_inspector_scroll(app: &mut MonitorApp, area: Rect) {
    let inspector = dashboard_layout(area).inspector;
    match app.inspector_view {
        InspectorView::Config => {
            app.config_scroll = app.config_scroll.min(config_scroll_max(app, inspector))
        }
        InspectorView::Diagnostics => {
            app.messages_scroll = app.messages_scroll.min(messages_scroll_max(app, inspector))
        }
        InspectorView::Artifacts => {
            app.files_scroll = app.files_scroll.min(files_scroll_max(app, inspector))
        }
        InspectorView::Summary => {}
    }
}

fn config_scroll_max(app: &MonitorApp, area: Rect) -> usize {
    let total = app
        .ready_config()
        .map(|(config, _)| config.channels.len())
        .unwrap_or(0);
    let (_, channels_area) = config_panel_layout(area);
    total.saturating_sub(table_visible_rows(channels_area))
}

fn messages_scroll_max(app: &MonitorApp, area: Rect) -> usize {
    let inner_width = area.width.saturating_sub(2);
    let visible_rows = area.height.saturating_sub(2) as usize;
    message_visual_lines(app, inner_width)
        .len()
        .saturating_sub(visible_rows)
}

fn files_scroll_max(app: &MonitorApp, area: Rect) -> usize {
    let total = artifact_rows(app.ready_config().map(|(config, _)| config)).len();
    total.saturating_sub(table_visible_rows(area))
}

fn table_visible_rows(area: Rect) -> usize {
    area.height.saturating_sub(3).max(1) as usize
}

fn visible_range_title(label: &str, start: usize, end: usize, total: usize) -> String {
    if total == 0 {
        format!(" {label} 0/0 ")
    } else {
        format!(" {label} {}-{end}/{total} ", start + 1)
    }
}

fn select_action_at(app: &mut MonitorApp, area: Rect, column: u16, row: u16) {
    // The panel border/title is part of the focus target, but only an inner row
    // is allowed to change the selected command.
    app.focus_commands();
    let (list_area, _) = workflow_layout(area);
    let inner = bordered_inner(list_area);
    if !contains(inner, column, row) {
        return;
    }
    let selected = app.workflow_cursor;
    let visible_rows = list_area.height.saturating_sub(2).max(1) as usize;
    let start = selected.saturating_sub(visible_rows / 2);
    let row_index = row.saturating_sub(inner.y) as usize;
    let entries_len = app.workflow_entries().len();
    if row_index < visible_rows && start + row_index < entries_len {
        app.workflow_cursor = start + row_index;
    }
}

fn select_output_at(app: &mut MonitorApp, area: Rect, column: u16, row: u16, extend: bool) -> bool {
    app.focus_output();
    if app.visible_output().is_empty() {
        return false;
    }

    let Some(log_content) = output_selectable_area(area) else {
        return false;
    };
    if !contains(log_content, column, row) {
        return false;
    }

    let visible_rows = log_content.height.max(1) as usize;
    let visual_lines = visual_output_lines(app.visible_output(), log_content.width, None, None);
    let max_scroll = visual_lines.len().saturating_sub(visible_rows);
    let scroll = app.run_output_scroll.min(max_scroll);
    let end = visual_lines.len().saturating_sub(scroll);
    let start = end.saturating_sub(visible_rows);
    let row_index = row.saturating_sub(log_content.y) as usize;
    if let Some(line) = visual_lines.get(start + row_index) {
        app.set_output_selection(line.entry_index, extend);
        true
    } else {
        false
    }
}

fn output_selectable_area(area: Rect) -> Option<Rect> {
    let sections = output_inner_layout(bordered_inner(area));
    let log_area = sections.log;
    if log_area.height <= 1 || log_area.width <= 1 {
        return None;
    }
    Some(Rect {
        x: log_area.x,
        y: log_area.y.saturating_add(1),
        width: log_area.width.saturating_sub(1),
        height: log_area.height.saturating_sub(1),
    })
}

fn ensure_selected_output_visible(app: &mut MonitorApp, area: Rect) {
    let Some(selected) = app.output_selected else {
        return;
    };
    if app.visible_output().is_empty() {
        return;
    }

    let Some(log_content) = output_log_content_area(area) else {
        return;
    };
    let visible_rows = log_content.height.max(1) as usize;
    let visual_lines = visual_output_lines(
        app.visible_output(),
        log_content.width.saturating_sub(1),
        None,
        None,
    );
    let total = visual_lines.len();
    if total == 0 {
        return;
    }
    let max_scroll = total.saturating_sub(visible_rows);
    let scroll = app.run_output_scroll.min(max_scroll);
    let end = total.saturating_sub(scroll);
    let start = end.saturating_sub(visible_rows);

    let Some((selected_start, selected_end)) = visual_entry_range(&visual_lines, selected) else {
        return;
    };

    app.run_output_scroll = if selected_start < start {
        max_scroll.saturating_sub(selected_start)
    } else if selected_end >= end {
        total.saturating_sub(selected_end + 1).min(max_scroll)
    } else {
        scroll
    };
}

fn effective_output_scroll(app: &MonitorApp, log_area: Rect, visual_line_count: usize) -> usize {
    let max_scroll = visual_line_count.saturating_sub(output_visible_rows(log_area));
    app.run_output_scroll.min(max_scroll)
}

fn max_output_scroll_for_area(app: &MonitorApp, area: Rect) -> Option<usize> {
    let log_content = output_log_content_area(area)?;
    let visual_line_count =
        visual_output_line_count(app.visible_output(), log_content.width.saturating_sub(1));
    Some(visual_line_count.saturating_sub(log_content.height.max(1) as usize))
}

fn clamp_output_scroll(app: &mut MonitorApp, area: Rect) {
    if app.visible_output().is_empty() {
        app.run_output_scroll = 0;
        return;
    }

    if let Some(max_scroll) = max_output_scroll_for_area(app, area) {
        app.run_output_scroll = app.run_output_scroll.min(max_scroll);
        if app.run_output_scroll == 0 {
            app.new_output_events = 0;
        }
    }
}

fn output_table_width_for_area(area: Rect) -> Option<u16> {
    output_log_content_area(area).map(|log_content| {
        log_content
            .width
            .saturating_sub(OUTPUT_PREFIX_WIDTH + 1)
            .clamp(24, 160)
    })
}

fn output_log_content_area(area: Rect) -> Option<Rect> {
    let layout = dashboard_layout(area);
    if layout.activity.width == 0 || layout.activity.height == 0 {
        return None;
    }

    let inner = bordered_inner(layout.activity);
    let sections = output_inner_layout(inner);
    let log_area = sections.log;
    if log_area.height <= 1 || log_area.width == 0 {
        return None;
    }

    Some(Rect {
        x: log_area.x,
        y: log_area.y.saturating_add(1),
        width: log_area.width,
        height: log_area.height.saturating_sub(1),
    })
}

fn visual_entry_range(lines: &[VisualOutputLine], entry_index: usize) -> Option<(usize, usize)> {
    let start = lines
        .iter()
        .position(|line| line.entry_index == entry_index)?;
    let end = lines
        .iter()
        .rposition(|line| line.entry_index == entry_index)?;
    Some((start, end))
}

fn visual_selection_range(
    lines: &[VisualOutputLine],
    selection: Option<(usize, usize)>,
) -> Option<(usize, usize)> {
    let (start_entry, end_entry) = selection?;
    let start = lines
        .iter()
        .position(|line| (start_entry..=end_entry).contains(&line.entry_index))?;
    let end = lines
        .iter()
        .rposition(|line| (start_entry..=end_entry).contains(&line.entry_index))?;
    Some((start, end))
}

fn output_selection_status(selected_visual_range: Option<(usize, usize)>) -> String {
    selected_visual_range
        .map(|(start, end)| {
            if start == end {
                format!("selected {}", start + 1)
            } else {
                format!(
                    "selected {}-{} / {} lines",
                    start + 1,
                    end + 1,
                    end - start + 1
                )
            }
        })
        .unwrap_or_else(|| "select --".to_string())
}

fn run_selected_action(app: &mut MonitorApp, area: Rect) -> Result<()> {
    let action = match app.selected_workflow_entry() {
        Some(WorkflowEntry::Group(_)) => {
            app.toggle_selected_group();
            return Ok(());
        }
        Some(WorkflowEntry::Action(action)) => action,
        None => {
            app.push_output(
                OutputStream::System,
                "No workflow action matches the search.",
            );
            return Ok(());
        }
    };
    if app.command_running() {
        app.push_output(
            OutputStream::System,
            "A command is already running. Wait for it to finish before starting another.",
        );
        return Ok(());
    }

    if let Err(reason) = action_readiness(action, &app.load) {
        app.history_view = None;
        app.run_output.clear();
        app.output_selected = None;
        app.output_selection_anchor = None;
        app.run_output_scroll = 0;
        app.last_run = Some(RunRecord {
            action,
            label: action.label(),
            elapsed: Duration::ZERO,
            result: reason.clone(),
            ok: false,
        });
        app.push_output(
            OutputStream::Stderr,
            &format!("{} is blocked: {reason}", action.label()),
        );
        app.archive_last_run();
        return Ok(());
    }

    let table_width = output_table_width_for_area(area);
    let handle = spawn_command_runner(action, app.config_path.clone(), table_width)?;
    app.history_view = None;
    app.run_output.clear();
    app.last_stderr_kind = None;
    app.output_selected = None;
    app.output_selection_anchor = None;
    app.copy_status = None;
    app.follow_output();
    let command_args = action.command_args();
    app.push_output(
        OutputStream::System,
        &format!(
            "pmoke --config {} {}",
            app.config_path,
            command_args.join(" ")
        ),
    );
    app.push_output(OutputStream::System, action.description());
    app.active_run = Some(ActiveRun {
        action,
        label: action.label(),
        started_at: Instant::now(),
        receiver: handle.receiver,
        cancel: handle.cancel,
        cancel_requested: false,
    });
    Ok(())
}

fn spawn_command_runner(
    action: MonitorAction,
    config_path: String,
    table_width: Option<u16>,
) -> Result<RunHandle> {
    let exe = std::env::current_exe()?;
    let command_args = action.command_args();
    // Keep verbose child output from growing without bound if rendering is
    // briefly delayed. Readers naturally apply backpressure at this boundary.
    let (tx, rx) = mpsc::sync_channel(512);
    let (cancel_tx, cancel_rx) = mpsc::channel();

    thread::spawn(move || {
        let mut command = ProcessCommand::new(exe);
        command
            .arg("--config")
            .arg(config_path)
            .args(command_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command.env("PMOKE_OUTPUT", "jsonl");
        command.env("PMOKE_STAGE", action.command_name());
        if let Some(width) = table_width {
            command.env("PMOKE_TABLE_WIDTH", width.to_string());
        }
        let spawn_result = command.spawn();

        let mut child = match spawn_result {
            Ok(child) => child,
            Err(err) => {
                let _ = tx.send(RunEvent::Failed(format!("failed to spawn pmoke: {err}")));
                return;
            }
        };

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let stdout_reader =
            stdout.map(|stream| spawn_stream_reader(stream, OutputStream::Stdout, tx.clone()));
        let stderr_reader =
            stderr.map(|stream| spawn_stream_reader(stream, OutputStream::Stderr, tx.clone()));

        let mut cancelled_by = None;
        let wait_result = loop {
            match cancel_rx.try_recv() {
                Ok(reason) => {
                    cancelled_by = Some(reason);
                    break stop_child(&mut child, reason, &tx);
                }
                Err(TryRecvError::Disconnected) => {
                    cancelled_by = Some(CancelReason::Closed);
                    break stop_child(&mut child, CancelReason::Closed, &tx);
                }
                Err(TryRecvError::Empty) => {}
            }

            match child.try_wait() {
                Ok(Some(status)) => break Ok(status),
                Ok(None) => thread::sleep(Duration::from_millis(30)),
                Err(err) => break Err(err),
            }
        };
        if let Some(reader) = stdout_reader {
            let _ = reader.join();
        }
        if let Some(reader) = stderr_reader {
            let _ = reader.join();
        }

        match wait_result {
            Ok(status) => {
                let ok = cancelled_by.is_none() && status.success();
                let status_text = if let Some(reason) = cancelled_by {
                    format!("stopped by {}", reason.label())
                } else {
                    status
                        .code()
                        .map(|code| format!("exited with code {code}"))
                        .unwrap_or_else(|| "terminated by signal".to_string())
                };
                let _ = tx.send(RunEvent::Finished {
                    ok,
                    status: status_text,
                });
            }
            Err(err) => {
                let _ = tx.send(RunEvent::Failed(format!("failed to wait for pmoke: {err}")));
            }
        }
    });

    Ok(RunHandle {
        receiver: rx,
        cancel: cancel_tx,
    })
}

fn stop_child(
    child: &mut Child,
    reason: CancelReason,
    tx: &SyncSender<RunEvent>,
) -> io::Result<ExitStatus> {
    if let Err(err) = interrupt_child(child) {
        let _ = tx.send(RunEvent::Output(
            OutputStream::Stderr,
            format!(
                "failed to send {} to pmoke: {err}; killing command",
                reason.label()
            ),
        ));
        child.kill()?;
        return child.wait();
    }

    let deadline = Instant::now() + Duration::from_millis(700);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            let _ = tx.send(RunEvent::Output(
                OutputStream::Stderr,
                format!("pmoke ignored {}; killing command", reason.label()),
            ));
            child.kill()?;
            return child.wait();
        }
        thread::sleep(Duration::from_millis(30));
    }
}

#[cfg(unix)]
fn interrupt_child(child: &mut Child) -> io::Result<()> {
    unsafe extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
    }
    const SIGINT: i32 = 2;
    let rc = unsafe { kill(child.id() as i32, SIGINT) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(unix))]
fn interrupt_child(child: &mut Child) -> io::Result<()> {
    child.kill()
}

fn spawn_stream_reader<R: Read + Send + 'static>(
    mut stream: R,
    kind: OutputStream,
    tx: SyncSender<RunEvent>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut read_buf = [0_u8; 4096];
        let mut record = Vec::new();
        let mut previous_was_cr = false;

        let emit = |bytes: &[u8], progress: bool| {
            let text = String::from_utf8_lossy(bytes).into_owned();
            let event = if !progress
                && kind == OutputStream::Stdout
                && let Some(event) = crate::ui::parse_jsonl_event(&text)
            {
                RunEvent::Structured(event)
            } else if progress {
                RunEvent::Progress(kind, text)
            } else {
                RunEvent::Output(kind, text)
            };
            tx.send(event)
        };

        loop {
            match stream.read(&mut read_buf) {
                Ok(0) => {
                    if !record.is_empty() {
                        let _ = emit(&record, previous_was_cr);
                    }
                    break;
                }
                Ok(read) => {
                    for &byte in &read_buf[..read] {
                        if previous_was_cr {
                            if byte == b'\n' {
                                if emit(&record, false).is_err() {
                                    return;
                                }
                                record.clear();
                                previous_was_cr = false;
                                continue;
                            }
                            if emit(&record, true).is_err() {
                                return;
                            }
                            record.clear();
                            previous_was_cr = false;
                        }
                        match byte {
                            b'\r' => {
                                previous_was_cr = true;
                            }
                            b'\n' => {
                                if emit(&record, false).is_err() {
                                    return;
                                }
                                record.clear();
                            }
                            _ => {
                                previous_was_cr = false;
                                record.push(byte);
                            }
                        }
                    }
                }
                Err(err) => {
                    if !record.is_empty() {
                        let _ = emit(&record, false);
                    }
                    let _ = tx.send(RunEvent::Output(
                        OutputStream::Stderr,
                        format!("failed to read command output: {err}"),
                    ));
                    break;
                }
            }
        }
    })
}

#[cfg(test)]
#[path = "tests/mod.rs"]
mod tests;
