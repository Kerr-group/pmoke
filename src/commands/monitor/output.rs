use super::*;

pub(super) struct VisualOutputLine {
    pub(super) entry_index: usize,
    pub(super) line: Line<'static>,
}

pub(super) fn visual_output_lines(
    entries: &[LogEntry],
    width: u16,
    selected_range: Option<(usize, usize)>,
    cursor: Option<usize>,
) -> Vec<VisualOutputLine> {
    visual_output_lines_with_motion(entries, width, selected_range, cursor, false, 0)
}

pub(super) fn visual_output_lines_with_motion(
    entries: &[LogEntry],
    width: u16,
    selected_range: Option<(usize, usize)>,
    _cursor: Option<usize>,
    running: bool,
    frame: usize,
) -> Vec<VisualOutputLine> {
    let width = width.max(1) as usize;
    let latest_entry = entries.iter().rposition(is_renderable_output_entry);
    let show_elapsed =
        width >= ELAPSED_MIN_OUTPUT_WIDTH && entries.iter().any(|entry| entry.elapsed_ms.is_some());
    entries
        .iter()
        .enumerate()
        .flat_map(|(entry_index, entry)| {
            let text = strip_ansi_codes(&entry.text);
            let kind = entry.kind;
            let display = if let Some(field) = &entry.field {
                Some(OutputDisplay::Field {
                    key: field.key.clone(),
                    value: field.value.clone(),
                    last: field.last,
                })
            } else {
                output_display(kind, &text)
            };
            let Some(display) = display else {
                return Vec::new();
            };
            let is_selected = selected_range
                .map(|(start, end)| (start..=end).contains(&entry_index))
                .unwrap_or(false);
            let is_live_latest =
                running && entry.transient && Some(entry_index) == latest_entry && !is_selected;
            let context = OutputRenderContext {
                entry_index,
                width: output_content_width(width, show_elapsed),
                selected: is_selected,
                live_latest: is_live_latest,
                frame,
                elapsed_ms: entry.elapsed_ms,
                event_head: entry.event_head,
                show_elapsed,
            };
            render_output_display_lines(context, kind, display)
        })
        .collect()
}

pub(super) fn visual_output_line_count(entries: &[LogEntry], width: u16) -> usize {
    let width = width.max(1) as usize;
    let show_elapsed =
        width >= ELAPSED_MIN_OUTPUT_WIDTH && entries.iter().any(|entry| entry.elapsed_ms.is_some());
    entries
        .iter()
        .map(|entry| {
            let text = strip_ansi_codes(&entry.text);
            let kind = entry.kind;
            let content_width = output_content_width(width, show_elapsed);
            output_display(kind, &text)
                .map(|display| output_display_line_count(&display, content_width))
                .unwrap_or(0)
        })
        .sum()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum OutputDisplay {
    Section(String),
    Metric {
        key: String,
        value: String,
    },
    Field {
        key: String,
        value: String,
        last: bool,
    },
    Continuation(String),
    Event(String),
}

#[derive(Clone, Copy)]
pub(super) struct OutputRenderContext {
    entry_index: usize,
    width: usize,
    selected: bool,
    live_latest: bool,
    frame: usize,
    elapsed_ms: Option<u64>,
    event_head: bool,
    show_elapsed: bool,
}

const ELAPSED_PREFIX_WIDTH: usize = 9;
const ELAPSED_MIN_OUTPUT_WIDTH: usize = 52;

fn output_content_width(width: usize, show_elapsed: bool) -> usize {
    if show_elapsed {
        width.saturating_sub(ELAPSED_PREFIX_WIDTH)
    } else {
        width
    }
}

fn elapsed_prefix(context: OutputRenderContext, first_line: bool) -> Vec<Span<'static>> {
    if !context.show_elapsed {
        return Vec::new();
    }
    let text = if context.event_head && first_line && context.elapsed_ms.is_some() {
        // Keep the prefix at its documented fixed width even for very long runs.
        let elapsed_ms = context.elapsed_ms.unwrap_or_default();
        let total_tenths = (elapsed_ms / 100).min(99 * 600 + 599);
        let minutes = total_tenths / 600;
        let seconds = (total_tenths / 10) % 60;
        let tenths = total_tenths % 10;
        format!("{minutes:02}:{seconds:02}.{tenths}  ")
    } else {
        " ".repeat(ELAPSED_PREFIX_WIDTH)
    };
    vec![Span::styled(text, Style::default().fg(Color::DarkGray))]
}

