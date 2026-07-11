use super::actions::monitor_actions;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::Line;

const WORKFLOW_MIN_WIDTH: u16 = 24;
const ACTIVITY_MIN_WIDTH: u16 = 48;
const WIDE_DASHBOARD_WIDTH: u16 = 92;
const COMPACT_INSPECTOR_HEIGHT: u16 = 7;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct UiLayout {
    pub(super) workflow: Rect,
    pub(super) inspector: Rect,
    pub(super) activity: Rect,
}

impl UiLayout {
    pub(super) fn new(area: Rect) -> Self {
        if area.width >= WIDE_DASHBOARD_WIDTH {
            let columns = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(workflow_panel_width(area.width)),
                    Constraint::Min(ACTIVITY_MIN_WIDTH),
                ])
                .split(area);
            let right = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(9), Constraint::Min(5)])
                .split(columns[1]);
            return Self {
                workflow: columns[0],
                inspector: right[0],
                activity: right[1],
            };
        }

        let inspector_height = if area.height >= 19 {
            COMPACT_INSPECTOR_HEIGHT
        } else {
            0
        };
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(8.min(area.height.saturating_sub(4))),
                Constraint::Length(inspector_height),
                Constraint::Min(4),
            ])
            .split(area);
        Self {
            workflow: rows[0],
            inspector: rows[1],
            activity: rows[2],
        }
    }
}

pub(super) fn workflow_panel_width(available_width: u16) -> u16 {
    let content_width = monitor_actions()
        .iter()
        .map(|action| display_width(&format!("▌   ●  {} STP", action.command_name())))
        .chain(
            super::actions::ActionGroup::ALL
                .iter()
                .map(|group| display_width(&format!("▌ ▾ {}", group.label()))),
        )
        .max()
        .unwrap_or(WORKFLOW_MIN_WIDTH);
    let panel_width = content_width.saturating_add(2);
    let max_width = available_width.saturating_sub(ACTIVITY_MIN_WIDTH);
    panel_width
        .max(WORKFLOW_MIN_WIDTH)
        .min(max_width.max(WORKFLOW_MIN_WIDTH))
}

fn display_width(text: &str) -> u16 {
    Line::from(text).width().try_into().unwrap_or(u16::MAX)
}

pub(super) fn workflow_layout(area: Rect) -> (Rect, Rect) {
    let description_height = if area.height >= 14 { 5 } else { 0 };
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

pub(super) fn config_panel_layout(area: Rect) -> (Rect, Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(7), Constraint::Min(6)])
        .split(area);
    (chunks[0], chunks[1])
}
