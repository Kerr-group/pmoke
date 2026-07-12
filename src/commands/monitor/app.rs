use super::*;

pub(super) struct MonitorApp {
    pub(super) config_path: String,
    pub(super) current_dir: String,
    pub(super) load: ConfigLoad,
    pub(super) started_at: Instant,
    pub(super) last_refresh: SystemTime,
    pub(super) inspector_view: InspectorView,
    pub(super) focus: FocusPane,
    pub(super) workflow_cursor: usize,
    pub(super) collapsed_groups: std::collections::BTreeSet<ActionGroup>,
    pub(super) action_query: String,
    pub(super) search_mode: bool,
    pub(super) last_run: Option<RunRecord>,
    pub(super) run_history: std::collections::VecDeque<RunSnapshot>,
    pub(super) history_view: Option<usize>,
    pub(super) active_run: Option<ActiveRun>,
    pub(super) run_output: Vec<LogEntry>,
    pub(super) last_stderr_kind: Option<LogKind>,
    pub(super) run_output_scroll: usize,
    pub(super) new_output_events: usize,
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
    pub(super) motion_mode: MotionMode,
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
            inspector_view: InspectorView::Summary,
            focus: FocusPane::Commands,
            // Utilities is the first group, so row 1 selects the safe, read-only
            // Config action instead of a collapsible group header.
            workflow_cursor: 1,
            collapsed_groups: std::collections::BTreeSet::new(),
            action_query: String::new(),
            search_mode: false,
            last_run: None,
            run_history: std::collections::VecDeque::new(),
            history_view: None,
            active_run: None,
            run_output: Vec::new(),
            last_stderr_kind: None,
            run_output_scroll: 0,
            new_output_events: 0,
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
            motion_mode: MotionMode::from_env(),
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

    pub(super) fn workflow_entries(&self) -> Vec<WorkflowEntry> {
        let query = self.action_query.trim().to_ascii_lowercase();
        let actions = self.actions();
        let mut entries = Vec::new();
        for group in ActionGroup::ALL {
            let matching = actions
                .iter()
                .copied()
                .filter(|action| action.group() == group)
                .filter(|action| {
                    query.is_empty()
                        || action.label().to_ascii_lowercase().contains(&query)
                        || action.command_name().to_ascii_lowercase().contains(&query)
                        || action.description().to_ascii_lowercase().contains(&query)
                })
                .collect::<Vec<_>>();
            if matching.is_empty() {
                continue;
            }
            entries.push(WorkflowEntry::Group(group));
            if !self.collapsed_groups.contains(&group) || !query.is_empty() {
                entries.extend(matching.into_iter().map(WorkflowEntry::Action));
            }
        }
        entries
    }

    pub(super) fn selected_workflow_entry(&self) -> Option<WorkflowEntry> {
        let entries = self.workflow_entries();
        entries
            .get(self.workflow_cursor.min(entries.len().saturating_sub(1)))
            .copied()
    }

    pub(super) fn selected_action(&self) -> MonitorAction {
        match self.selected_workflow_entry() {
            Some(WorkflowEntry::Action(action)) => action,
            Some(WorkflowEntry::Group(group)) => self
                .actions()
                .into_iter()
                .find(|action| action.group() == group)
                .unwrap_or(MonitorAction::Show),
            None => MonitorAction::Show,
        }
    }

    pub(super) fn clamp_workflow_cursor(&mut self) {
        self.workflow_cursor = self
            .workflow_cursor
            .min(self.workflow_entries().len().saturating_sub(1));
    }

    pub(super) fn toggle_selected_group(&mut self) -> bool {
        let Some(WorkflowEntry::Group(group)) = self.selected_workflow_entry() else {
            return false;
        };
        if !self.collapsed_groups.remove(&group) {
            self.collapsed_groups.insert(group);
        }
        self.clamp_workflow_cursor();
        true
    }