impl OutputDisplay {
    #[cfg(test)]
    pub(super) fn plain_text(&self) -> String {
        match self {
            Self::Section(title) => title.clone(),
            Self::Metric { key, value } => format!("{key}  →  {value}"),
            Self::Field { key, value, .. } => format!("{key}  →  {value}"),
            Self::Continuation(value) => value.clone(),
            Self::Event(text) => text.clone(),
        }
    }
}

pub(super) fn render_output_display_lines(
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
        OutputDisplay::Field { key, value, last } => {
            field_output_lines(context, &key, &value, last)
        }
        OutputDisplay::Continuation(value) => metric_continuation_lines(context, &value),
        OutputDisplay::Event(text) => event_output_lines(context, kind, &text),
    }
}

pub(super) fn section_output_line(
    title: &str,
    kind: LogKind,
    context: OutputRenderContext,
) -> Line<'static> {
    let tag = if context.live_latest {
        "RUN"
    } else {
        kind.label()
    };
    let mut spans = elapsed_prefix(context, true);
    spans.extend([
        Span::styled(
            format!("{tag:<5} "),
            selected_output_style(Style::default().fg(Color::Cyan), context.selected),
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
    ]);
    Line::from(spans)
}

pub(super) fn metric_output_lines(
    context: OutputRenderContext,
    key: &str,
    value: &str,
) -> Vec<VisualOutputLine> {
    let key_width = key.width_cjk().clamp(8, 18);
    let value_width = context.width.saturating_sub(key_width + 10).max(12);
    wrap_log_text(value, value_width)
        .into_iter()
        .enumerate()
        .map(|(idx, value_line)| {
            let tag = if context.live_latest && idx == 0 {
                "RUN"
            } else {
                ""
            };
            let rail = selected_output_style(
                live_output_rail_style(context.live_latest, context.frame),
                context.selected,
            );
            let mut spans = elapsed_prefix(context, idx == 0);
            spans.extend(if idx == 0 {
                vec![
                    Span::styled(format!("{tag:<5} │ "), rail),
                    Span::styled(
                        pad_display_width(key, key_width),
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
                        selected_output_style(
                            live_output_metric_value_style(context.live_latest, context.frame),
                            context.selected,
                        ),
                    ),
                ]
            } else {
                vec![
                    Span::styled("      │ ", rail),
                    Span::styled(" ".repeat(key_width), rail),
                    Span::styled("  ", rail),
                    Span::styled(
                        value_line,
                        selected_output_style(
                            live_output_metric_continuation_style(
                                context.live_latest,
                                context.frame,
                            ),
                            context.selected,
                        ),
                    ),
                ]
            });
            VisualOutputLine {
                entry_index: context.entry_index,
                line: Line::from(spans),
            }
        })
        .collect()
}

pub(super) fn field_output_lines(
    context: OutputRenderContext,
    key: &str,
    value: &str,
    last: bool,
) -> Vec<VisualOutputLine> {
    let key_width = key.width_cjk().clamp(8, 18);
    let value_width = context.width.saturating_sub(key_width + 11).max(12);
    let branch = if last { "└─ " } else { "├─ " };
    wrap_log_text(value, value_width)
        .into_iter()
        .enumerate()
        .map(|(index, value_line)| {
            let mut spans = elapsed_prefix(context, index == 0);
            spans.extend([
                Span::styled(
                    if index == 0 {
                        format!("      {branch}")
                    } else {
                        "         ".to_string()
                    },
                    selected_output_style(
                        live_output_rail_style(context.live_latest, context.frame),
                        context.selected,
                    ),
                ),
                Span::styled(
                    if index == 0 {
                        pad_display_width(key, key_width)
                    } else {
                        " ".repeat(key_width)
                    },
                    selected_output_style(Style::default().fg(Color::DarkGray), context.selected),
                ),
                Span::raw("  "),
                Span::styled(
                    value_line,
                    selected_output_style(
                        live_output_metric_value_style(context.live_latest, context.frame),
                        context.selected,
                    ),
                ),
            ]);
            VisualOutputLine {
                entry_index: context.entry_index,
                line: Line::from(spans),
            }
        })
        .collect()
}

