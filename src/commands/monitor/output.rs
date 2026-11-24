use super::*;

pub(super) struct VisualOutputLine {
    pub(super) entry_index: usize,
    // UTF-8 offset in the displayed payload, excluding tags and elapsed time.
    text_start: usize,
    pub(super) line: Line<'static>,
}

/// First row of a paused viewport, independent of output appended below it.
#[derive(Clone, Copy)]
pub(super) struct OutputAnchor {
    pub(super) entry: usize,
    pub(super) text_start: usize,
}

impl OutputAnchor {
    pub(super) fn at(lines: &[VisualOutputLine], top: usize) -> Option<Self> {
        let line = lines.get(top)?;
        Some(Self {
            entry: line.entry_index,
            text_start: line.text_start,
        })
    }

    pub(super) fn top(self, lines: &[VisualOutputLine]) -> usize {
        let first = lines.partition_point(|line| line.entry_index < self.entry);
        let count = lines[first..].partition_point(|line| {
            line.entry_index == self.entry && line.text_start <= self.text_start
        });
        first + count.saturating_sub(1)
    }
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
            let kind = entry.kind;
            let Some(display) = entry.display() else {
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

impl LogEntry {
    pub(super) fn display(&self) -> Option<OutputDisplay> {
        if let Some(field) = &self.field {
            Some(OutputDisplay::Field {
                key: field.key.clone(),
                value: field.value.clone(),
                last: field.last,
            })
        } else {
            output_display(self.kind, &strip_ansi_codes(&self.text))
        }
    }
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
            wrap_log_text(&title, context.width.saturating_sub(6).max(1))
                .into_iter()
                .map(|(text_start, text)| VisualOutputLine {
                    entry_index: context.entry_index,
                    text_start,
                    line: section_output_line(text, kind, context, text_start == 0),
                })
                .collect()
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
    title: String,
    kind: LogKind,
    context: OutputRenderContext,
    first_line: bool,
) -> Line<'static> {
    let tag = if !first_line {
        ""
    } else if context.live_latest {
        "RUN"
    } else {
        kind.label()
    };
    let mut spans = elapsed_prefix(context, first_line);
    spans.extend([
        Span::styled(
            format!("{tag:<5} "),
            selected_output_style(Style::default().fg(Color::Cyan), context.selected),
        ),
        Span::styled(
            title,
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

fn metric_widths(key: &str, value: &str, width: usize, decoration: usize) -> (usize, usize) {
    let available = width.saturating_sub(decoration);
    // Show the entire name when it fits. Reserve a readable part of a long
    // value without abbreviating the name just to leave unused space.
    let value_reserve = activity_text_width(value).clamp(1, 12);
    let key_width = activity_text_width(key)
        .max(8)
        .min(available.saturating_sub(value_reserve));
    (key_width, available.saturating_sub(key_width).max(1))
}

pub(super) fn metric_output_lines(
    context: OutputRenderContext,
    key: &str,
    value: &str,
) -> Vec<VisualOutputLine> {
    let (key_width, value_width) = metric_widths(key, value, context.width, 10);
    wrap_log_text(value, value_width)
        .into_iter()
        .map(|(text_start, value_line)| {
            let first = text_start == 0;
            let tag = if context.live_latest && first {
                "RUN"
            } else {
                ""
            };
            let rail = selected_output_style(
                live_output_rail_style(context.live_latest, context.frame),
                context.selected,
            );
            let mut spans = elapsed_prefix(context, first);
            spans.extend(if first {
                vec![
                    Span::styled(format!("{tag:<5} │ "), rail),
                    Span::styled(
                        fit_padded_column(key, key_width),
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
                text_start,
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
    let (key_width, value_width) = metric_widths(key, value, context.width, 11);
    let branch = if last { "└─ " } else { "├─ " };
    wrap_log_text(value, value_width)
        .into_iter()
        .map(|(text_start, value_line)| {
            let first = text_start == 0;
            let mut spans = elapsed_prefix(context, first);
            spans.extend([
                Span::styled(
                    if first {
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
                    if first {
                        fit_padded_column(key, key_width)
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
                text_start,
                line: Line::from(spans),
            }
        })
        .collect()
}

pub(super) fn metric_continuation_lines(
    context: OutputRenderContext,
    value: &str,
) -> Vec<VisualOutputLine> {
    let value_width = context.width.saturating_sub(9).max(1);
    wrap_log_text(value, value_width)
        .into_iter()
        .map(|(text_start, value_line)| {
            let first = text_start == 0;
            let tag = if context.live_latest && first {
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
                text_start,
                line: Line::from({
                    let mut spans = elapsed_prefix(context, first);
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
    wrap_event_text(text, context.width)
        .into_iter()
        .map(|(text_start, line)| {
            let first = text_start == 0;
            let mut spans = elapsed_prefix(context, first);
            spans.extend(if first {
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
                text_start,
                line: Line::from(spans),
            }
        })
        .collect()
}

fn wrap_log_text(text: &str, width: usize) -> Vec<(usize, String)> {
    wrap_log_rows(text, width, width)
}

fn wrap_event_text(text: &str, width: usize) -> Vec<(usize, String)> {
    // The first row has a six-cell tag; continuation rows add a rail.
    wrap_log_rows(
        text,
        width.saturating_sub(6).max(1),
        width.saturating_sub(8).max(1),
    )
}

fn wrap_log_rows(text: &str, first_width: usize, later_width: usize) -> Vec<(usize, String)> {
    let mut text_start = 0;

    let mut lines = Vec::new();
    let mut width = first_width;
    let mut current = String::new();
    let mut current_width = 0;
    // Never split the graphemes ratatui paints. Keep the monitor's CJK
    // budget for ambiguous characters, but never budget less than the renderer.
    for grapheme in Span::raw(text).styled_graphemes(Style::default()) {
        let symbol = grapheme.symbol;
        let cells = grapheme_cells(symbol);
        if current_width + cells > width && !current.is_empty() {
            let next_start = text_start + current.len();
            lines.push((text_start, current));
            text_start = next_start;
            width = later_width;
            current = String::new();
            current_width = 0;
        }
        current.push_str(symbol);
        current_width += cells;
    }
    lines.push((text_start, current));
    lines
}

pub(super) fn displayed_run_status(app: &MonitorApp) -> (&'static str, Color) {
    if app.history_view.is_none()
        && let Some(run) = &app.active_run
    {
        return if run.cancel_requested {
            ("STOPPING", Color::LightRed)
        } else {
            ("RUNNING", Color::Yellow)
        };
    }
    if app.visible_run_record().is_some_and(|run| !run.ok) {
        ("FAILED", Color::Red)
    } else if app.history_view.is_some() {
        ("DONE", Color::Green)
    } else {
        ("READY", Color::Green)
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
