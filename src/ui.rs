use comfy_table::{
    modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, Attribute, Cell, Color, ContentArrangement,
    Table,
};
use console::style;
use indicatif::{ProgressBar, ProgressStyle};
use std::fmt::Display;
use std::time::Duration;

fn badge(label: &str) -> String {
    format!("[{label:^7}]")
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

pub fn section_err(title: impl Display) {
    eprintln!();
    eprintln!("{}", style(format!("{title}")).bold().underlined());
}

pub fn summary_table(title: impl Display, headers: &[&str], rows: Vec<Vec<String>>) {
    section(title);
    println!("{}", table(headers, rows));
}

pub fn table(headers: &[&str], rows: Vec<Vec<String>>) -> Table {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic);
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