pub(super) fn metric_continuation_lines(
    context: OutputRenderContext,
    value: &str,
) -> Vec<VisualOutputLine> {
    let value_width = context.width.saturating_sub(9).max(12);
    wrap_log_text(value, value_width)
        .into_iter()
        .enumerate()
        .map(|(idx, value_line)| {
            let tag = if context.live_latest && idx == 0 {
                "RUN"
            } else {
                ""
            };
            let rail = selected_output_style(
                live_output_rail_style(context.live_latest, context.frame),
                context.selected,
            );
            VisualOutputLine {
                entry_index: context.entry_index,
                line: Line::from({
                    let mut spans = elapsed_prefix(context, idx == 0);
                    spans.extend([
                        Span::styled(format!("{tag:<5} └─ "), rail),
                        Span::styled(
                            value_line,
                            selected_output_style(
                                live_output_metric_continuation_style(
                                    context.live_latest,
                                    context.frame,
                                ),
                                context.selected,
                            ),
                        ),
                    ]);
                    spans
                }),
            }
        })
        .collect()
}

pub(super) fn event_output_lines(
    context: OutputRenderContext,
    kind: LogKind,
    text: &str,
) -> Vec<VisualOutputLine> {
    let text_width = context.width.saturating_sub(6).max(12);
    wrap_log_text(text, text_width)
        .into_iter()
        .enumerate()
        .map(|(idx, line)| {
            let mut spans = elapsed_prefix(context, idx == 0);
            spans.extend(if idx == 0 {
                let tag = if context.live_latest {
                    "RUN"
                } else {
                    kind.label()
                };
                vec![
                    Span::styled(
                        format!("{tag:<5} "),
                        selected_output_style(
                            live_output_marker_style(kind, context.live_latest, context.frame),
                            context.selected,
                        ),
                    ),
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
                        "      │ ",
                        selected_output_style(
                            live_output_rail_style(context.live_latest, context.frame),
                            context.selected,
                        ),
                    ),
                    Span::styled(
                        line,
                        selected_output_style(
                            live_output_text_style(kind, context.live_latest, context.frame),
                            context.selected,
                        ),
                    ),
                ]
            });
            VisualOutputLine {
                entry_index: context.entry_index,
                line: Line::from(spans),
            }
        })
        .collect()
}

pub(super) fn output_display_line_count(display: &OutputDisplay, width: usize) -> usize {
    match display {
        OutputDisplay::Section(_) => 1,
        OutputDisplay::Metric { key, value } => {
            let key_width = key.width_cjk().clamp(8, 18);
            let value_width = width.saturating_sub(key_width + 10).max(12);
            wrap_line_count(value, value_width)
        }
        OutputDisplay::Field { key, value, .. } => {
            let key_width = key.width_cjk().clamp(8, 18);
            let value_width = width.saturating_sub(key_width + 11).max(12);
            wrap_line_count(value, value_width)
        }
        OutputDisplay::Continuation(value) => {
            let value_width = width.saturating_sub(9).max(12);
            wrap_line_count(value, value_width)
        }
        OutputDisplay::Event(text) => {
            let text_width = width.saturating_sub(6).max(12);
            wrap_line_count(text, text_width)
        }
    }
}

pub(super) fn wrap_log_text(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;
    for ch in text.chars() {
        let ch_width = ch.width_cjk().unwrap_or(0);
        if current_width + ch_width > width && !current.is_empty() {
            lines.push(current);
            current = String::new();
            current_width = 0;
        }
        current.push(ch);
        current_width += ch_width;
    }
    lines.push(current);
    lines
}

pub(super) fn wrap_line_count(text: &str, width: usize) -> usize {
    if width == 0 {
        return text.width_cjk().max(1);
    }
    wrap_log_text(text, width).len()
}

