use ratatui::layout::{Constraint, Direction, Layout, Rect};
use std::time::Duration;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub(super) fn pad_display_width(text: &str, width: usize) -> String {
    let len = text.width_cjk();
    if len >= width {
        return text.to_string();
    }
    format!("{}{}", text, " ".repeat(width - len))
}

pub(super) fn strip_ansi_codes(text: &str) -> String {
    let mut stripped = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && chars.peek() == Some(&'[') {
            chars.next();
            for code in chars.by_ref() {
                if code.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        stripped.push(ch);
    }
    stripped
}

pub(super) fn format_age(duration: Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else {
        format!("{}h ago", secs / 3600)
    }
}

pub(super) fn format_duration(duration: Duration) -> String {
    let secs = duration.as_secs();
    let millis = duration.subsec_millis();
    if secs < 60 {
        format!("{secs}.{millis:03}s")
    } else {
        format!("{}m{:02}s", secs / 60, secs % 60)
    }
}

pub(super) fn format_live_duration(duration: Duration) -> String {
    let total_centis = duration.as_millis() / 10;
    let secs = total_centis / 100;
    let centis = total_centis % 100;
    if secs < 60 {
        format!("{secs}.{centis:02}s")
    } else {
        format!("{}m{:02}.{:02}s", secs / 60, secs % 60, centis)
    }
}

pub(super) fn fit_text(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }

    let len = text.width_cjk();
    if len <= width {
        return text.to_string();
    }
    if width <= 3 {
        return take_display_width(text, width);
    }

    let mut fitted = take_display_width(text, width - 3);
    fitted.push_str("...");
    fitted
}

pub(super) fn fit_path(path: &str, width: usize) -> String {
    if path.width_cjk() <= width {
        return path.to_string();
    }
    if width <= 3 {
        return fit_text(path, width);
    }

    let tail_width = width - 3;
    let tail = take_display_width_from_end(path, tail_width);
    format!("...{tail}")
}

fn take_display_width(text: &str, width: usize) -> String {
    let mut taken = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let ch_width = ch.width_cjk().unwrap_or(0);
        if used + ch_width > width {
            break;
        }
        taken.push(ch);
        used += ch_width;
    }
    taken
}

fn take_display_width_from_end(text: &str, width: usize) -> String {
    let mut chars = Vec::new();
    let mut used = 0usize;
    for ch in text.chars().rev() {
        let ch_width = ch.width_cjk().unwrap_or(0);
        if used + ch_width > width {
            break;
        }
        chars.push(ch);
        used += ch_width;
    }
    chars.into_iter().rev().collect()
}

pub(super) fn percent_width(total: usize, percent: usize) -> usize {
    ((total * percent) / 100).max(1)
}

pub(super) fn contains(area: Rect, x: u16, y: u16) -> bool {
    x >= area.x
        && y >= area.y
        && x < area.x.saturating_add(area.width)
        && y < area.y.saturating_add(area.height)
}

pub(super) fn bordered_inner(area: Rect) -> Rect {
    Rect {
        x: area.x.saturating_add(1),
        y: area.y.saturating_add(1),
        width: area.width.saturating_sub(2),
        height: area.height.saturating_sub(2),
    }
}

pub(super) fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let percent_x = percent_x.min(100);
    let percent_y = percent_y.min(100);
    let vertical_margin = (100 - percent_y) / 2;
    let horizontal_margin = (100 - percent_x) / 2;
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(vertical_margin),
            Constraint::Percentage(percent_y),
            Constraint::Percentage(100 - percent_y - vertical_margin),
        ])
        .split(area);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(horizontal_margin),
            Constraint::Percentage(percent_x),
            Constraint::Percentage(100 - percent_x - horizontal_margin),
        ])
        .split(vertical[1]);
    horizontal[1]
}