    pub(super) fn begin_action_search(&mut self) {
        self.search_mode = true;
        self.action_query.clear();
        self.select_first_matching_action();
        self.focus_actions();
    }

    pub(super) fn clear_action_search(&mut self) {
        self.search_mode = false;
        self.action_query.clear();
        self.select_first_matching_action();
    }

    pub(super) fn push_action_query(&mut self, ch: char) {
        self.action_query.push(ch);
        self.select_first_matching_action();
    }

    pub(super) fn pop_action_query(&mut self) {
        self.action_query.pop();
        self.select_first_matching_action();
    }

    fn select_first_matching_action(&mut self) {
        self.workflow_cursor = self
            .workflow_entries()
            .iter()
            .position(|entry| matches!(entry, WorkflowEntry::Action(_)))
            .unwrap_or(0);
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
                RunEvent::Progress(stream, text) => self.push_progress(stream, &text),
                RunEvent::Structured(event) => self.push_structured_output(event),
                RunEvent::Finished { ok, status } => self.finish_run(ok, status),
                RunEvent::Failed(message) => self.finish_run(false, message),
            }
        }
    }

    pub(super) fn command_running(&self) -> bool {
        self.active_run.is_some()
    }

    pub(super) fn visible_output(&self) -> &[LogEntry] {
        self.history_view
            .and_then(|index| self.run_history.get(index))
            .map(|snapshot| snapshot.output.as_slice())
            .unwrap_or(&self.run_output)
    }

    pub(super) fn visible_run_record(&self) -> Option<&RunRecord> {
        self.history_view
            .and_then(|index| self.run_history.get(index))
            .map(|snapshot| &snapshot.record)
            .or(self.last_run.as_ref())
    }

    pub(super) fn history_position(&self) -> Option<(usize, usize)> {
        self.history_view
            .map(|index| (index + 1, self.run_history.len()))
    }

    pub(super) fn show_previous_run(&mut self) {
        // While idle, the newest snapshot is also the live output and must be
        // skipped. During a run, every completed snapshot is historical.
        let live_is_latest_snapshot = !self.command_running();
        let required = if live_is_latest_snapshot { 2 } else { 1 };
        if self.run_history.len() < required {
            return;
        }
        self.history_view = Some(match self.history_view {
            Some(index) => index.saturating_sub(1),
            None => self.run_history.len() - if live_is_latest_snapshot { 2 } else { 1 },
        });
        self.reset_output_view();
    }

    pub(super) fn show_next_run(&mut self) {
        let Some(index) = self.history_view else {
            return;
        };
        let next_is_history = if self.command_running() {
            index + 1 < self.run_history.len()
        } else {
            index + 2 < self.run_history.len()
        };
        self.history_view = next_is_history.then_some(index + 1);
        self.reset_output_view();
    }

    fn reset_output_view(&mut self) {
        self.run_output_scroll = 0;
        self.new_output_events = 0;
        self.output_selection_anchor = None;
        self.output_mouse_drag_active = false;
        self.output_selected = last_renderable_output_index(self.visible_output());
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
        let mut appended_events = 0;
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
                .push(LogEntry::with_kind(line.to_string(), kind, stream));
            appended += 1;
            appended_events += usize::from(output_display(kind, &clean_line).is_some());
        }
        self.after_output_appended(appended, appended_events);
    }

    pub(super) fn push_structured_output(&mut self, event: UiEvent) {
        let entries = LogEntry::from_event(&event);
        if let Some(progress_id) = event.progress_id.as_deref()
            && let Some(start) = self
                .run_output
                .iter()
                .position(|entry| entry.progress_id.as_deref() == Some(progress_id))
        {
            let end = self.run_output[start..]
                .iter()
                .position(|entry| entry.progress_id.as_deref() != Some(progress_id))
                .map_or(self.run_output.len(), |offset| start + offset);
            let old_len = end - start;
            let new_len = entries.len();
            self.run_output.splice(start..end, entries);
            if self.history_view.is_none() && self.run_output_scroll > 0 {
                self.run_output_scroll = if new_len >= old_len {
                    self.run_output_scroll.saturating_add(new_len - old_len)
                } else {
                    self.run_output_scroll.saturating_sub(old_len - new_len)
                };
            }
            self.output_selected =
                shift_index_after_replacement(self.output_selected, start, old_len, new_len);
            self.output_selection_anchor = shift_index_after_replacement(
                self.output_selection_anchor,
                start,
                old_len,
                new_len,
            );
            if event.kind != EventKind::Progress {
                self.trigger_event_feed_effect();
            }
            return;
        }
        let appended = entries.len();
        self.run_output.extend(entries);
        self.after_output_appended(appended, 1);
    }

    pub(super) fn push_progress(&mut self, stream: OutputStream, text: &str) {
        if text.is_empty() {
            return;
        }
        let clean = strip_ansi_codes(text);
        let kind = classify_log_entry(stream, &clean);
        if let Some(last) = self.run_output.last_mut()
            && last.transient
            && last.stream == stream
            && last.sequence.is_none()
        {
            last.text = text.to_string();
            last.kind = kind;
            self.trigger_event_feed_effect();
            return;
        }
        let mut entry = LogEntry::with_kind(text.to_string(), kind, stream);
        entry.transient = true;
        self.run_output.push(entry);
        self.after_output_appended(1, 1);
    }

    fn after_output_appended(&mut self, appended_entries: usize, appended_events: usize) {
        if appended_entries == 0 {
            return;
        }
        if self.history_view.is_none() && self.run_output_scroll > 0 {
            self.run_output_scroll += appended_entries;
            self.new_output_events = self.new_output_events.saturating_add(appended_events);
        }
        const MAX_LOG_LINES: usize = 1_000;
        if self.run_output.len() > MAX_LOG_LINES {
            let minimum = self.run_output.len() - MAX_LOG_LINES;
            let drained = complete_event_drain_count(&self.run_output, minimum);
            self.run_output.drain(0..drained);
            if self.history_view.is_none() {
                self.run_output_scroll = self.run_output_scroll.saturating_sub(drained);
                self.output_selected = shift_log_index_after_drain(self.output_selected, drained);
                self.output_selection_anchor =
                    shift_log_index_after_drain(self.output_selection_anchor, drained);
            }
        }
        if self.run_output.is_empty() {
            self.output_selected = None;
            self.output_selection_anchor = None;
        }
        self.trigger_event_feed_effect();
    }

    pub(super) fn trigger_event_feed_effect(&mut self) {
        if self.motion_mode != MotionMode::Full
            || self.history_view.is_some()
            || self.run_output_scroll > 0
        {
            return;
        }
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
        if self.run_output_scroll == 0 {
            self.new_output_events = 0;
        }
    }

    pub(super) fn follow_output(&mut self) {
        self.run_output_scroll = 0;
        self.new_output_events = 0;
    }

    pub(super) fn visible_event_count(&self) -> usize {
        logical_event_count(self.visible_output(), |_| true)
    }

    pub(super) fn visible_warning_count(&self) -> usize {
        logical_event_count(self.visible_output(), |entry| {
            entry.kind == LogKind::Warning
        })
    }

    pub(super) fn visible_error_count(&self) -> usize {
        logical_event_count(self.visible_output(), |entry| entry.kind == LogKind::Error)
    }

    pub(super) fn focus_actions(&mut self) {
        self.output_mouse_drag_active = false;
        self.focus = FocusPane::Commands;
    }

    pub(super) fn focus_status(&mut self) {
        self.focus_inspector();
    }

    pub(super) fn focus_output(&mut self) {
        self.focus = FocusPane::Output;
        if self.output_selected.is_none() && !self.visible_output().is_empty() {
            self.output_selected = last_renderable_output_index(self.visible_output());
        }
    }

    pub(super) fn focus_commands(&mut self) {
        self.focus_actions();
    }

    pub(super) fn focus_messages(&mut self) {
        self.inspector_view = InspectorView::Diagnostics;
        self.focus_inspector();
    }

    pub(super) fn focus_files(&mut self) {
        self.inspector_view = InspectorView::Artifacts;
        self.focus_inspector();
    }

    pub(super) fn focus_inspector(&mut self) {
        self.output_mouse_drag_active = false;
        self.focus = FocusPane::Inspector;
    }

    pub(super) fn cycle_inspector(&mut self) {
        self.inspector_view = self.inspector_view.next();
        self.focus_inspector();
    }

    pub(super) fn select_previous_output(&mut self, extend: bool) {
        self.focus_output();
        let Some(selected) = self.output_selected else {
            return;
        };
        if let Some(index) = previous_renderable_output_index(self.visible_output(), selected) {
            self.set_output_selection(index, extend);
        }
    }

    pub(super) fn select_next_output(&mut self, extend: bool) {
        self.focus_output();
        let Some(selected) = self.output_selected else {
            return;
        };
        if let Some(index) = next_renderable_output_index(self.visible_output(), selected) {
            self.set_output_selection(index, extend);
        }
    }

    pub(super) fn select_first_output(&mut self, extend: bool) {
        self.focus_output();
        if self.visible_output().is_empty() {
            return;
        }
        if let Some(index) = first_renderable_output_index(self.visible_output()) {
            self.set_output_selection(index, extend);
        }
    }

    pub(super) fn select_last_output(&mut self, extend: bool) {
        self.focus_output();
        if self.visible_output().is_empty() {
            return;
        }
        if let Some(index) = last_renderable_output_index(self.visible_output()) {
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
        self.output_selected = nearest_renderable_output_index(self.visible_output(), index);
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
            self.visible_output()
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
        self.archive_last_run();
        self.refresh();
    }

    pub(super) fn archive_last_run(&mut self) {
        let Some(record) = self.last_run.clone() else {
            return;
        };
        const MAX_RUN_HISTORY: usize = 10;
        self.run_history.push_back(RunSnapshot {
            record,
            output: self.run_output.clone(),
        });
        while self.run_history.len() > MAX_RUN_HISTORY {
            self.run_history.pop_front();
        }
        self.history_view = None;
    }
}