pub(super) fn run_status_color(app: &MonitorApp) -> Color {
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
        .visible_run_record()
        .map(|record| record.ok)
        .unwrap_or(true)
    {
        Color::Green
    } else {
        Color::Red
    }
}

pub(super) fn selected_output_style(style: Style, selected: bool) -> Style {
    if selected {
        style.bg(Color::DarkGray).add_modifier(Modifier::BOLD)
    } else {
        style
    }
}

pub(super) fn live_output_marker_style(kind: LogKind, live_latest: bool, frame: usize) -> Style {
    if live_latest {
        Style::default()
            .fg(event_feed_pulse_color(frame))
            .add_modifier(Modifier::BOLD)
    } else {
        kind.text_style()
    }
}

pub(super) fn live_output_text_style(kind: LogKind, live_latest: bool, frame: usize) -> Style {
    if live_latest {
        event_text_style(kind)
            .fg(event_feed_pulse_color(frame))
            .add_modifier(Modifier::BOLD)
    } else {
        event_text_style(kind)
    }
}

pub(super) fn live_output_rail_style(live_latest: bool, frame: usize) -> Style {
    if live_latest {
        Style::default()
            .fg(event_feed_pulse_color(frame))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

pub(super) fn live_output_metric_value_style(live_latest: bool, frame: usize) -> Style {
    if live_latest {
        Style::default()
            .fg(event_feed_pulse_color(frame))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::White)
    }
}

pub(super) fn live_output_metric_continuation_style(live_latest: bool, frame: usize) -> Style {
    if live_latest {
        Style::default()
            .fg(event_feed_pulse_color(frame))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::Gray)
    }
}

pub(super) fn event_text_style(kind: LogKind) -> Style {
    match kind {
        LogKind::Success => Style::default().fg(Color::Green),
        LogKind::Save => Style::default().fg(Color::Magenta),
        LogKind::Read | LogKind::Info => Style::default().fg(Color::Cyan),
        LogKind::Warning => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
        LogKind::Skipped => Style::default().fg(Color::Yellow),
        LogKind::Error => Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD),
        LogKind::System => Style::default().fg(Color::Gray),
        LogKind::Fit => Style::default().fg(Color::LightYellow),
        _ => Style::default().fg(Color::Gray),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LogKind {
    Plain,
    System,
    Success,
    Info,
    Read,
    Save,
    Fit,
    Metric,
    Skipped,
    Warning,
    Error,
    Section,
}

impl LogKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Plain => "LOG",
            Self::System => "SYS",
            Self::Success => "OK",
            Self::Info => "INFO",
            Self::Read => "READ",
            Self::Save => "SAVE",
            Self::Fit => "FIT",
            Self::Metric => "DATA",
            Self::Skipped => "SKIP",
            Self::Warning => "WARN",
            Self::Error => "ERR",
            Self::Section => "STEP",
        }
    }

    pub(super) fn color(self) -> Color {
        match self {
            Self::Plain => Color::Gray,
            Self::System => Color::Gray,
            Self::Success => Color::Green,
            Self::Info => Color::Cyan,
            Self::Read => Color::Blue,
            Self::Save => Color::Magenta,
            Self::Fit => Color::LightYellow,
            Self::Metric => Color::LightCyan,
            Self::Skipped => Color::Yellow,
            Self::Warning => Color::Yellow,
            Self::Error => Color::LightRed,
            Self::Section => Color::White,
        }
    }

    pub(super) fn text_style(self) -> Style {
        let style = Style::default().fg(self.color());
        match self {
            Self::Section | Self::Fit => style.add_modifier(Modifier::BOLD),
            Self::Error | Self::Warning | Self::Success => style.add_modifier(Modifier::BOLD),
            _ => style,
        }
    }
}

#[cfg(test)]
pub(super) fn display_output_text(kind: LogKind, text: &str) -> Option<String> {
    output_display(kind, text).map(|display| display.plain_text())
}

