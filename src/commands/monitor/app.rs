use super::*;

pub(super) struct MonitorApp {
    pub(super) config_path: String,
    pub(super) current_dir: String,
    pub(super) load: ConfigLoad,
    pub(super) started_at: Instant,
    pub(super) last_refresh: SystemTime,
    pub(super) active_tab: usize,
    pub(super) focus: FocusPane,
    pub(super) selected_action: usize,
    pub(super) last_run: Option<RunRecord>,
    pub(super) active_run: Option<ActiveRun>,
    pub(super) run_output: Vec<LogEntry>,
    pub(super) last_stderr_kind: Option<LogKind>,
    pub(super) run_output_scroll: usize,
    pub(super) output_selected: Option<usize>,
    pub(super) output_selection_anchor: Option<usize>,
    pub(super) output_mouse_drag_active: bool,
    pub(super) config_scroll: usize,
    pub(super) messages_scroll: usize,
    pub(super) files_scroll: usize,
    pub(super) copy_status: Option<String>,
    pub(super) show_help: bool,
    pub(super) effects: EffectManager<MonitorEffect>,
    pub(super) last_effect_frame: Instant,
}

impl MonitorApp {
    pub(super) fn new(config_path: String, load: ConfigLoad) -> Self {
        let current_dir = env::current_dir()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_else(|_| ".".to_string());
        Self {
            config_path,
            current_dir,
            load,
            started_at: Instant::now(),
            last_refresh: SystemTime::now(),
            active_tab: 0,
            focus: FocusPane::Commands,
            selected_action: 0,
            last_run: None,
            active_run: None,
            run_output: Vec::new(),
            last_stderr_kind: None,
            run_output_scroll: 0,
            output_selected: None,
            output_selection_anchor: None,
            output_mouse_drag_active: false,
            config_scroll: 0,
            messages_scroll: 0,
            files_scroll: 0,
            copy_status: None,
            show_help: false,
            effects: EffectManager::default(),
            last_effect_frame: Instant::now(),
        }
    }

    pub(super) fn refresh(&mut self) {
        self.load = config::load_from_path(&self.config_path);
        self.last_refresh = SystemTime::now();
        self.config_scroll = 0;
        self.messages_scroll = 0;
        self.files_scroll = 0;
    }

    pub(super) fn status(&self) -> (&'static str, Color) {
        match self.load {
            ConfigLoad::Ready { .. } => ("RUNNABLE", Color::Green),
            ConfigLoad::Diagnostics(_) => ("BLOCKED", Color::Red),
        }
    }

    pub(super) fn elapsed(&self) -> String {
        let elapsed = self.started_at.elapsed();
        let mins = elapsed.as_secs() / 60;
        let secs = elapsed.as_secs() % 60;
        format!("{mins:02}:{secs:02}")
    }

    pub(super) fn ready_config(&self) -> Option<(&Config, &[ConfigWarning])> {
        match &self.load {
            ConfigLoad::Ready { config, warnings } => Some((config, warnings)),
            ConfigLoad::Diagnostics(_) => None,
        }
    }

    pub(super) fn diagnostics(&self) -> Option<&ConfigDiagnostics> {
        match &self.load {
            ConfigLoad::Diagnostics(diag) => Some(diag),
            ConfigLoad::Ready { .. } => None,
        }
    }

    pub(super) fn actions(&self) -> Vec<MonitorAction> {
        monitor_actions()
    }

    pub(super) fn selected_action(&self) -> MonitorAction {
        let actions = self.actions();
        actions
            .get(self.selected_action.min(actions.len().saturating_sub(1)))
            .copied()
            .unwrap_or(MonitorAction::Show)
    }

