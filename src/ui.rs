use comfy_table::{
    Attribute, Cell, Color, ContentArrangement, Table, modifiers::UTF8_ROUND_CORNERS,
    presets::UTF8_FULL,
};
use console::{Term, style};
use indicatif::{ProgressBar, ProgressStyle};
use std::fmt::Display;
use std::io::{self, Write};
use std::time::Duration;

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

pub fn spinner(message: impl Into<String>) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.set_style(spinner_style());
    pb.set_message(message.into());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

pub fn progress(message: impl Into<String>, len: u64) -> ProgressBar {
    let pb = ProgressBar::new(len);
    pb.set_style(progress_style());
    pb.set_message(message.into());
    pb.enable_steady_tick(Duration::from_millis(80));
    pb
}

pub fn finish_read(pb: ProgressBar, message: impl Display) {
    pb.finish_and_clear();
    read(message);
}

pub fn finish_success(pb: ProgressBar, message: impl Display) {
    pb.finish_and_clear();
    success(message);
}

pub fn finish_saved(pb: ProgressBar, message: impl Display) {
    pb.finish_and_clear();
    saved(message);
}

pub fn suspend_progress<R>(pb: &ProgressBar, f: impl FnOnce() -> R) -> R {
    pb.suspend(f)
}

pub fn success(message: impl Display) {
    println!("{} {}", style(badge("OK")).green().bold(), message);
}

pub fn info(message: impl Display) {
    println!("{} {}", style(badge("INFO")).cyan().bold(), message);
}

pub fn read(message: impl Display) {
    println!("{} {}", style(badge("READ")).cyan().bold(), message);
}

pub fn saved(message: impl Display) {
    println!("{} {}", style(badge("SAVE")).magenta().bold(), message);
    flush_stdout();
}

pub fn skipped(message: impl Display) {
    println!("{} {}", style(badge("SKIP")).yellow().bold(), message);
}

pub fn warn(message: impl Display) {
    eprintln!("{} {}", style(badge("WARN")).yellow().bold(), message);
}

pub fn section(title: impl Display) {
    println!();
    println!("{}", style(format!("{title}")).bold().underlined());
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
    println!();
    println!("{}", style(format!("╭─ {}", title)).cyan().bold());

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
