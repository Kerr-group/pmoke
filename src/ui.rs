use comfy_table::{
    Attribute, Cell, Color, ContentArrangement, Table, modifiers::UTF8_ROUND_CORNERS,
    presets::UTF8_FULL,
};
use console::{Term, style};
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use std::fmt::Display;
use std::io::{self, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use std::time::Instant;

const JSONL_OUTPUT_ENV: &str = "PMOKE_OUTPUT";
const JSONL_OUTPUT_VALUE: &str = "jsonl";
const OUTPUT_STAGE_ENV: &str = "PMOKE_STAGE";
static EVENT_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static PROGRESS_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static EVENT_EPOCH: OnceLock<Instant> = OnceLock::new();

pub(crate) fn initialize_output() {
    EVENT_EPOCH.get_or_init(Instant::now);
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EventLevel {
    Success,
    Info,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EventKind {
    Status,
    Read,
    Save,
    Skip,
    Progress,
    Section,
    Metric,
    System,
    Raw,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub(crate) struct UiEvent {
    #[serde(rename = "type")]
    pub(crate) event_type: String,
    pub(crate) sequence: u64,
    pub(crate) elapsed_ms: u64,
    pub(crate) level: EventLevel,
    pub(crate) kind: EventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stage: Option<String>,
    pub(crate) message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) fields: Vec<(String, String)>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) progress_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) progress_current: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) progress_total: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) duration_ms: Option<u64>,
}

impl UiEvent {
    fn new(
        level: EventLevel,
        kind: EventKind,
        message: impl Display,
        fields: Vec<(String, String)>,
    ) -> Self {
        let elapsed = EVENT_EPOCH.get_or_init(Instant::now).elapsed();
        Self {
            event_type: "event".to_string(),
            sequence: EVENT_SEQUENCE.fetch_add(1, Ordering::Relaxed),
            elapsed_ms: u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
            level,
            kind,
            stage: std::env::var(OUTPUT_STAGE_ENV).ok(),
            message: message.to_string(),
            fields,
            progress_id: None,
            progress_current: None,
            progress_total: None,
            duration_ms: None,
        }
    }
}

pub(crate) fn parse_jsonl_event(line: &str) -> Option<UiEvent> {
    let event = serde_json::from_str::<UiEvent>(line).ok()?;
    (event.event_type == "event").then_some(event)
}

fn jsonl_output_enabled() -> bool {
    std::env::var(JSONL_OUTPUT_ENV).as_deref() == Ok(JSONL_OUTPUT_VALUE)
}

fn emit_event(event: &UiEvent, human: impl FnOnce()) {
    if !jsonl_output_enabled() {
        human();
        return;
    }
    let stdout = io::stdout();
    let mut output = stdout.lock();
    if serde_json::to_writer(&mut output, event).is_ok() {
        let _ = output.write_all(b"\n");
        let _ = output.flush();
    }
}

fn badge(label: &str) -> String {
    format!("[{label:^6}]")
}

fn spinner_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.cyan} {msg}")
        .unwrap_or_else(|_| ProgressStyle::default_spinner())
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
}

fn progress_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.cyan} {msg} [{bar:32.cyan/blue}] {pos}/{len}")
        .unwrap_or_else(|_| ProgressStyle::default_bar())
        .progress_chars("=>-")
        .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
}

pub fn fmt_duration(duration: Duration) -> String {
    format!("{duration:.2?}")
}

#[derive(Clone)]
pub struct UiProgress {
    bar: ProgressBar,
    state: Arc<ProgressState>,
}

struct ProgressState {
    id: String,
    total: Option<u64>,
    current: AtomicU64,
    message: Mutex<String>,
    started_at: Instant,
}

impl UiProgress {
    fn new(bar: ProgressBar, message: String, total: Option<u64>) -> Self {
        let progress = Self {
            bar,
            state: Arc::new(ProgressState {
                id: format!(
                    "progress:{}",
                    PROGRESS_SEQUENCE.fetch_add(1, Ordering::Relaxed)
                ),
                total,
                current: AtomicU64::new(0),
                message: Mutex::new(message),
                started_at: Instant::now(),
            }),
        };
        progress.emit_update();
        progress
    }

