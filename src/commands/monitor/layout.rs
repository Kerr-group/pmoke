use super::actions::monitor_actions;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;

const ACTIONS_MIN_WIDTH: u16 = 24;
const ACTIONS_OUTPUT_MIN_WIDTH: u16 = 40;
const ACTIONS_VERTICAL_WIDTH: u16 = 86;

pub(super) struct UiLayout {
    pub(super) tabs: Rect,
    pub(super) command_palette: Rect,
    pub(super) run_output: Rect,
}

impl UiLayout {
    pub(super) fn new(area: Rect, active_tab: usize) -> Self {
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(10),
                Constraint::Length(0),
            ])
            .split(area);
        let body = outer[1];
        let (tabs, active_panel) = active_panel_layout(body);
        let (command_palette, run_output) = if active_tab == 0 {
            actions_layout(active_panel)
        } else {
            (Rect::default(), Rect::default())
        };
        Self {
            tabs,
            command_palette,
            run_output,
        }
    }
}

pub(super) fn active_panel_layout(area: Rect) -> (Rect, Rect) {
    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(8)])
        .split(area);
    (body[0], body[1])
}

pub(super) fn actions_layout(area: Rect) -> (Rect, Rect) {
    let (command_area, _, run_output) = actions_full_layout(area);
    let (command_palette, _) = command_palette_layout(command_area);
    (command_palette, run_output)
}

pub(super) fn actions_full_layout(area: Rect) -> (Rect, Rect, Rect) {
    let command_width = actions_panel_width(area.width);
    let chunks = if area.width >= ACTIONS_VERTICAL_WIDTH {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(command_width),
                Constraint::Min(ACTIONS_OUTPUT_MIN_WIDTH),
            ])
            .split(area)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(8), Constraint::Min(10)])
            .split(area)
    };

    let output = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(6)])
        .split(chunks[1]);
    (chunks[0], output[0], output[1])
}

pub(super) fn actions_panel_width(available_width: u16) -> u16 {
    let content_width = monitor_actions()
        .iter()
        .enumerate()
        .map(|(idx, action)| {
            // Matches the command list row content:
            // marker + number + icon + command name + status message.
            display_width(&format!(
                "▌ {:02} ●  {} STP",
                idx + 1,
                action.command_name()
            ))
        })
        .max()
        .unwrap_or(ACTIONS_MIN_WIDTH);
    let panel_width = content_width.saturating_add(2);
    let max_width = available_width.saturating_sub(ACTIONS_OUTPUT_MIN_WIDTH);
    panel_width
        .max(ACTIONS_MIN_WIDTH)
        .min(max_width.max(ACTIONS_MIN_WIDTH))
}

fn display_width(text: &str) -> u16 {
    Line::from(text).width().try_into().unwrap_or(u16::MAX)
}

pub(super) fn command_palette_layout(area: Rect) -> (Rect, Rect) {
    let description_height = if area.height >= 12 {
        5
    } else if area.height >= 8 {
        4
    } else {
        0
    };
    if description_height == 0 {
        return (area, Rect::default());
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(4), Constraint::Length(description_height)])
        .split(area);
    (chunks[0], chunks[1])
}

pub(super) struct OutputSections {
    pub(super) status: Rect,
    pub(super) timeline: Rect,
    pub(super) log: Rect,
}

pub(super) fn output_inner_layout(area: Rect) -> OutputSections {
    if area.height == 0 {
        return OutputSections {
            status: area,
            timeline: Rect::default(),
            log: Rect::default(),
        };
    }

    if area.height < 9 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(area);
        return OutputSections {
            status: chunks[0],
            timeline: Rect::default(),
            log: chunks[1],
        };
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Min(1),
        ])
        .split(area);
    OutputSections {
        status: chunks[0],
        timeline: chunks[1],
        log: chunks[2],
    }
}

pub(super) fn output_visible_rows(log_area: Rect) -> usize {
    log_area.height.saturating_sub(1).max(1) as usize
}

pub(super) fn latest_event_feed_effect_area(
    log_content: Rect,
    visual_line_count: usize,
    visible_rows: usize,
    effective_scroll: usize,
) -> Option<Rect> {
    if effective_scroll != 0 || visual_line_count == 0 || log_content.height == 0 {
        return None;
    }

    let visible_count = visual_line_count
        .min(visible_rows)
        .min(log_content.height as usize);
    if visible_count == 0 || log_content.width <= 1 {
        return None;
    }

    Some(Rect {
        x: log_content.x,
        y: log_content.y + visible_count as u16 - 1,
        width: log_content.width.saturating_sub(1),
        height: 1,
    })
}