pub(super) fn output_display(kind: LogKind, text: &str) -> Option<OutputDisplay> {
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
        LogKind::Success
        | LogKind::Info
        | LogKind::Read
        | LogKind::Save
        | LogKind::Skipped
        | LogKind::Warning
        | LogKind::Error => Some(OutputDisplay::Event(
            strip_cli_badge(trimmed)
                .unwrap_or(trimmed)
                .trim_start()
                .to_string(),
        )),
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

pub(super) fn strip_cli_badge(text: &str) -> Option<&str> {
    if !text.starts_with('[') {
        return None;
    }
    let end = text.find(']')?;
    (end <= 9).then_some(&text[end + 1..])
}

pub(super) fn classify_log_entry(stream: OutputStream, text: &str) -> LogKind {
    let trimmed = text.trim();

    if matches!(stream, OutputStream::System) {
        return LogKind::System;
    }

    // stderr is also the conventional stream for warnings. Respect an explicit
    // severity marker before using the stream as an error fallback.
    if let Some(kind) = marked_log_kind(trimmed) {
        return kind;
    }
    if matches!(stream, OutputStream::Stderr) {
        return LogKind::Error;
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
        || matches!(trimmed, "Warnings" | "Diagnostics")
        || trimmed == "Lock-in settings"
        || trimmed.ends_with("settings")
        || is_table_border_line(trimmed)
    {
        return LogKind::Section;
    }
    if trimmed.contains("✅") {
        return LogKind::Success;
    }
    LogKind::Plain
}

pub(super) fn marked_log_kind(text: &str) -> Option<LogKind> {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();

    if let Some(kind) = cli_badge_kind(trimmed) {
        return Some(kind);
    }
    if lower.contains("warning:")
        || lower.starts_with("warn:")
        || lower.starts_with("warn ")
        || lower.starts_with("[warn]")
        || lower.starts_with("(warn)")
    {
        return Some(LogKind::Warning);
    }
    if lower.starts_with("info:") || lower.starts_with("[info]") || lower.starts_with("(info)") {
        return Some(LogKind::Info);
    }
    if lower.starts_with("error:")
        || lower.starts_with("error[")
        || lower.starts_with("(error)")
        || lower.starts_with("traceback")
        || lower.starts_with("failed to ")
    {
        return Some(LogKind::Error);
    }
    None
}

pub(super) fn cli_badge_kind(text: &str) -> Option<LogKind> {
    let label = text
        .strip_prefix('[')?
        .split_once(']')?
        .0
        .trim()
        .to_ascii_uppercase();

    match label.as_str() {
        "OK" => Some(LogKind::Success),
        "INFO" => Some(LogKind::Info),
        "READ" => Some(LogKind::Read),
        "SAVE" => Some(LogKind::Save),
        "SKIP" | "SKIPPED" => Some(LogKind::Skipped),
        "WARN" | "WARNING" => Some(LogKind::Warning),
        "ERR" | "ERROR" => Some(LogKind::Error),
        _ => None,
    }
}

pub(super) fn is_table_border_line(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|ch| matches!(ch, '╭' | '╞' | '├' | '╰'))
}

pub(super) fn parse_panel_title(text: &str) -> Option<String> {
    text.trim()
        .strip_prefix("╭─ ")
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(ToOwned::to_owned)
}

pub(super) fn parse_panel_row(text: &str) -> Option<(String, String)> {
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

pub(super) fn parse_panel_continuation(text: &str) -> Option<String> {
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

pub(super) fn parse_table_cells(text: &str) -> Option<Vec<String>> {
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

pub(super) fn table_cells_to_metric(cells: Vec<String>) -> Option<(String, String)> {
    let mut iter = cells.into_iter();
    let key = iter.next()?;
    let values = iter.collect::<Vec<_>>();
    if key.is_empty() || values.is_empty() {
        return None;
    }

    Some((key, values.join("  /  ")))
}

pub(super) fn is_table_header_cells(cells: &[String]) -> bool {
    let normalized = cells.iter().map(|cell| cell.trim()).collect::<Vec<_>>();
    matches!(
        normalized.as_slice(),
        ["Metric", "Value"]
            | ["Setting", "Value"]
            | ["Item", "Value"]
            | ["Channel", "Role", "Label", "Unit", "Factor"]
    )
}