    pub fn set_message(&self, message: impl Into<String>) {
        let message = message.into();
        self.bar.set_message(message.clone());
        if let Ok(mut current) = self.state.message.lock() {
            *current = message;
        }
        self.emit_update();
    }

    pub fn inc(&self, delta: u64) {
        self.bar.inc(delta);
        self.state.current.fetch_add(delta, Ordering::Relaxed);
        self.emit_update();
    }

    fn finish_and_clear(&self) {
        self.bar.finish_and_clear();
    }

    fn suspend<R>(&self, f: impl FnOnce() -> R) -> R {
        self.bar.suspend(f)
    }

    fn emit_update(&self) {
        if !jsonl_output_enabled() {
            return;
        }
        let message = self
            .state
            .message
            .lock()
            .map(|message| message.clone())
            .unwrap_or_else(|_| "working".to_string());
        let mut event = UiEvent::new(EventLevel::Info, EventKind::Progress, message, Vec::new());
        event.progress_id = Some(self.state.id.clone());
        event.progress_current = self
            .state
            .total
            .map(|_| self.state.current.load(Ordering::Relaxed));
        event.progress_total = self.state.total;
        emit_event(&event, || {});
    }

    fn emit_completion(&self, level: EventLevel, kind: EventKind, message: String) {
        if !jsonl_output_enabled() {
            return;
        }
        let mut event = UiEvent::new(level, kind, message, Vec::new());
        event.progress_id = Some(self.state.id.clone());
        event.duration_ms =
            Some(u64::try_from(self.state.started_at.elapsed().as_millis()).unwrap_or(u64::MAX));
        emit_event(&event, || {});
    }
}

pub fn spinner(message: impl Into<String>) -> UiProgress {
    let message = message.into();
    let pb = if jsonl_output_enabled() {
        ProgressBar::hidden()
    } else {
        let pb = ProgressBar::new_spinner();
        pb.set_style(spinner_style());
        pb.set_message(message.clone());
        pb.enable_steady_tick(Duration::from_millis(80));
        pb
    };
    UiProgress::new(pb, message, None)
}

pub fn progress(message: impl Into<String>, len: u64) -> UiProgress {
    let message = message.into();
    let pb = if jsonl_output_enabled() {
        ProgressBar::hidden()
    } else {
        let pb = ProgressBar::new(len);
        pb.set_style(progress_style());
        pb.set_message(message.clone());
        pb.enable_steady_tick(Duration::from_millis(80));
        pb
    };
    UiProgress::new(pb, message, Some(len))
}

pub fn finish_read(pb: UiProgress, message: impl Display) {
    pb.finish_and_clear();
    let message = message.to_string();
    if jsonl_output_enabled() {
        pb.emit_completion(EventLevel::Info, EventKind::Read, message);
    } else {
        read(message);
    }
}

pub fn finish_success(pb: UiProgress, message: impl Display) {
    pb.finish_and_clear();
    let message = message.to_string();
    if jsonl_output_enabled() {
        pb.emit_completion(EventLevel::Success, EventKind::Status, message);
    } else {
        success(message);
    }
}

pub fn finish_saved(pb: UiProgress, message: impl Display) {
    pb.finish_and_clear();
    let message = message.to_string();
    if jsonl_output_enabled() {
        pb.emit_completion(EventLevel::Success, EventKind::Save, message);
    } else {
        saved(message);
    }
}

pub fn finish_cancelled(pb: UiProgress, message: impl Display) {
    pb.finish_and_clear();
    let message = message.to_string();
    if jsonl_output_enabled() {
        pb.emit_completion(EventLevel::Info, EventKind::Skip, message);
    } else {
        skipped(message);
    }
}

pub fn finish_warning(pb: UiProgress, message: impl Display) {
    pb.finish_and_clear();
    let message = message.to_string();
    if jsonl_output_enabled() {
        pb.emit_completion(EventLevel::Warning, EventKind::Status, message);
    } else {
        warn(message);
    }
}

