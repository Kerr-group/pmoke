use crate::config::{self, Config, ConfigDiagnostics, ConfigLoad, ConfigWarning, FetchOutput};
use crate::constants::{
    FETCHED_FNAME, KERR_NAME, LI_RESULTS_NAME, LI_ROTATED_NAME, RAW_METADATA_FNAME,
    RAW_WAVEFORM_DIR,
};
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
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    prelude::{Color, Line, Modifier, Span, Style},
    symbols,
    widgets::{
        Block, BorderType, Borders, Clear, List, ListItem, Paragraph, Row, Table, Tabs, Wrap,
    },
};
use std::{
    fs,
    io::{self, Read, Stdout},
    process::{Child, Command as ProcessCommand, ExitStatus, Stdio},
    sync::mpsc::{self, Receiver, Sender, TryRecvError},
    thread,
    time::{Duration, Instant, SystemTime},
};
use tachyonfx::{CellFilter, Duration as FxDuration, EffectManager, Interpolation, Motion, fx};
use tui_spinner::FluxFrames;

mod actions;
mod clipboard;
mod formatting;
mod layout;
mod timeline;

use actions::{MonitorAction, action_runnable, monitor_actions};
#[cfg(test)]
use clipboard::base64_encode;
use clipboard::{ClipboardMethod, copy_text_to_clipboard};
use formatting::{
    bordered_inner, centered_rect, centered_text, contains, fit_path, fit_text, format_age,
    format_duration, format_live_duration, percent_width, strip_ansi_codes,
};
use layout::{
    UiLayout, actions_full_layout, active_panel_layout, command_palette_layout,
    latest_event_feed_effect_area, output_inner_layout, output_visible_rows,
};
#[cfg(test)]
use timeline::{
    StageProgressState, TimelineStep, TimelineStepState, timeline_badge_cell, timeline_for_action,
    timeline_separator, timeline_step_spans,
};
use timeline::{render_run_timeline, spinner_frame, timeline_motion_frame};

const TUI_IDLE_TICK: Duration = Duration::from_millis(150);
const TUI_ANIMATION_TICK: Duration = Duration::from_millis(16);
const OUTPUT_PREFIX_WIDTH: u16 = 12;
const EVENT_BADGE_WIDTH: usize = 6;
const TIMELINE_BADGE_WIDTH: usize = 5;
const EVENT_FEED_EFFECT_MS: u32 = 520;

pub fn monitor(config_path: &str, load: ConfigLoad) -> Result<()> {
    let mut terminal = setup_terminal()?;
    let mut app = MonitorApp::new(config_path.to_string(), load);
    let result = run(&mut terminal, &mut app);
    restore_terminal(&mut terminal)?;
    result
}

struct MonitorApp {
    config_path: String,
    load: ConfigLoad,
    started_at: Instant,
    last_refresh: SystemTime,
    active_tab: usize,
    focus: FocusPane,
    selected_action: usize,
    last_run: Option<RunRecord>,
    active_run: Option<ActiveRun>,
    run_output: Vec<LogEntry>,
    run_output_scroll: usize,
    output_selected: Option<usize>,
    output_selection_anchor: Option<usize>,
    copy_status: Option<String>,
    show_help: bool,
    effects: EffectManager<MonitorEffect>,
    last_effect_frame: Instant,
}

impl MonitorApp {
    fn new(config_path: String, load: ConfigLoad) -> Self {
        Self {
            config_path,
            load,
            started_at: Instant::now(),
            last_refresh: SystemTime::now(),
            active_tab: 0,
            focus: FocusPane::Commands,
            selected_action: 0,
            last_run: None,
            active_run: None,
            run_output: Vec::new(),
            run_output_scroll: 0,
            output_selected: None,
            output_selection_anchor: None,
            copy_status: None,
            show_help: false,
            effects: EffectManager::default(),
            last_effect_frame: Instant::now(),
        }
    }

    fn refresh(&mut self) {
        self.load = config::load_from_path(&self.config_path);
        self.last_refresh = SystemTime::now();
    }

    fn status(&self) -> (&'static str, Color) {
        match self.load {
            ConfigLoad::Ready { .. } => ("RUNNABLE", Color::Green),
            ConfigLoad::Diagnostics(_) => ("BLOCKED", Color::Red),
        }
    }

    fn elapsed(&self) -> String {
        let elapsed = self.started_at.elapsed();
        let mins = elapsed.as_secs() / 60;
        let secs = elapsed.as_secs() % 60;
        format!("{mins:02}:{secs:02}")
    }

    fn ready_config(&self) -> Option<(&Config, &[ConfigWarning])> {
        match &self.load {
            ConfigLoad::Ready { config, warnings } => Some((config, warnings)),
            ConfigLoad::Diagnostics(_) => None,
        }
    }

    fn diagnostics(&self) -> Option<&ConfigDiagnostics> {
        match &self.load {
            ConfigLoad::Diagnostics(diag) => Some(diag),
            ConfigLoad::Ready { .. } => None,
        }
    }

    fn actions(&self) -> Vec<MonitorAction> {
        monitor_actions()
    }

    fn selected_action(&self) -> MonitorAction {
        let actions = self.actions();
        actions
            .get(self.selected_action.min(actions.len().saturating_sub(1)))
            .copied()
            .unwrap_or(MonitorAction::Show)
    }

    fn poll_command(&mut self) {
        let Some(run) = &self.active_run else {
            return;
        };

        let mut events = Vec::new();
        loop {
            match run.receiver.try_recv() {
                Ok(event) => events.push(event),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    events.push(RunEvent::Failed("command runner disconnected".to_string()));
                    break;
                }
            }
        }