fn complete_event_drain_count(entries: &[LogEntry], minimum: usize) -> usize {
    let Some(sequence) = minimum
        .checked_sub(1)
        .and_then(|index| entries.get(index))
        .and_then(|entry| entry.sequence)
    else {
        return minimum;
    };
    let mut drained = minimum;
    while entries
        .get(drained)
        .is_some_and(|entry| entry.sequence == Some(sequence))
    {
        drained += 1;
    }
    drained
}

pub(super) fn shift_log_index_after_drain(index: Option<usize>, drained: usize) -> Option<usize> {
    index.and_then(|idx| idx.checked_sub(drained))
}

fn shift_index_after_replacement(
    index: Option<usize>,
    start: usize,
    old_len: usize,
    new_len: usize,
) -> Option<usize> {
    index.map(|index| {
        if index < start {
            index
        } else if index < start + old_len {
            start + new_len.saturating_sub(1)
        } else if new_len >= old_len {
            index.saturating_add(new_len - old_len)
        } else {
            index.saturating_sub(old_len - new_len)
        }
    })
}

fn logical_event_count(entries: &[LogEntry], include: impl Fn(&LogEntry) -> bool) -> usize {
    let mut structured = std::collections::BTreeSet::new();
    let mut legacy = 0usize;
    for entry in entries
        .iter()
        .filter(|entry| include(entry) && is_renderable_output_entry(entry))
    {
        if let Some(sequence) = entry.sequence {
            structured.insert(sequence);
        } else {
            legacy = legacy.saturating_add(1);
        }
    }
    legacy.saturating_add(structured.len())
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