pub fn suspend_progress<R>(pb: &UiProgress, f: impl FnOnce() -> R) -> R {
    pb.suspend(f)
}

pub fn success(message: impl Display) {
    let event = UiEvent::new(EventLevel::Success, EventKind::Status, &message, Vec::new());
    emit_event(&event, || {
        println!("{} {}", style(badge("OK")).green().bold(), message);
    });
}

pub fn info(message: impl Display) {
    let event = UiEvent::new(EventLevel::Info, EventKind::Status, &message, Vec::new());
    emit_event(&event, || {
        println!("{} {}", style(badge("INFO")).cyan().bold(), message);
    });
}

pub fn read(message: impl Display) {
    let event = UiEvent::new(EventLevel::Info, EventKind::Read, &message, Vec::new());
    emit_event(&event, || {
        println!("{} {}", style(badge("READ")).cyan().bold(), message);
    });
}

pub fn saved(message: impl Display) {
    let event = UiEvent::new(EventLevel::Success, EventKind::Save, &message, Vec::new());
    emit_event(&event, || {
        println!("{} {}", style(badge("SAVE")).magenta().bold(), message);
        flush_stdout();
    });
}

pub fn skipped(message: impl Display) {
    let event = UiEvent::new(EventLevel::Info, EventKind::Skip, &message, Vec::new());
    emit_event(&event, || {
        println!("{} {}", style(badge("SKIP")).yellow().bold(), message);
    });
}

pub fn warn(message: impl Display) {
    let event = UiEvent::new(EventLevel::Warning, EventKind::Status, &message, Vec::new());
    emit_event(&event, || {
        eprintln!("{} {}", style(badge("WARN")).yellow().bold(), message);
    });
}

pub fn error(message: impl Display) {
    let event = UiEvent::new(EventLevel::Error, EventKind::Status, &message, Vec::new());
    emit_event(&event, || {
        eprintln!("{} {}", style(badge("ERR")).red().bold(), message);
    });
}

pub fn section(title: impl Display) {
    let event = UiEvent::new(EventLevel::Info, EventKind::Section, &title, Vec::new());
    emit_event(&event, || {
        println!();
        println!("{}", style(format!("{title}")).bold().underlined());
    });
}

pub fn summary_table(title: impl Display, headers: &[&str], rows: Vec<Vec<String>>) {
    let _ = headers;
    summary_panel(
        title,
        rows.into_iter()
            .filter_map(|row| {
                let key = row.first()?.clone();
                let value = row.get(1).cloned().unwrap_or_default();
                Some((key, value))
            })
            .collect(),
    );
    flush_stdout();
}

pub fn settings_table(title: impl Display, rows: Vec<(String, String)>) {
    summary_panel(title, rows);
}

fn flush_stdout() {
    let _ = io::stdout().flush();
}

fn summary_panel(title: impl Display, rows: Vec<(String, String)>) {
    let title = title.to_string();
    let event = UiEvent::new(EventLevel::Info, EventKind::Section, &title, rows.clone());
    emit_event(&event, || summary_panel_human(&title, rows));
}

fn summary_panel_human(title: &str, rows: Vec<(String, String)>) {
    println!();
    println!("{}", style(format!("╭─ {title}")).cyan().bold());

    if rows.is_empty() {
        println!("{} {}", style("│").cyan(), style("empty").dim());
        println!("{}", style("╰─").cyan());
        return;
    }

    let key_width = rows
        .iter()
        .map(|(key, _)| key.chars().count())
        .max()
        .unwrap_or(0)
        .min(22);
    let width = output_table_width() as usize;
    let value_width = width.saturating_sub(key_width + 7).max(18);

    for (key, value) in rows {
        let mut wrapped = wrap_text(&value, value_width).into_iter();
        let first = wrapped.next().unwrap_or_default();
        println!(
            "{} {}  {}",
            style("│").cyan(),
            style(format!("{key:key_width$}")).cyan(),
            style(first).white()
        );
        let continuation_indent = " ".repeat(key_width);
        for line in wrapped {
            println!(
                "{} {}  {}",
                style("│").cyan(),
                continuation_indent,
                style(line).white()
            );
        }
    }

    println!("{}", style("╰─").cyan());
}