        for event in events {
            match event {
                RunEvent::Output(stream, text) => self.push_output(stream, &text),
                RunEvent::Finished { ok, status } => self.finish_run(ok, status),
                RunEvent::Failed(message) => self.finish_run(false, message),
            }
        }
    }

    fn command_running(&self) -> bool {
        self.active_run.is_some()
    }

    fn cancel_command(&mut self, reason: CancelReason) {
        let Some(run) = &mut self.active_run else {
            return;
        };
        if run.cancel_requested {
            return;
        }
        run.cancel_requested = true;
        let _ = run.cancel.send(reason);
        self.push_output(
            OutputStream::System,
            &format!("Stopping command via {}...", reason.label()),
        );
    }

    fn interrupt_current_operation(&mut self) {
        if self.show_help {
            self.show_help = false;
        }

        if self.command_running() {
            self.cancel_command(CancelReason::CtrlC);
            return;
        }

        if self.output_selection_anchor.take().is_some() {
            self.copy_status = Some("selection cleared".to_string());
            return;
        }

        if self.focus == FocusPane::Output {
            self.focus_commands();
            self.copy_status = None;
        }
    }

    fn escape_current_mode(&mut self) {
        if self.show_help {
            self.show_help = false;
            return;
        }

        if self.output_selection_anchor.take().is_some() {
            self.copy_status = Some("selection cleared".to_string());
            return;
        }

        if self.focus == FocusPane::Output {
            self.focus_commands();
            self.copy_status = None;
        }
    }

    fn push_output(&mut self, stream: OutputStream, text: &str) {
        let mut appended = 0;
        for line in text.replace('\r', "\n").split('\n') {
            if line.is_empty() {
                continue;
            }
            self.run_output.push(LogEntry {
                stream,
                text: line.to_string(),
            });
            appended += 1;
        }
        if self.run_output_scroll > 0 {
            self.run_output_scroll += appended;
        }
        const MAX_LOG_LINES: usize = 600;
        if self.run_output.len() > MAX_LOG_LINES {
            let extra = self.run_output.len() - MAX_LOG_LINES;
            self.run_output.drain(0..extra);
            self.run_output_scroll = self.run_output_scroll.saturating_sub(extra);
            self.output_selected = shift_log_index_after_drain(self.output_selected, extra);
            self.output_selection_anchor =
                shift_log_index_after_drain(self.output_selection_anchor, extra);
        }
        if self.run_output.is_empty() {
            self.output_selected = None;
            self.output_selection_anchor = None;
        }
        if appended > 0 {
            self.trigger_event_feed_effect();
        }
    }

    fn trigger_event_feed_effect(&mut self) {
        let effect = fx::parallel(&[
            fx::sweep_in(
                Motion::LeftToRight,
                10,
                0,
                Color::Black,
                (EVENT_FEED_EFFECT_MS, Interpolation::SineOut),
            )
            .with_filter(CellFilter::Text),
            fx::fade_from_fg(
                Color::LightCyan,
                (EVENT_FEED_EFFECT_MS, Interpolation::QuadOut),
            )
            .with_filter(CellFilter::Text),
        ]);
        self.effects
            .add_unique_effect(MonitorEffect::EventFeedLatest, effect);
    }

    fn effect_delta(&mut self) -> FxDuration {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last_effect_frame);
        self.last_effect_frame = now;
        fx_duration(elapsed)
    }

    fn scroll_output_up(&mut self, lines: usize) {
        self.run_output_scroll = self.run_output_scroll.saturating_add(lines);
    }

    fn scroll_output_down(&mut self, lines: usize) {
        self.run_output_scroll = self.run_output_scroll.saturating_sub(lines);
    }

    fn follow_output(&mut self) {
        self.run_output_scroll = 0;
    }

    fn focus_actions(&mut self) {
        self.active_tab = 0;
        self.focus = FocusPane::Commands;
    }

    fn focus_status(&mut self) {
        self.active_tab = 0;
        self.focus = FocusPane::Status;
    }

    fn focus_output(&mut self) {
        self.active_tab = 0;
        self.focus = FocusPane::Output;
        if self.output_selected.is_none() && !self.run_output.is_empty() {
            self.output_selected = Some(self.run_output.len() - 1);
        }
    }

    fn focus_commands(&mut self) {
        self.focus_actions();
    }

    fn focus_messages(&mut self) {
        self.active_tab = 2;
        self.focus = FocusPane::Messages;
    }

    fn focus_files(&mut self) {
        self.active_tab = 3;
        self.focus = FocusPane::Files;
    }

    fn select_previous_output(&mut self, extend: bool) {
        self.focus_output();
        let Some(selected) = self.output_selected else {
            return;
        };
        self.set_output_selection(selected.saturating_sub(1), extend);
    }

    fn select_next_output(&mut self, extend: bool) {
        self.focus_output();
        let Some(selected) = self.output_selected else {
            return;
        };
        let last = self.run_output.len().saturating_sub(1);
        self.set_output_selection((selected + 1).min(last), extend);
    }

    fn select_first_output(&mut self, extend: bool) {
        self.focus_output();
        if self.run_output.is_empty() {
            return;
        }
        self.set_output_selection(0, extend);
    }

    fn select_last_output(&mut self, extend: bool) {
        self.focus_output();
        if self.run_output.is_empty() {
            return;
        }
        self.set_output_selection(self.run_output.len() - 1, extend);
        self.follow_output();
    }

    fn enter_output_line_visual_mode(&mut self) {
        self.focus_output();
        if self.output_selected.is_none() {
            return;
        }
        self.output_selection_anchor = self.output_selected;
    }

    fn set_output_selection(&mut self, index: usize, extend: bool) {
        self.focus_output();
        if extend {
            if self.output_selection_anchor.is_none() {
                self.output_selection_anchor = self.output_selected.or(Some(index));
            }
        } else {
            self.output_selection_anchor = None;
        }
        self.output_selected = Some(index.min(self.run_output.len().saturating_sub(1)));
        self.scroll_selected_output_into_view();
    }

    fn selected_output_text(&self) -> Option<String> {
        let (start, end) = self.output_selection_range()?;
        Some(
            self.run_output
                .get(start..=end)?
                .iter()
                .map(|entry| strip_ansi_codes(&entry.text))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    fn output_selection_range(&self) -> Option<(usize, usize)> {
        let cursor = self.output_selected?;
        let anchor = self.output_selection_anchor.unwrap_or(cursor);
        Some((anchor.min(cursor), anchor.max(cursor)))
    }

    fn output_selection_line_count(&self) -> usize {
        self.output_selection_range()
            .map(|(start, end)| end - start + 1)
            .unwrap_or(0)
    }

    fn finish_output_yank(&mut self, text_len: usize, method: ClipboardMethod) {
        let lines = self.output_selection_line_count();
        self.copy_status = Some(format!(
            "copied {lines} lines / {text_len} chars via {}",
            method.label()
        ));
        self.output_selection_anchor = None;
    }

    fn scroll_selected_output_into_view(&mut self) {
        let Some(selected) = self.output_selected else {
            return;
        };
        let total_after_selected = self.run_output.len().saturating_sub(selected + 1);
        self.run_output_scroll = self.run_output_scroll.max(total_after_selected);
    }

    fn copy_selected_output(&mut self) {
        self.focus_output();
        let Some(text) = self.selected_output_text() else {
            self.copy_status = Some("nothing selected".to_string());
            return;
        };

        match copy_text_to_clipboard(&text) {
            Ok(method) => self.finish_output_yank(text.chars().count(), method),
            Err(err) => {
                self.copy_status = Some(format!("copy failed: {err}"));
            }
        }
    }

    fn finish_run(&mut self, ok: bool, result: String) {
        let Some(run) = self.active_run.take() else {
            return;
        };
        let elapsed = run.started_at.elapsed();
        self.push_output(
            if ok {
                OutputStream::System
            } else {
                OutputStream::Stderr
            },
            &format!("{} in {}", result, format_duration(elapsed)),
        );
        self.last_run = Some(RunRecord {
            action: run.action,
            label: run.label,
            elapsed,
            result,
            ok,
        });
        self.refresh();
    }
}

fn shift_log_index_after_drain(index: Option<usize>, drained: usize) -> Option<usize> {
    index.and_then(|idx| idx.checked_sub(drained))
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

struct RunRecord {
    action: MonitorAction,
    label: &'static str,
    elapsed: Duration,
    result: String,
    ok: bool,
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
    Status,
    Output,
    Messages,
    Files,
}

struct LogEntry {
    stream: OutputStream,
    text: String,
}

enum RunEvent {
    Output(OutputStream, String),
    Finished { ok: bool, status: String },
    Failed(String),
}

#[derive(Clone, Copy)]
enum OutputStream {
    Stdout,
    Stderr,
    System,
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let terminal = Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Fullscreen,
        },
    )?;
    Ok(terminal)
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<()> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
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
                        KeyCode::Char('?') => app.show_help = !app.show_help,
                        KeyCode::Esc => app.escape_current_mode(),
                        KeyCode::Char('q') if app.show_help => {
                            app.show_help = false;
                        }
                        _ if app.show_help => {}
                        KeyCode::PageUp => {
                            app.scroll_output_up(12);
                            clamp_output_scroll(app, terminal.size()?.into());
                        }
                        KeyCode::PageDown => {
                            app.scroll_output_down(12);
                            clamp_output_scroll(app, terminal.size()?.into());
                        }
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.scroll_output_up(6);
                            clamp_output_scroll(app, terminal.size()?.into());
                        }
                        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.scroll_output_down(6);
                            clamp_output_scroll(app, terminal.size()?.into());
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
                        KeyCode::Char('a') => app.focus_actions(),
                        KeyCode::Char('o') => app.focus_output(),
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
                        KeyCode::Up | KeyCode::Char('k') => select_previous_action(app),
                        KeyCode::Down | KeyCode::Char('j') => select_next_action(app),
                        KeyCode::Char('g') => select_first_action(app),
                        KeyCode::Char('G') => select_last_action(app),
                        KeyCode::Left | KeyCode::BackTab | KeyCode::Char('h') => {
                            select_previous_tab(app)
                        }
                        KeyCode::Right | KeyCode::Tab | KeyCode::Char('l') => select_next_tab(app),
                        _ => {}
                    }
                }
                Event::Mouse(mouse) if app.show_help => {
                    if matches!(mouse.kind, MouseEventKind::Down(_)) {
                        app.show_help = false;
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
    app.selected_action = app.selected_action.saturating_sub(1);
    app.active_tab = 0;
}

fn select_next_action(app: &mut MonitorApp) {
    app.focus_commands();
    let last = app.actions().len().saturating_sub(1);
    app.selected_action = (app.selected_action + 1).min(last);
    app.active_tab = 0;
}

fn select_first_action(app: &mut MonitorApp) {
    app.focus_commands();
    app.selected_action = 0;
    app.active_tab = 0;
}

fn select_last_action(app: &mut MonitorApp) {
    app.focus_commands();
    app.selected_action = app.actions().len().saturating_sub(1);
    app.active_tab = 0;
}

fn select_previous_tab(app: &mut MonitorApp) {
    app.active_tab = app.active_tab.saturating_sub(1);
}

fn select_next_tab(app: &mut MonitorApp) {
    app.active_tab = (app.active_tab + 1).min(3);
}

fn handle_mouse(app: &mut MonitorApp, area: Rect, mouse: MouseEvent) -> Result<()> {
    let layout = UiLayout::new(area, app.active_tab);
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            if contains(layout.run_output, mouse.column, mouse.row) {
                app.scroll_output_up(4);
                clamp_output_scroll(app, area);
            } else if contains(layout.command_palette, mouse.column, mouse.row) {
                select_previous_action(app);
            } else {
                app.scroll_output_up(3);
                clamp_output_scroll(app, area);
            }
        }
        MouseEventKind::ScrollDown => {
            if contains(layout.run_output, mouse.column, mouse.row) {
                app.scroll_output_down(4);
                clamp_output_scroll(app, area);
            } else if contains(layout.command_palette, mouse.column, mouse.row) {
                select_next_action(app);
            } else {
                app.scroll_output_down(3);
                clamp_output_scroll(app, area);
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            if contains(layout.tabs, mouse.column, mouse.row) {
                select_tab_at(app, layout.tabs, mouse.column);
            } else if contains(layout.command_palette, mouse.column, mouse.row) {
                select_action_at(app, layout.command_palette, mouse.row);
            } else if contains(layout.run_output, mouse.column, mouse.row) {
                app.active_tab = 0;
                select_output_at(
                    app,
                    layout.run_output,
                    mouse.row,
                    mouse.modifiers.contains(KeyModifiers::SHIFT),
                );
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if contains(layout.run_output, mouse.column, mouse.row) {
                app.active_tab = 0;
                select_output_at(app, layout.run_output, mouse.row, true);
            }
        }
        MouseEventKind::Down(MouseButton::Right) | MouseEventKind::Down(MouseButton::Middle)
            if contains(layout.run_output, mouse.column, mouse.row) =>
        {
            app.follow_output();
        }
        _ => {}
    }
    Ok(())
}

fn select_action_at(app: &mut MonitorApp, area: Rect, row: u16) {
    let selected = app.selected_action;
    let visible_rows = area.height.saturating_sub(2).max(1) as usize;
    let start = selected.saturating_sub(visible_rows / 2);
    let row_index = row.saturating_sub(area.y + 1) as usize;
    let actions_len = app.actions().len();
    if row_index < visible_rows && start + row_index < actions_len {
        app.focus_commands();
        app.selected_action = start + row_index;
        app.active_tab = 0;
    }
}

fn select_output_at(app: &mut MonitorApp, area: Rect, row: u16, extend: bool) {
    app.focus_output();
    if app.run_output.is_empty() {
        return;
    }

    let inner = bordered_inner(area);
    let sections = output_inner_layout(inner);
    let log_area = sections.log;
    if log_area.height <= 1 || row <= log_area.y {
        return;
    }

    let log_content = Rect {
        x: log_area.x,
        y: log_area.y.saturating_add(1),
        width: log_area.width,
        height: log_area.height.saturating_sub(1),
    };
    if !contains(log_content, log_content.x, row) {
        return;
    }

    let visible_rows = log_content.height.max(1) as usize;
    let visual_lines = visual_output_lines(
        &app.run_output,
        log_content.width.saturating_sub(1),
        None,
        None,
    );
    let max_scroll = visual_lines.len().saturating_sub(visible_rows);
    let scroll = app.run_output_scroll.min(max_scroll);
    let end = visual_lines.len().saturating_sub(scroll);
    let start = end.saturating_sub(visible_rows);
    let row_index = row.saturating_sub(log_content.y) as usize;
    if let Some(line) = visual_lines.get(start + row_index) {
        app.set_output_selection(line.entry_index, extend);
    }
}

fn ensure_selected_output_visible(app: &mut MonitorApp, area: Rect) {
    let Some(selected) = app.output_selected else {
        return;
    };
    if app.run_output.is_empty() {
        return;
    }

    let Some(log_content) = output_log_content_area(app, area) else {
        return;
    };
    let visible_rows = log_content.height.max(1) as usize;
    let visual_lines = visual_output_lines(
        &app.run_output,
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
    let log_content = output_log_content_area(app, area)?;
    let visual_line_count =
        visual_output_line_count(&app.run_output, log_content.width.saturating_sub(1));
    Some(visual_line_count.saturating_sub(log_content.height.max(1) as usize))
}

fn clamp_output_scroll(app: &mut MonitorApp, area: Rect) {
    if app.run_output.is_empty() {
        app.run_output_scroll = 0;
        return;
    }

    if let Some(max_scroll) = max_output_scroll_for_area(app, area) {
        app.run_output_scroll = app.run_output_scroll.min(max_scroll);
    }
}

fn output_table_width_for_area(app: &MonitorApp, area: Rect) -> Option<u16> {
    output_log_content_area(app, area).map(|log_content| {
        log_content
            .width
            .saturating_sub(OUTPUT_PREFIX_WIDTH + 1)
            .clamp(24, 160)
    })
}

fn output_log_content_area(app: &MonitorApp, area: Rect) -> Option<Rect> {
    let layout = UiLayout::new(area, app.active_tab);
    if layout.run_output.width == 0 || layout.run_output.height == 0 {
        return None;
    }

    let inner = bordered_inner(layout.run_output);
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

fn select_tab_at(app: &mut MonitorApp, area: Rect, column: u16) {
    let inner_x = area.x + 1;
    let inner_width = area.width.saturating_sub(2);
    if inner_width == 0 || column < inner_x {
        return;
    }
    let tab_width = (inner_width / 4).max(1);
    let idx = ((column - inner_x) / tab_width).min(3);
    app.active_tab = idx as usize;
}

fn run_selected_action(app: &mut MonitorApp, area: Rect) -> Result<()> {
    let action = app.selected_action();
    if app.command_running() {
        app.push_output(
            OutputStream::System,
            "A command is already running. Wait for it to finish before starting another.",
        );
        return Ok(());
    }

    if !action_runnable(action, &app.load) {
        app.last_run = Some(RunRecord {
            action,
            label: action.label(),
            elapsed: Duration::ZERO,
            result: "not runnable with the current configuration or artifacts".to_string(),
            ok: false,
        });
        app.push_output(
            OutputStream::Stderr,
            &format!(
                "{} is not runnable with current config/artifacts",
                action.label()
            ),
        );
        return Ok(());
    }

    app.active_tab = 0;
    let table_width = output_table_width_for_area(app, area);
    let handle = spawn_command_runner(action, app.config_path.clone(), table_width)?;
    app.run_output.clear();
    app.output_selected = None;
    app.output_selection_anchor = None;
    app.copy_status = None;
    app.follow_output();
    app.push_output(
        OutputStream::System,
        &format!(
            "pmoke --config {} {}",
            app.config_path,
            action.command_name()
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
    let command_name = action.command_name().to_string();
    let (tx, rx) = mpsc::channel();
    let (cancel_tx, cancel_rx) = mpsc::channel();

    thread::spawn(move || {
        let mut command = ProcessCommand::new(exe);
        command
            .arg("--config")
            .arg(config_path)
            .arg(command_name)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
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
    tx: &Sender<RunEvent>,
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
    tx: Sender<RunEvent>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buf = [0; 4096];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    let text = String::from_utf8_lossy(&buf[..n]).to_string();
                    if tx.send(RunEvent::Output(kind, text)).is_err() {
                        break;
                    }
                }
                Err(err) => {
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

fn render(frame: &mut Frame<'_>, app: &mut MonitorApp, effect_delta: FxDuration) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(10),
            Constraint::Length(0),
        ])
        .split(area);

    render_header(frame, app, outer[0]);
    render_body(frame, app, outer[1], effect_delta);

    if app.show_help {
        render_help_overlay(frame, app, area);
    }
}

fn render_header(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);
    let header =
        Paragraph::new(Line::from(header_spans(app, area.width))).alignment(Alignment::Left);
    frame.render_widget(header, chunks[0]);

    let rule = "━".repeat(area.width as usize);
    frame.render_widget(
        Paragraph::new(Line::styled(rule, Style::default().fg(Color::DarkGray))),
        chunks[1],
    );
}

fn header_spans(app: &MonitorApp, width: u16) -> Vec<Span<'static>> {
    let (status, color) = app.status();
    let run = run_label(app);
    let config_width = width.saturating_sub(66) as usize;
    vec![
        Span::styled(
            " pMOKE ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled("●", Style::default().fg(color)),
        Span::raw(" "),
        Span::styled(
            status,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  │  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            run,
            Style::default()
                .fg(run_status_color(app))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  │  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            fit_text(&app.config_path, config_width),
            Style::default().fg(Color::Gray),
        ),
        Span::styled("  │  ", Style::default().fg(Color::DarkGray)),
        Span::styled(app.elapsed(), Style::default().fg(Color::DarkGray)),
    ]
}

fn render_body(frame: &mut Frame<'_>, app: &mut MonitorApp, area: Rect, effect_delta: FxDuration) {
    let (tabs, active_panel) = active_panel_layout(area);
    render_tabs(frame, app, tabs);
    match app.active_tab {
        0 => render_actions(frame, app, active_panel, effect_delta),
        1 => {
            render_config(frame, app, active_panel);
            process_event_feed_effects(app, effect_delta, frame.buffer_mut(), None);
        }
        2 => {
            render_messages(frame, app, active_panel);
            process_event_feed_effects(app, effect_delta, frame.buffer_mut(), None);
        }
        _ => {
            render_files(frame, app, active_panel);
            process_event_feed_effects(app, effect_delta, frame.buffer_mut(), None);
        }
    }
}

fn render_tabs(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    let tabs = Tabs::new(vec![" ACTIONS ", " CONFIG ", " MESSAGES ", " FILES "])
        .select(app.active_tab)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )
        .divider(symbols::line::VERTICAL);
    frame.render_widget(tabs, area);
}

fn render_actions(
    frame: &mut Frame<'_>,
    app: &mut MonitorApp,
    area: Rect,
    effect_delta: FxDuration,
) {
    let (command_palette, run_status, run_output) = actions_full_layout(area);
    render_command_palette(frame, app, command_palette);
    render_run_status(frame, app, run_status);
    render_run_output(frame, app, run_output, effect_delta);
}

fn process_event_feed_effects(
    app: &mut MonitorApp,
    effect_delta: FxDuration,
    buffer: &mut ratatui::buffer::Buffer,
    area: Option<Rect>,
) {
    if app.effects.is_running() {
        app.effects
            .process_effects(effect_delta, buffer, area.unwrap_or_default());
    }
}

fn render_command_palette(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    let (list_area, description_area) = command_palette_layout(area);
    let selected = app.selected_action;
    let actions = app.actions();
    let visible_rows = list_area.height.saturating_sub(2).max(1) as usize;
    let start = selected.saturating_sub(visible_rows / 2);
    let items = actions
        .iter()
        .enumerate()
        .skip(start)
        .take(visible_rows)
        .map(|(idx, action)| {
            let is_selected = idx == selected;
            let selected_style = if idx == selected {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            let runnable = action_runnable(*action, &app.load);
            let accent_color = if runnable { Color::Cyan } else { Color::Red };
            let marker = if is_selected { "▌" } else { " " };
            let icon = if runnable { "●" } else { "·" };
            let icon_style = if is_selected {
                Style::default()
                    .fg(accent_color)
                    .add_modifier(Modifier::BOLD)
            } else if runnable {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::Red)
            };
            let badge = if let Some(run) = app
                .active_run
                .as_ref()
                .filter(|run| run.label == action.label())
            {
                if run.cancel_requested { "STP" } else { "RUN" }
            } else if runnable {
                "OK "
            } else {
                "-- "
            };
            let badge_style = if is_selected && badge == "STP" {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::LightRed)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(accent_color)
                    .add_modifier(Modifier::BOLD)
            } else if badge == "RUN" || badge == "OK " {
                Style::default().fg(Color::Green)
            } else if badge == "STP" {
                Style::default().fg(Color::LightRed)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{marker} "),
                    if is_selected {
                        Style::default()
                            .fg(accent_color)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
                Span::styled(
                    format!("{:02}", idx + 1),
                    if is_selected {
                        Style::default()
                            .fg(accent_color)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ),
                Span::raw(" "),
                Span::styled(icon, icon_style),
                Span::raw("  "),
                Span::styled(action.label(), selected_style),
                Span::raw(" "),
                Span::styled(badge.trim(), badge_style),
            ]))
        })
        .collect::<Vec<_>>();

    frame.render_widget(
        List::new(items).block(
            accent_panel(format!(" COMMANDS {:02}/{} ", selected + 1, actions.len())).border_style(
                focus_border_style(app, FocusPane::Commands, Color::DarkGray),
            ),
        ),
        list_area,
    );
    render_command_description(frame, app, description_area);
}

fn render_command_description(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    if area.height == 0 {
        return;
    }

    let action = app.selected_action();
    frame.render_widget(
        Paragraph::new(action.description())
            .style(Style::default().fg(Color::Gray))
            .block(accent_panel(format!(" DETAIL {} ", action.label())))
            .wrap(Wrap { trim: true }),
        area,
    );
}

fn render_run_status(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    let selected_action = app.selected_action();
    let block = accent_panel(" STATUS ").border_style(focus_border_style(
        app,
        FocusPane::Status,
        run_status_color(app),
    ));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(inner);

    let status_line = if let Some(run) = &app.active_run {
        let status = if run.cancel_requested {
            "STOPPING "
        } else {
            "RUN "
        };
        Line::from(vec![
            Span::styled(
                status,
                Style::default()
                    .fg(if run.cancel_requested {
                        Color::LightRed
                    } else {
                        Color::Yellow
                    })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                run.label,
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!(
                "  {}",
                format_live_duration(run.started_at.elapsed())
            )),
        ])
    } else if let Some(record) = &app.last_run {
        Line::from(vec![
            Span::styled("LAST ", Style::default().fg(Color::Gray)),
            Span::styled(
                record.label,
                Style::default()
                    .fg(if record.ok { Color::Green } else { Color::Red })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  {}", format_duration(record.elapsed))),
            Span::styled("  │  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                record.result.clone(),
                Style::default().fg(if record.ok { Color::Green } else { Color::Red }),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled("IDLE ", Style::default().fg(Color::Gray)),
            Span::styled("NEXT ", Style::default().fg(Color::DarkGray)),
            Span::styled(selected_action.label(), Style::default().fg(Color::Cyan)),
        ])
    };
    frame.render_widget(Paragraph::new(status_line), chunks[0]);

    let runnable = action_runnable(selected_action, &app.load);
    let next_line = if app.command_running() {
        Line::from(vec![
            Span::styled("NEXT ", Style::default().fg(Color::DarkGray)),
            Span::styled(selected_action.label(), Style::default().fg(Color::Gray)),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                "READY ",
                Style::default().fg(if runnable { Color::Green } else { Color::Red }),
            ),
            Span::styled("next ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                selected_action.label(),
                Style::default().fg(if runnable { Color::Cyan } else { Color::Red }),
            ),
        ])
    };
    frame.render_widget(Paragraph::new(next_line), chunks[1]);
}

fn render_run_output(
    frame: &mut Frame<'_>,
    app: &mut MonitorApp,
    area: Rect,
    effect_delta: FxDuration,
) {
    let content_width = area.width.saturating_sub(3);
    let visual_line_count = if app.run_output.is_empty() {
        0
    } else {
        visual_output_line_count(&app.run_output, content_width)
    };
    let block_base = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(focus_border_style(
            app,
            FocusPane::Output,
            run_status_color(app),
        ));
    let inner = block_base.inner(area);
    let output_sections = output_inner_layout(inner);
    let log_area = output_sections.log;
    let visible_rows = output_visible_rows(log_area);
    let effective_scroll = effective_output_scroll(app, log_area, visual_line_count);
    let title = if app.run_output.is_empty() {
        " OUTPUT ".to_string()
    } else if effective_scroll == 0 {
        format!(" OUTPUT latest · {visual_line_count} lines ")
    } else {
        format!(" OUTPUT -{effective_scroll} lines ")
    };

    let block = block_base.title(title);
    frame.render_widget(block, area);

    render_output_status_bar(
        frame,
        app,
        output_sections.status,
        visual_line_count,
        effective_scroll,
    );
    render_run_timeline(frame, app, output_sections.timeline);

    if log_area.height == 0 || log_area.width == 0 {
        process_event_feed_effects(app, effect_delta, frame.buffer_mut(), None);
        return;
    }

    let log_width = log_area.width.saturating_sub(1);
    let lines = if app.run_output.is_empty() {
        vec![Line::styled(
            "  ready",
            Style::default().fg(Color::DarkGray),
        )]
    } else {
        let visual_lines = visual_output_lines_with_motion(
            &app.run_output,
            log_width,
            app.output_selection_range(),
            app.output_selected,
            app.command_running(),
            timeline_motion_frame(app),
        );
        let end = visual_lines.len().saturating_sub(effective_scroll);
        let start = end.saturating_sub(visible_rows);
        visual_lines
            .into_iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .map(|line| line.line)
            .collect()
    };
    let log_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(log_area);
    frame.render_widget(
        output_header(
            log_chunks[0].width,
            app.command_running(),
            timeline_motion_frame(app),
        ),
        log_chunks[0],
    );
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        log_chunks[1],
    );

    let effect_area = latest_event_feed_effect_area(
        log_chunks[1],
        visual_line_count,
        visible_rows,
        effective_scroll,
    );
    process_event_feed_effects(app, effect_delta, frame.buffer_mut(), effect_area);

    render_output_scrollbar(
        frame.buffer_mut(),
        log_chunks[1],
        visual_line_count,
        visible_rows,
        effective_scroll,
    );
}

fn render_output_scrollbar(
    buffer: &mut Buffer,
    area: Rect,
    visual_line_count: usize,
    visible_rows: usize,
    effective_scroll: usize,
) {
    let Some((thumb_start, thumb_len)) = output_scrollbar_thumb(
        visual_line_count,
        visible_rows,
        effective_scroll,
        area.height,
    ) else {
        return;
    };
    if area.width == 0 {
        return;
    }

    let x = area.right().saturating_sub(1);
    let thumb_end = thumb_start.saturating_add(thumb_len);
    for row in 0..area.height as usize {
        let (symbol, style) = if (thumb_start..thumb_end).contains(&row) {
            (
                "█",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            ("│", Style::default().fg(Color::DarkGray))
        };
        buffer.set_string(x, area.y + row as u16, symbol, style);
    }
}

fn output_scrollbar_thumb(
    visual_line_count: usize,
    visible_rows: usize,
    effective_scroll: usize,
    track_height: u16,
) -> Option<(usize, usize)> {
    let track_height = track_height as usize;
    if visual_line_count <= visible_rows || visible_rows == 0 || track_height == 0 {
        return None;
    }

    let max_scroll = visual_line_count.saturating_sub(visible_rows);
    let position_from_top = max_scroll.saturating_sub(effective_scroll.min(max_scroll));
    let thumb_len = visible_rows
        .saturating_mul(track_height)
        .div_ceil(visual_line_count)
        .clamp(1, track_height);
    let max_thumb_start = track_height.saturating_sub(thumb_len);
    let thumb_start = position_from_top
        .saturating_mul(max_thumb_start)
        .saturating_add(max_scroll / 2)
        .checked_div(max_scroll)
        .unwrap_or(0);

    Some((thumb_start, thumb_len))
}

fn output_header(width: u16, running: bool, frame: usize) -> Paragraph<'static> {
    Paragraph::new(Line::from(output_header_spans_with_motion(
        width, running, frame,
    )))
}

#[cfg(test)]
fn output_header_spans(width: u16) -> Vec<Span<'static>> {
    output_header_spans_with_motion(width, false, 0)
}

fn output_header_spans_with_motion(width: u16, running: bool, frame: usize) -> Vec<Span<'static>> {
    let scanner = event_feed_spinner_symbol(frame);
    let scanner_style = Style::default()
        .fg(event_feed_pulse_color(frame))
        .add_modifier(Modifier::BOLD);
    let mut spans = vec![
        Span::styled(
            " EVENT FEED ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        if running {
            Span::styled(format!("{scanner} live"), scanner_style)
        } else {
            Span::styled("analysis output", Style::default().fg(Color::Gray))
        },
    ];

    if width >= 52 {
        spans.extend([
            Span::raw("  "),
            Span::styled("●", Style::default().fg(Color::Green)),
            Span::raw(" "),
            Span::styled("ok", Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled("●", Style::default().fg(Color::Cyan)),
            Span::raw(" "),
            Span::styled("info", Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled("●", Style::default().fg(Color::LightRed)),
            Span::raw(" "),
            Span::styled("error", Style::default().fg(Color::DarkGray)),
        ]);
    }

    spans
}

fn event_feed_spinner_symbol(frame: usize) -> char {
    spinner_frame(FluxFrames::PISTON, frame)
}

fn event_feed_pulse_color(frame: usize) -> Color {
    if frame.is_multiple_of(2) {
        Color::LightCyan
    } else {
        Color::Cyan
    }
}

fn render_output_status_bar(
    frame: &mut Frame<'_>,
    app: &MonitorApp,
    area: Rect,
    visual_line_count: usize,
    effective_scroll: usize,
) {
    let (state, color) = if let Some(run) = &app.active_run {
        if run.cancel_requested {
            ("STOPPING", Color::LightRed)
        } else {
            ("RUNNING", Color::Yellow)
        }
    } else if app
        .last_run
        .as_ref()
        .map(|record| record.ok)
        .unwrap_or(true)
    {
        ("READY", Color::Green)
    } else {
        ("FAILED", Color::Red)
    };
    let scroll = if effective_scroll == 0 {
        "latest".to_string()
    } else {
        format!("-{effective_scroll} lines")
    };
    let selection = app
        .output_selection_range()
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
        .unwrap_or_else(|| "select --".to_string());

    let mut spans = vec![
        Span::styled(
            format!(" {state} "),
            Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled("lines ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            visual_line_count.to_string(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  /  ", Style::default().fg(Color::DarkGray)),
        Span::styled(selection, Style::default().fg(Color::Gray)),
        Span::styled("  /  ", Style::default().fg(Color::DarkGray)),
        Span::styled(scroll, Style::default().fg(Color::Cyan)),
    ];
    if let Some(status) = &app.copy_status {
        spans.extend([
            Span::styled("  │  ", Style::default().fg(Color::DarkGray)),
            Span::styled(status.clone(), Style::default().fg(Color::Cyan)),
        ]);
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

struct VisualOutputLine {
    entry_index: usize,
    line: Line<'static>,
}

fn visual_output_lines(
    entries: &[LogEntry],
    width: u16,
    selected_range: Option<(usize, usize)>,
    cursor: Option<usize>,
) -> Vec<VisualOutputLine> {
    visual_output_lines_with_motion(entries, width, selected_range, cursor, false, 0)
}

fn visual_output_lines_with_motion(
    entries: &[LogEntry],
    width: u16,
    selected_range: Option<(usize, usize)>,
    cursor: Option<usize>,
    running: bool,
    frame: usize,
) -> Vec<VisualOutputLine> {
    let width = width.max(1) as usize;
    let latest_entry = entries.len().saturating_sub(1);
    entries
        .iter()
        .enumerate()
        .flat_map(|(entry_index, entry)| {
            let text = strip_ansi_codes(&entry.text);
            let kind = classify_log_entry(entry.stream, &text);
            let Some(display) = output_display(kind, &text) else {
                return Vec::new();
            };
            let is_selected = selected_range
                .map(|(start, end)| (start..=end).contains(&entry_index))
                .unwrap_or(false);
            let is_cursor = cursor == Some(entry_index);
            let is_live_latest = running && entry_index == latest_entry && !is_selected;
            let context = OutputRenderContext {
                entry_index,
                width,
                selected: is_selected,
                cursor: is_cursor,
                live_latest: is_live_latest,
                frame,
            };
            render_output_display_lines(context, kind, display)
        })
        .collect()
}

fn visual_output_line_count(entries: &[LogEntry], width: u16) -> usize {
    let width = width.max(1) as usize;
    entries
        .iter()
        .map(|entry| {
            let text = strip_ansi_codes(&entry.text);
            let kind = classify_log_entry(entry.stream, &text);
            output_display(kind, &text)
                .map(|display| output_display_line_count(&display, width))
                .unwrap_or(0)
        })
        .sum()
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum OutputDisplay {
    Section(String),
    Metric { key: String, value: String },
    Continuation(String),
    Event(String),
}

#[derive(Clone, Copy)]
struct OutputRenderContext {
    entry_index: usize,
    width: usize,
    selected: bool,
    cursor: bool,
    live_latest: bool,
    frame: usize,
}

impl OutputDisplay {
    #[cfg(test)]
    fn plain_text(&self) -> String {
        match self {
            Self::Section(title) => title.clone(),
            Self::Metric { key, value } => format!("{key}  →  {value}"),
            Self::Continuation(value) => value.clone(),
            Self::Event(text) => text.clone(),
        }
    }
}

fn render_output_display_lines(
    context: OutputRenderContext,
    kind: LogKind,
    display: OutputDisplay,
) -> Vec<VisualOutputLine> {
    match display {
        OutputDisplay::Section(title) => {
            vec![VisualOutputLine {
                entry_index: context.entry_index,
                line: section_output_line(&title, kind, context),
            }]
        }
        OutputDisplay::Metric { key, value } => metric_output_lines(context, &key, &value),
        OutputDisplay::Continuation(value) => metric_continuation_lines(context, &value),
        OutputDisplay::Event(text) => event_output_lines(context, kind, &text),
    }
}

fn section_output_line(title: &str, kind: LogKind, context: OutputRenderContext) -> Line<'static> {
    let marker = if context.cursor {
        "◆".to_string()
    } else if context.selected {
        "▌".to_string()
    } else if context.live_latest {
        event_feed_spinner_symbol(context.frame).to_string()
    } else {
        kind.marker().to_string()
    };
    Line::from(vec![
        Span::styled(
            format!("{marker} "),
            selected_output_style(Style::default().fg(Color::Cyan), context.selected),
        ),
        Span::styled(
            "━━ ",
            selected_output_style(Style::default().fg(Color::DarkGray), context.selected),
        ),
        Span::styled(
            title.to_string(),
            selected_output_style(
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
                context.selected,
            ),
        ),
        Span::styled(
            " ━━",
            selected_output_style(Style::default().fg(Color::DarkGray), context.selected),
        ),
    ])
}

fn metric_output_lines(
    context: OutputRenderContext,
    key: &str,
    value: &str,
) -> Vec<VisualOutputLine> {
    let key_width = key.chars().count().clamp(8, 18);
    let value_width = context.width.saturating_sub(key_width + 10).max(12);
    wrap_log_text(value, value_width)
        .into_iter()
        .enumerate()
        .map(|(idx, value_line)| {
            let marker = if context.cursor && idx == 0 {
                "◆".to_string()
            } else if context.selected && idx == 0 {
                "▌".to_string()
            } else if context.live_latest && idx == 0 {
                event_feed_spinner_symbol(context.frame).to_string()
            } else {
                " ".to_string()
            };
            let rail = selected_output_style(
                live_output_rail_style(context.live_latest && idx == 0, context.frame),
                context.selected,
            );
            let spans = if idx == 0 {
                vec![
                    Span::styled(format!("{marker}  │ "), rail),
                    Span::styled(
                        format!("{key:key_width$}"),
                        selected_output_style(
                            Style::default()
                                .fg(Color::LightCyan)
                                .add_modifier(Modifier::BOLD),
                            context.selected,
                        ),
                    ),
                    Span::styled("  ", rail),
                    Span::styled(
                        value_line,
                        selected_output_style(Style::default().fg(Color::White), context.selected),
                    ),
                ]
            } else {
                vec![
                    Span::styled("   │ ", rail),
                    Span::styled(" ".repeat(key_width), rail),
                    Span::styled("  ", rail),
                    Span::styled(
                        value_line,
                        selected_output_style(Style::default().fg(Color::Gray), context.selected),
                    ),
                ]
            };
            VisualOutputLine {
                entry_index: context.entry_index,
                line: Line::from(spans),
            }
        })
        .collect()
}

fn metric_continuation_lines(context: OutputRenderContext, value: &str) -> Vec<VisualOutputLine> {
    let value_width = context.width.saturating_sub(13).max(12);
    wrap_log_text(value, value_width)
        .into_iter()
        .enumerate()
        .map(|(idx, value_line)| {
            let marker = if context.cursor && idx == 0 {
                "◆"
            } else if context.selected && idx == 0 {
                "▌"
            } else {
                " "
            };
            let rail =
                selected_output_style(Style::default().fg(Color::DarkGray), context.selected);
            VisualOutputLine {
                entry_index: context.entry_index,
                line: Line::from(vec![
                    Span::styled(format!("{marker}  │ "), rail),
                    Span::styled(
                        "↳ ",
                        selected_output_style(
                            Style::default().fg(Color::DarkGray),
                            context.selected,
                        ),
                    ),
                    Span::styled(
                        value_line,
                        selected_output_style(Style::default().fg(Color::Gray), context.selected),
                    ),
                ]),
            }
        })
        .collect()
}

fn event_output_lines(
    context: OutputRenderContext,
    kind: LogKind,
    text: &str,
) -> Vec<VisualOutputLine> {
    let text_width = context.width.saturating_sub(13).max(12);
    wrap_log_text(text, text_width)
        .into_iter()
        .enumerate()
        .map(|(idx, line)| {
            let spans = if idx == 0 {
                let marker = if context.cursor {
                    "◆".to_string()
                } else if context.selected {
                    "▌".to_string()
                } else if context.live_latest {
                    event_feed_spinner_symbol(context.frame).to_string()
                } else {
                    kind.marker().to_string()
                };
                vec![
                    Span::styled(
                        format!("{marker} "),
                        selected_output_style(
                            live_output_marker_style(kind, context.live_latest, context.frame),
                            context.selected,
                        ),
                    ),
                    Span::styled(
                        event_badge_cell(kind),
                        selected_output_style(kind.prefix_style(), context.selected),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        line,
                        selected_output_style(
                            live_output_text_style(kind, context.live_latest, context.frame),
                            context.selected,
                        ),
                    ),
                ]
            } else {
                vec![
                    Span::styled(
                        "   │       ",
                        selected_output_style(
                            Style::default().fg(Color::DarkGray),
                            context.selected,
                        ),
                    ),
                    Span::styled(
                        line,
                        selected_output_style(event_text_style(kind), context.selected),
                    ),
                ]
            };
            VisualOutputLine {
                entry_index: context.entry_index,
                line: Line::from(spans),
            }
        })
        .collect()
}

fn output_display_line_count(display: &OutputDisplay, width: usize) -> usize {
    match display {
        OutputDisplay::Section(_) => 1,
        OutputDisplay::Metric { key, value } => {
            let key_width = key.chars().count().clamp(8, 18);
            let value_width = width.saturating_sub(key_width + 10).max(12);
            wrap_line_count(value, value_width)
        }
        OutputDisplay::Continuation(value) => {
            let value_width = width.saturating_sub(13).max(12);
            wrap_line_count(value, value_width)
        }
        OutputDisplay::Event(text) => {
            let text_width = width.saturating_sub(13).max(12);
            wrap_line_count(text, text_width)
        }
    }
}

fn event_badge_cell(kind: LogKind) -> String {
    centered_text(kind.badge(), EVENT_BADGE_WIDTH)
}

fn wrap_log_text(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_len = 0;
    for ch in text.chars() {
        if current_len >= width {
            lines.push(current);
            current = String::new();
            current_len = 0;
        }
        current.push(ch);
        current_len += 1;
    }
    lines.push(current);
    lines
}

fn wrap_line_count(text: &str, width: usize) -> usize {
    if width == 0 {
        return text.chars().count().max(1);
    }
    let len = text.chars().count();
    len.max(1).div_ceil(width)
}

fn run_status_color(app: &MonitorApp) -> Color {
    if app
        .active_run
        .as_ref()
        .map(|run| run.cancel_requested)
        .unwrap_or(false)
    {
        Color::LightRed
    } else if app.command_running() {
        Color::Yellow
    } else if app
        .last_run
        .as_ref()
        .map(|record| record.ok)
        .unwrap_or(true)
    {
        Color::Green
    } else {
        Color::Red
    }
}

fn selected_output_style(style: Style, selected: bool) -> Style {
    if selected {
        style.bg(Color::DarkGray).add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

fn live_output_marker_style(kind: LogKind, live_latest: bool, frame: usize) -> Style {
    if live_latest {
        Style::default()
            .fg(event_feed_pulse_color(frame))
            .add_modifier(Modifier::BOLD)
    } else {
        kind.text_style()
    }
}

fn live_output_text_style(kind: LogKind, live_latest: bool, frame: usize) -> Style {
    if live_latest {
        event_text_style(kind)
            .fg(event_feed_pulse_color(frame))
            .add_modifier(Modifier::BOLD)
    } else {
        event_text_style(kind)
    }
}

fn live_output_rail_style(live_latest: bool, frame: usize) -> Style {
    if live_latest {
        Style::default()
            .fg(event_feed_pulse_color(frame))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn event_text_style(kind: LogKind) -> Style {
    match kind {
        LogKind::Success => Style::default().fg(Color::Green),
        LogKind::Save => Style::default().fg(Color::Magenta),
        LogKind::Read | LogKind::Info => Style::default().fg(Color::Cyan),
        LogKind::Warning => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        LogKind::Error => Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD),
        LogKind::System => Style::default().fg(Color::Yellow),
        LogKind::Fit => Style::default().fg(Color::LightYellow),
        _ => Style::default().fg(Color::Gray),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LogKind {
    Plain,
    System,
    Success,
    Info,
    Read,
    Save,
    Fit,
    Metric,
    Warning,
    Error,
    Section,
}

impl LogKind {
    fn badge(self) -> &'static str {
        match self {
            Self::Plain => "LOG",
            Self::System => "SYS",
            Self::Success => "OK",
            Self::Info => "INFO",
            Self::Read => "READ",
            Self::Save => "SAVE",
            Self::Fit => "FIT",
            Self::Metric => "KV",
            Self::Warning => "WARN",
            Self::Error => "ERR",
            Self::Section => "STEP",
        }
    }

    fn marker(self) -> &'static str {
        match self {
            Self::Plain => "•",
            Self::System => "◆",
            Self::Success => "✓",
            Self::Info => "i",
            Self::Read => "◉",
            Self::Save => "⬥",
            Self::Fit => "◇",
            Self::Metric => "›",
            Self::Warning => "!",
            Self::Error => "×",
            Self::Section => "▣",
        }
    }

    fn color(self) -> Color {
        match self {
            Self::Plain => Color::Gray,
            Self::System => Color::Yellow,
            Self::Success => Color::Green,
            Self::Info => Color::Cyan,
            Self::Read => Color::Blue,
            Self::Save => Color::Magenta,
            Self::Fit => Color::LightYellow,
            Self::Metric => Color::LightCyan,
            Self::Warning => Color::Yellow,
            Self::Error => Color::LightRed,
            Self::Section => Color::White,
        }
    }

    fn text_style(self) -> Style {
        let style = Style::default().fg(self.color());
        match self {
            Self::System | Self::Section | Self::Fit => style.add_modifier(Modifier::BOLD),
            Self::Error | Self::Warning | Self::Success => style.add_modifier(Modifier::BOLD),
            _ => style,
        }
    }

    fn prefix_style(self) -> Style {
        match self {
            Self::Plain => Style::default().fg(Color::DarkGray),
            Self::Error => Style::default()
                .fg(Color::Black)
                .bg(Color::LightRed)
                .add_modifier(Modifier::BOLD),
            Self::Warning => Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
            Self::Success => Style::default()
                .fg(Color::Black)
                .bg(Color::Green)
                .add_modifier(Modifier::BOLD),
            Self::Section => Style::default()
                .fg(Color::Black)
                .bg(Color::White)
                .add_modifier(Modifier::BOLD),
            _ => Style::default()
                .fg(Color::Black)
                .bg(self.color())
                .add_modifier(Modifier::BOLD),
        }
    }
}

#[cfg(test)]
fn display_output_text(kind: LogKind, text: &str) -> Option<String> {
    output_display(kind, text).map(|display| display.plain_text())
}

fn output_display(kind: LogKind, text: &str) -> Option<OutputDisplay> {
    let trimmed = text.trim_start();
    if let Some(title) = parse_panel_title(trimmed) {
        return Some(OutputDisplay::Section(title));
    }
    if let Some((key, value)) = parse_panel_row(trimmed) {
        return Some(OutputDisplay::Metric { key, value });
    }
    if let Some(value) = parse_panel_continuation(trimmed) {
        return Some(OutputDisplay::Continuation(value));
    }
    if is_table_border_line(trimmed) {
        return None;
    }
    if let Some(cells) = parse_table_cells(trimmed) {
        if is_table_header_cells(&cells) {
            return None;
        }
        if let Some((key, value)) = table_cells_to_metric(cells) {
            return Some(OutputDisplay::Metric { key, value });
        }
    }

    match kind {
        LogKind::Success | LogKind::Info | LogKind::Read | LogKind::Save => {
            Some(OutputDisplay::Event(
                strip_cli_badge(trimmed)
                    .unwrap_or(trimmed)
                    .trim_start()
                    .to_string(),
            ))
        }
        LogKind::Fit => Some(OutputDisplay::Event(
            trimmed
                .strip_prefix("🛠️")
                .unwrap_or(trimmed)
                .trim_start()
                .to_string(),
        )),
        LogKind::Section => Some(OutputDisplay::Section(text.trim().to_string())),
        _ => Some(OutputDisplay::Event(text.to_string())),
    }
}

fn strip_cli_badge(text: &str) -> Option<&str> {
    if !text.starts_with('[') {
        return None;
    }
    let end = text.find(']')?;
    (end <= 9).then_some(&text[end + 1..])
}

fn classify_log_entry(stream: OutputStream, text: &str) -> LogKind {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();

    if matches!(stream, OutputStream::System) {
        return LogKind::System;
    }
    if matches!(stream, OutputStream::Stderr)
        || lower.contains("error")
        || lower.contains("failed")
        || lower.contains("traceback")
    {
        return if lower.contains("warning") {
            LogKind::Warning
        } else {
            LogKind::Error
        };
    }
    if lower.contains("warning") || lower.contains("userwarning") {
        return LogKind::Warning;
    }
    if trimmed.contains("Fit result") || trimmed.starts_with("[[") {
        return LogKind::Fit;
    }
    if parse_table_cells(trimmed).is_some()
        || parse_panel_row(trimmed).is_some()
        || parse_panel_continuation(trimmed).is_some()
    {
        return LogKind::Metric;
    }
    if parse_panel_title(trimmed).is_some()
        || trimmed == "Lock-in settings"
        || trimmed.ends_with("settings")
        || is_table_border_line(trimmed)
    {
        return LogKind::Section;
    }
    if trimmed.contains("✅") || trimmed.starts_with("[  OK") || trimmed.starts_with("[ OK") {
        return LogKind::Success;
    }
    if trimmed.starts_with("[ INFO") {
        return LogKind::Info;
    }
    if trimmed.starts_with("[ READ") {
        return LogKind::Read;
    }
    if trimmed.starts_with("[ SAVE") {
        return LogKind::Save;
    }
    LogKind::Plain
}

fn is_table_border_line(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|ch| matches!(ch, '╭' | '╞' | '├' | '╰'))
}

fn parse_panel_title(text: &str) -> Option<String> {
    text.trim()
        .strip_prefix("╭─ ")
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
}

fn parse_panel_row(text: &str) -> Option<(String, String)> {
    let inner = text.trim().strip_prefix('│')?.trim();
    if inner.contains('┆') || inner.is_empty() {
        return None;
    }

    let split_at = inner.find("  ")?;
    let key = inner[..split_at].trim();
    let value = inner[split_at..].trim();
    if key.is_empty() || value.is_empty() {
        return None;
    }

    Some((key.to_string(), value.to_string()))
}

fn parse_panel_continuation(text: &str) -> Option<String> {
    let inner = text.trim_end().strip_prefix('│')?;
    if inner.contains('┆') || parse_panel_row(text).is_some() {
        return None;
    }

    let value = inner.trim();
    if value.is_empty() || value == "empty" {
        return None;
    }

    Some(value.to_string())
}

fn parse_table_cells(text: &str) -> Option<Vec<String>> {
    let trimmed = text.trim();
    if !(trimmed.starts_with('│') && trimmed.ends_with('│')) {
        return None;
    }

    let inner = trimmed.trim_start_matches('│').trim_end_matches('│').trim();
    let cells = inner
        .split('┆')
        .map(str::trim)
        .filter(|cell| !cell.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    if cells.len() < 2 {
        return None;
    }

    Some(cells)
}

fn table_cells_to_metric(cells: Vec<String>) -> Option<(String, String)> {
    let mut iter = cells.into_iter();
    let key = iter.next()?;
    let values = iter.collect::<Vec<_>>();
    if key.is_empty() || values.is_empty() {
        return None;
    }

    Some((key, values.join("  /  ")))
}

fn is_table_header_cells(cells: &[String]) -> bool {
    let normalized = cells.iter().map(|cell| cell.trim()).collect::<Vec<_>>();
    matches!(
        normalized.as_slice(),
        ["Metric", "Value"]
            | ["Setting", "Value"]
            | ["Item", "Value"]
            | ["Channel", "Role", "Label", "Unit", "Factor"]
    )
}

fn render_config(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    let Some((cfg, _)) = app.ready_config() else {
        let text = Paragraph::new("Configuration is not runnable. Open Messages for diagnostics.")
            .block(accent_panel(" CONFIG "))
            .wrap(Wrap { trim: true });
        frame.render_widget(text, area);
        return;
    };

    let inner = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(7), Constraint::Min(6)])
        .split(area);

    let summary = vec![
        vec!["Version".to_string(), cfg.version.to_string()],
        vec![
            "Roles".to_string(),
            format!(
                "sensor={:?}, reference=ch{}, signal={:?}",
                cfg.roles.sensor_ch, cfg.roles.reference_ch, cfg.roles.signal_ch
            ),
        ],
        vec![
            "Lock-in".to_string(),
            format!(
                "{:?}, workers={}, stride={}",
                cfg.lockin.lpf_kind, cfg.lockin.workers, cfg.lockin.stride_samples
            ),
        ],
        vec![
            "Kerr".to_string(),
            format!("{:?}, factor={}", cfg.kerr.kerr_type, cfg.kerr.factor),
        ],
    ];
    frame.render_widget(
        two_col_table(summary, " OVERVIEW ", inner[0].width),
        inner[0],
    );

    let visible_rows = inner[1].height.saturating_sub(3).max(1) as usize;
    let inner_width = inner[1].width.saturating_sub(6) as usize;
    let channel_width = 8;
    let role_width = 16;
    let unit_width = 10;
    let factor_width = 14;
    let label_width = inner_width
        .saturating_sub(channel_width + role_width + unit_width + factor_width)
        .max(8);
    let total = cfg.channels.len();
    let shown = total.min(visible_rows);
    let rows = cfg
        .channels
        .iter()
        .take(visible_rows)
        .map(|channel| {
            Row::new(vec![
                format!("ch{}", channel.index),
                fit_text(&channel_role(cfg, channel.index), role_width),
                fit_text(
                    &channel.label.clone().unwrap_or_else(|| "-".to_string()),
                    label_width,
                ),
                fit_text(
                    &channel.unit_out.clone().unwrap_or_else(|| "-".to_string()),
                    unit_width,
                ),
                fit_text(
                    &channel
                        .factor
                        .map(|factor| format!("{factor:.4e}"))
                        .unwrap_or_else(|| "-".to_string()),
                    factor_width,
                ),
            ])
        })
        .collect::<Vec<_>>();
    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(18),
            Constraint::Percentage(25),
            Constraint::Length(12),
            Constraint::Percentage(25),
        ],
    )
    .header(
        Row::new(vec!["Channel", "Role", "Label", "Unit", "Factor"]).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(accent_panel(format!(" CHANNELS {shown}/{total} ")));
    frame.render_widget(table, inner[1]);
}

fn render_messages(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    let mut lines = Vec::new();
    if let Some((_, warnings)) = app.ready_config() {
        if warnings.is_empty() {
            lines.push(Line::styled(
                "No warnings.",
                Style::default().fg(Color::Green),
            ));
        } else {
            for warning in warnings {
                lines.push(Line::from(vec![
                    Span::styled("WARN ", Style::default().fg(Color::Yellow)),
                    Span::raw(warning.message.clone()),
                ]));
            }
        }
    }

    if let Some(diag) = app.diagnostics() {
        lines.push(Line::styled(
            format!(
                "Config version: {}",
                diag.version.map_or("-".to_string(), |v| v.to_string())
            ),
            Style::default().fg(Color::Gray),
        ));
        for warning in &diag.warnings {
            lines.push(Line::from(vec![
                Span::styled("WARN ", Style::default().fg(Color::Yellow)),
                Span::raw(warning.message.clone()),
            ]));
        }
        for issue in &diag.diagnostics {
            let path = issue.path.as_deref().unwrap_or("-");
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} ", issue.kind),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("[{path}] "), Style::default().fg(Color::Gray)),
                Span::raw(issue.message.clone()),
            ]));
            if let Some(suggestion) = &issue.suggestion {
                lines.push(Line::from(vec![
                    Span::styled("  hint ", Style::default().fg(Color::Cyan)),
                    Span::raw(suggestion.clone()),
                ]));
            }
        }
    }

    let paragraph = Paragraph::new(lines)
        .block(accent_panel(" MESSAGES ").border_style(focus_border_style(
            app,
            FocusPane::Messages,
            Color::DarkGray,
        )))
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn render_files(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    let artifacts = artifact_rows(app.ready_config().map(|(cfg, _)| cfg));
    let visible_rows = area.height.saturating_sub(3).max(1) as usize;
    let inner_width = area.width.saturating_sub(6) as usize;
    let name_width = percent_width(inner_width, 28);
    let path_width = percent_width(inner_width, 42);
    let modified_width = percent_width(inner_width, 30);
    let total = artifacts.len();
    let shown = total.min(visible_rows);
    let rows = artifacts
        .into_iter()
        .take(visible_rows)
        .map(|artifact| {
            Row::new(vec![
                fit_text(&artifact.name, name_width),
                fit_path(&artifact.path, path_width),
                fit_text(&artifact.modified, modified_width),
            ])
            .style(Style::default().fg(artifact.color))
        })
        .collect::<Vec<_>>();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(28),
            Constraint::Percentage(42),
            Constraint::Percentage(30),
        ],
    )
    .header(
        Row::new(vec!["Artifact", "Path", "Modified"]).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        accent_panel(format!(" FILES {shown}/{total} ")).border_style(focus_border_style(
            app,
            FocusPane::Files,
            Color::DarkGray,
        )),
    );
    frame.render_widget(table, area);
}

fn render_help_overlay(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    let popup = centered_rect(70, 70, area);
    let selected = app.selected_action();
    let lines = vec![
        Line::from(vec![
            Span::styled(
                "pMOKE TUI",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("? / q", Style::default().fg(Color::Yellow)),
            Span::raw(" close"),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Selected  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                selected.label(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::styled(selected.description(), Style::default().fg(Color::Gray)),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Enter", Style::default().fg(Color::Cyan)),
            Span::raw(" run selected command"),
        ]),
        Line::from(vec![
            Span::styled("Esc", Style::default().fg(Color::Cyan)),
            Span::raw(" leave current mode"),
        ]),
        Line::from(vec![
            Span::styled("Ctrl+C", Style::default().fg(Color::Cyan)),
            Span::raw(" interrupt command or cancel output selection"),
        ]),
        Line::from(vec![
            Span::styled("j/k, Up/Down, g/G", Style::default().fg(Color::Cyan)),
            Span::raw(" move command selection"),
        ]),
        Line::from(vec![
            Span::styled("h/l, Tab/Shift-Tab", Style::default().fg(Color::Cyan)),
            Span::raw(" switch panels"),
        ]),
        Line::from(vec![
            Span::styled("a/o/m/f/s", Style::default().fg(Color::Cyan)),
            Span::raw(" focus actions/output/messages/files/status"),
        ]),
        Line::from(vec![
            Span::styled(
                "PageUp/PageDown, Ctrl-u/Ctrl-d",
                Style::default().fg(Color::Cyan),
            ),
            Span::raw(" scroll live output"),
        ]),
        Line::from(vec![
            Span::styled("o / Click output", Style::default().fg(Color::Cyan)),
            Span::raw(" focus and select output"),
        ]),
        Line::from(vec![
            Span::styled("j/k, g/G in output", Style::default().fg(Color::Cyan)),
            Span::raw(" move selected output line"),
        ]),
        Line::from(vec![
            Span::styled("V then j/k", Style::default().fg(Color::Cyan)),
            Span::raw(" visual-line select output"),
        ]),
        Line::from(vec![
            Span::styled("y / Enter in output", Style::default().fg(Color::Cyan)),
            Span::raw(" copy selected output lines"),
        ]),
        Line::from(vec![
            Span::styled("End", Style::default().fg(Color::Cyan)),
            Span::raw(" follow latest output"),
        ]),
        Line::from(vec![
            Span::styled("Mouse wheel", Style::default().fg(Color::Cyan)),
            Span::raw(" scroll output or command list"),
        ]),
        Line::from(vec![
            Span::styled("Click / drag", Style::default().fg(Color::Cyan)),
            Span::raw(" select command or output range"),
        ]),
        Line::from(vec![
            Span::styled("r", Style::default().fg(Color::Cyan)),
            Span::raw(" refresh config and files"),
        ]),
        Line::from(vec![
            Span::styled("q", Style::default().fg(Color::Cyan)),
            Span::raw(" quit when idle"),
        ]),
    ];

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Help ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Plain)
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .wrap(Wrap { trim: true }),
        popup,
    );
}

fn run_label(app: &MonitorApp) -> String {
    if let Some(run) = &app.active_run {
        format!(
            "{} {} {}",
            if run.cancel_requested {
                "STOPPING"
            } else {
                "RUN"
            },
            run.label,
            format_live_duration(run.started_at.elapsed())
        )
    } else if let Some(record) = &app.last_run {
        format!(
            "{} {} {}",
            if record.ok { "DONE" } else { "FAIL" },
            record.label,
            format_duration(record.elapsed)
        )
    } else {
        "IDLE".to_string()
    }
}

struct ArtifactRow {
    name: String,
    path: String,
    modified: String,
    color: Color,
}

fn artifact_rows(cfg: Option<&Config>) -> Vec<ArtifactRow> {
    let mut files = vec![("raw".to_string(), FETCHED_FNAME.to_string())];

    if let Some(cfg) = cfg {
        if matches!(cfg.fetch.output, FetchOutput::Raw | FetchOutput::CsvAndRaw) {
            files.push((
                "raw word".to_string(),
                format!("{RAW_WAVEFORM_DIR}/{RAW_METADATA_FNAME}"),
            ));
        }
        for ch in cfg.phase_signal_ch() {
            files.push((
                format!("li ch{ch}"),
                format!("{}_ch{}.csv", LI_RESULTS_NAME, ch),
            ));
            files.push((
                format!("rotated ch{ch}"),
                format!("{}_ch{}.csv", LI_ROTATED_NAME, ch),
            ));
        }
        files.push(("kerr".to_string(), format!("{}_results.csv", KERR_NAME)));
    }

    files
        .into_iter()
        .map(|(name, path)| {
            let status = file_status(&path);
            ArtifactRow {
                name,
                path,
                modified: status.modified,
                color: status.color,
            }
        })
        .collect()
}

struct FileStatus {
    modified: String,
    color: Color,
}

fn file_status(path: &str) -> FileStatus {
    match fs::metadata(path) {
        Ok(meta) => FileStatus {
            modified: meta
                .modified()
                .ok()
                .and_then(|time| time.elapsed().ok())
                .map(format_age)
                .unwrap_or_else(|| "-".to_string()),
            color: Color::Green,
        },
        Err(_) => FileStatus {
            modified: "-".to_string(),
            color: Color::DarkGray,
        },
    }
}

fn two_col_table(rows: Vec<Vec<String>>, title: &'static str, width: u16) -> Table<'static> {
    let value_width = width.saturating_sub(22) as usize;
    Table::new(
        rows.into_iter().map(|row| {
            let item = row.first().cloned().unwrap_or_default();
            let value = row.get(1).cloned().unwrap_or_default();
            Row::new(vec![fit_text(&item, 14), fit_text(&value, value_width)])
        }),
        [Constraint::Length(16), Constraint::Min(20)],
    )
    .header(
        Row::new(vec!["Item", "Value"]).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(accent_panel(title))
}

fn channel_role(cfg: &Config, ch: u8) -> String {
    let mut roles = Vec::new();
    if cfg.roles.sensor_ch.contains(&ch) {
        roles.push("sensor");
    }
    if cfg.roles.reference_ch == ch {
        roles.push("reference");
    }
    if cfg.roles.signal_ch.contains(&ch) {
        roles.push("signal");
    }
    if roles.is_empty() {
        "-".to_string()
    } else {
        roles.join(", ")
    }
}

fn accent_panel(title: impl Into<String>) -> Block<'static> {
    Block::default()
        .title(title.into())
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(Color::DarkGray))
}

fn focus_border_style(app: &MonitorApp, pane: FocusPane, fallback: Color) -> Style {
    if app.focus == pane {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(fallback)
    }
}

#[cfg(test)]
mod tests;