    pub(super) fn poll_command(&mut self) {
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

    pub(super) fn command_running(&self) -> bool {
        self.active_run.is_some()
    }

    pub(super) fn cancel_command(&mut self, reason: CancelReason) {
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

    pub(super) fn interrupt_current_operation(&mut self) {
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

    pub(super) fn escape_current_mode(&mut self) {
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

    pub(super) fn push_output(&mut self, stream: OutputStream, text: &str) {
        let mut appended = 0;
        for line in text.replace('\r', "\n").split('\n') {
            if line.is_empty() {
                if matches!(stream, OutputStream::Stderr) {
                    self.last_stderr_kind = None;
                }
                continue;
            }
            let clean_line = strip_ansi_codes(line);
            let mut kind = classify_log_entry(stream, &clean_line);
            if matches!(stream, OutputStream::Stderr)
                && kind == LogKind::Error
                && clean_line.starts_with(char::is_whitespace)
                && marked_log_kind(&clean_line).is_none()
                && let Some(inherited @ (LogKind::Warning | LogKind::Info)) = self.last_stderr_kind
            {
                kind = inherited;
            }
            if matches!(stream, OutputStream::Stderr) {
                self.last_stderr_kind = Some(kind);
            }
            self.run_output
                .push(LogEntry::with_kind(line.to_string(), kind));
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

    pub(super) fn trigger_event_feed_effect(&mut self) {
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

    pub(super) fn effect_delta(&mut self) -> FxDuration {
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(self.last_effect_frame);
        self.last_effect_frame = now;
        fx_duration(elapsed)
    }

    pub(super) fn scroll_output_up(&mut self, lines: usize) {
        self.run_output_scroll = self.run_output_scroll.saturating_add(lines);
    }

    pub(super) fn scroll_output_down(&mut self, lines: usize) {
        self.run_output_scroll = self.run_output_scroll.saturating_sub(lines);
    }

    pub(super) fn follow_output(&mut self) {
        self.run_output_scroll = 0;
    }

    pub(super) fn focus_actions(&mut self) {
        self.active_tab = 0;
        self.focus = FocusPane::Commands;
    }

    pub(super) fn focus_status(&mut self) {
        self.active_tab = 0;
        self.focus = FocusPane::Status;
    }

    pub(super) fn focus_output(&mut self) {
        self.active_tab = 0;
        self.focus = FocusPane::Output;
        if self.output_selected.is_none() && !self.run_output.is_empty() {
            self.output_selected = last_renderable_output_index(&self.run_output);
        }
    }

    pub(super) fn focus_commands(&mut self) {
        self.focus_actions();
    }

    pub(super) fn focus_messages(&mut self) {
        self.active_tab = 2;
        self.focus = FocusPane::Messages;
    }

    pub(super) fn focus_files(&mut self) {
        self.active_tab = 3;
        self.focus = FocusPane::Files;
    }

    pub(super) fn select_previous_output(&mut self, extend: bool) {
        self.focus_output();
        let Some(selected) = self.output_selected else {
            return;
        };
        if let Some(index) = previous_renderable_output_index(&self.run_output, selected) {
            self.set_output_selection(index, extend);
        }
    }

    pub(super) fn select_next_output(&mut self, extend: bool) {
        self.focus_output();
        let Some(selected) = self.output_selected else {
            return;
        };
        if let Some(index) = next_renderable_output_index(&self.run_output, selected) {
            self.set_output_selection(index, extend);
        }
    }

    pub(super) fn select_first_output(&mut self, extend: bool) {
        self.focus_output();
        if self.run_output.is_empty() {
            return;
        }
        if let Some(index) = first_renderable_output_index(&self.run_output) {
            self.set_output_selection(index, extend);
        }
    }

    pub(super) fn select_last_output(&mut self, extend: bool) {
        self.focus_output();
        if self.run_output.is_empty() {
            return;
        }
        if let Some(index) = last_renderable_output_index(&self.run_output) {
            self.set_output_selection(index, extend);
            self.follow_output();
        }
    }

    pub(super) fn enter_output_line_visual_mode(&mut self) {
        self.focus_output();
        if self.output_selected.is_none() {
            return;
        }
        self.output_selection_anchor = self.output_selected;
    }

    pub(super) fn set_output_selection(&mut self, index: usize, extend: bool) {
        self.focus_output();
        if extend {
            if self.output_selection_anchor.is_none() {
                self.output_selection_anchor = self.output_selected.or(Some(index));
            }
        } else {
            self.output_selection_anchor = None;
        }
        self.output_selected = nearest_renderable_output_index(&self.run_output, index);
    }

    pub(super) fn selected_output_text(&self) -> Option<String> {
        let entries = self.selected_output_entries()?;
        if entries.is_empty() {
            return None;
        }

        Some(
            entries
                .into_iter()
                .map(|entry| strip_ansi_codes(&entry.text))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    pub(super) fn selected_output_entries(&self) -> Option<Vec<&LogEntry>> {
        let (start, end) = self.output_selection_range()?;
        Some(
            self.run_output
                .get(start..=end)?
                .iter()
                .filter(|entry| is_renderable_output_entry(entry))
                .collect(),
        )
    }

    pub(super) fn output_selection_range(&self) -> Option<(usize, usize)> {
        let cursor = self.output_selected?;
        let anchor = self.output_selection_anchor.unwrap_or(cursor);
        Some((anchor.min(cursor), anchor.max(cursor)))
    }

    pub(super) fn output_selection_line_count(&self) -> usize {
        self.selected_output_entries()
            .map(|entries| entries.len())
            .unwrap_or(0)
    }

    pub(super) fn finish_output_yank(&mut self, text_len: usize, method: ClipboardMethod) {
        let lines = self.output_selection_line_count();
        self.copy_status = Some(format!(
            "copied {lines} lines / {text_len} chars via {}",
            method.label()
        ));
        self.output_selection_anchor = None;
    }

    pub(super) fn copy_selected_output(&mut self) {
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

    pub(super) fn finish_run(&mut self, ok: bool, result: String) {
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

pub(super) fn shift_log_index_after_drain(index: Option<usize>, drained: usize) -> Option<usize> {
    index.and_then(|idx| idx.checked_sub(drained))
}

pub(super) fn is_renderable_output_entry(entry: &LogEntry) -> bool {
    let text = strip_ansi_codes(&entry.text);
    output_display(entry.kind, &text).is_some()
}

pub(super) fn first_renderable_output_index(entries: &[LogEntry]) -> Option<usize> {
    entries.iter().position(is_renderable_output_entry)
}

pub(super) fn last_renderable_output_index(entries: &[LogEntry]) -> Option<usize> {
    entries.iter().rposition(is_renderable_output_entry)
}

pub(super) fn previous_renderable_output_index(
    entries: &[LogEntry],
    selected: usize,
) -> Option<usize> {
    entries
        .iter()
        .take(selected)
        .rposition(is_renderable_output_entry)
}

pub(super) fn next_renderable_output_index(entries: &[LogEntry], selected: usize) -> Option<usize> {
    entries
        .iter()
        .enumerate()
        .skip(selected.saturating_add(1))
        .find_map(|(idx, entry)| is_renderable_output_entry(entry).then_some(idx))
}

pub(super) fn nearest_renderable_output_index(entries: &[LogEntry], index: usize) -> Option<usize> {
    if entries.get(index).is_some_and(is_renderable_output_entry) {
        return Some(index);
    }

    entries
        .iter()
        .enumerate()
        .skip(index.saturating_add(1))
        .find_map(|(idx, entry)| is_renderable_output_entry(entry).then_some(idx))
        .or_else(|| {
            entries
                .iter()
                .take(index)
                .rposition(is_renderable_output_entry)
        })
}