pub fn table(headers: &[&str], rows: Vec<Vec<String>>) -> Table {
    table_with_width(headers, rows, output_table_width())
}

fn table_with_width(headers: &[&str], rows: Vec<Vec<String>>, width: u16) -> Table {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_width(width.max(24));
    table.set_header(
        headers
            .iter()
            .map(|header| {
                Cell::new(header)
                    .fg(Color::Cyan)
                    .add_attribute(Attribute::Bold)
            })
            .collect::<Vec<_>>(),
    );
    for row in rows {
        table.add_row(row);
    }
    table
}

fn output_table_width() -> u16 {
    std::env::var("PMOKE_TABLE_WIDTH")
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .filter(|width| *width >= 24)
        .or_else(|| {
            Term::stdout()
                .size_checked()
                .map(|(_, columns)| columns.saturating_sub(1))
        })
        .unwrap_or(100)
        .max(24)
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }

    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_len = 0;
    for word in text.split_whitespace() {
        let word_len = word.chars().count();
        if current_len > 0 && current_len + 1 + word_len > width {
            lines.push(std::mem::take(&mut current));
            current_len = 0;
        }
        if current_len > 0 {
            current.push(' ');
            current_len += 1;
        }
        current.push_str(word);
        current_len += word_len;

        while current_len > width {
            let split_at = current
                .char_indices()
                .nth(width)
                .map(|(idx, _)| idx)
                .unwrap_or(current.len());
            let rest = current.split_off(split_at);
            lines.push(std::mem::replace(&mut current, rest));
            current_len = current.chars().count();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use console::measure_text_width;

    #[test]
    fn badge_centers_even_width_labels() {
        assert_eq!(badge("OK"), "[  OK  ]");
        assert_eq!(badge("READ"), "[ READ ]");
        assert_eq!(badge("SAVE"), "[ SAVE ]");
    }

    #[test]
    fn jsonl_event_round_trips_with_structured_fields() {
        let event = UiEvent {
            event_type: "event".to_string(),
            sequence: 7,
            elapsed_ms: 1234,
            level: EventLevel::Warning,
            kind: EventKind::Section,
            stage: Some("lockin".to_string()),
            message: "Lock-in settings".to_string(),
            fields: vec![("output rate".to_string(), "500 kHz".to_string())],
            progress_id: Some("lockin:ch3".to_string()),
            progress_current: Some(4),
            progress_total: Some(6),
            duration_ms: None,
        };
        let encoded = serde_json::to_string(&event).unwrap();
        assert_eq!(parse_jsonl_event(&encoded), Some(event));
        assert!(parse_jsonl_event("plain output").is_none());
        assert!(parse_jsonl_event(r#"{"type":"other"}"#).is_none());
    }

    #[test]
    fn ui_progress_tracks_shared_position_and_message_state() {
        let progress = UiProgress::new(ProgressBar::hidden(), "starting".to_string(), Some(4));
        let worker = progress.clone();
        worker.set_message("channel 2");
        worker.inc(2);

        assert_eq!(progress.state.current.load(Ordering::Relaxed), 2);
        assert_eq!(progress.state.total, Some(4));
        assert_eq!(progress.state.message.lock().unwrap().as_str(), "channel 2");
    }

    #[test]
    fn table_with_width_wraps_long_values_inside_table() {
        let rendered = table_with_width(
            &["Setting", "Value"],
            vec![vec![
                "sample_rate".to_string(),
                "5.000000e8 Hz, output_rate=5.000000e6 Hz, stride_samples=100".to_string(),
            ]],
            60,
        )
        .to_string();

        assert!(
            rendered.lines().all(|line| measure_text_width(line) <= 60),
            "{rendered}"
        );
    }
}
