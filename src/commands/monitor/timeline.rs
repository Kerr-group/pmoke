use ratatui::{
    Frame,
    layout::Rect,
    prelude::{Color, Line, Modifier, Span, Style},
    widgets::Paragraph,
};
use unicode_width::UnicodeWidthStr;

use super::{LogEntry, MonitorAction, MonitorApp, TIMELINE_BADGE_WIDTH, strip_ansi_codes};

const TIMELINE_COMPACT_BADGE_WIDTH: usize = 3;

pub(super) fn render_run_timeline(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let Some(timeline) = run_timeline(app) else {
        let line = Line::from(vec![
            Span::styled(" timeline ", Style::default().fg(Color::DarkGray)),
            Span::styled("idle", Style::default().fg(Color::Gray)),
        ]);
        let lines = if area.height >= 3 {
            vec![line, Line::default(), timeline_separator(area.width)]
        } else {
            vec![line]
        };
        frame.render_widget(Paragraph::new(lines), area);
        return;
    };

    let motion_frame = timeline_motion_frame(app);
    let step_lines = timeline_step_lines(&timeline.steps, area.width, area.height, motion_frame);
    let header = timeline_header_line(
        app.command_running(),
        timeline.done,
        timeline.total,
        motion_frame,
    );

    let lines = if area.height >= 3 {
        let mut lines = Vec::with_capacity(area.height as usize);
        lines.push(header);
        let available_step_rows = area.height.saturating_sub(1) as usize;
        let include_separator = step_lines.len() < available_step_rows;
        let take_steps = if include_separator {
            available_step_rows.saturating_sub(1)
        } else {
            available_step_rows
        };
        lines.extend(step_lines.into_iter().take(take_steps));
        if include_separator {
            lines.push(timeline_separator(area.width));
        }
        lines
    } else if area.height >= 2 {
        vec![
            header,
            timeline_compact_step_lines(&timeline.steps, area.width, 1, motion_frame)
                .into_iter()
                .next()
                .unwrap_or_default(),
        ]
    } else {
        let mut compact = vec![Span::styled(
            " TIMELINE ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )];
        compact.push(Span::raw(" "));
        compact.extend(timeline_compact_step_spans(&timeline.steps, motion_frame));
        vec![Line::from(compact)]
    };
    frame.render_widget(Paragraph::new(lines), area);
}

fn timeline_header_line(
    running: bool,
    done: usize,
    total: usize,
    motion_frame: usize,
) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            " RUN TIMELINE ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        if running {
            Span::styled(
                format!("{} ", timeline_spinner_symbol(motion_frame)),
                Style::default()
                    .fg(timeline_pulse_color(motion_frame))
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw("")
        },
        Span::styled(
            format!("{done}/{total} complete"),
            Style::default().fg(Color::Gray),
        ),
    ])
}

pub(super) fn timeline_step_lines(
    steps: &[TimelineStep],
    width: u16,
    height: u16,
    frame: usize,
) -> Vec<Line<'static>> {
    let full = timeline_full_step_line(steps, frame);
    if line_width(&full) <= width as usize {
        return vec![full];
    }

    timeline_compact_step_lines(steps, width, height.saturating_sub(1) as usize, frame)
}

fn timeline_full_step_line(steps: &[TimelineStep], frame: usize) -> Line<'static> {
    let mut spans = Vec::new();
    for (idx, step) in steps.iter().enumerate() {
        if idx > 0 {
            spans.push(timeline_connector_span(step, frame));
        }
        spans.extend(timeline_step_spans(step, frame));
    }
    Line::from(spans)
}

fn timeline_compact_step_lines(
    steps: &[TimelineStep],
    width: u16,
    max_lines: usize,
    frame: usize,
) -> Vec<Line<'static>> {
    if steps.is_empty() || max_lines == 0 {
        return Vec::new();
    }

    let width = width.max(1) as usize;
    let mut lines = Vec::new();
    let mut current = Vec::new();
    let mut current_width = 0usize;

    for (idx, step) in steps.iter().enumerate() {
        let mut item = Vec::new();
        if idx > 0 {
            item.push(timeline_compact_connector_span(step, frame));
        }
        item.push(timeline_compact_step_span(step, frame));
        let item_width = spans_width(&item);

        if !current.is_empty() && current_width + item_width > width && lines.len() + 1 < max_lines
        {
            lines.push(Line::from(current));
            current = vec![timeline_compact_step_span(step, frame)];
            current_width = spans_width(&current);
        } else {
            current_width += item_width;
            current.extend(item);
        }
    }

    lines.push(Line::from(current));
    lines
}

fn timeline_compact_step_spans(steps: &[TimelineStep], frame: usize) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (idx, step) in steps.iter().enumerate() {
        if idx > 0 {
            spans.push(timeline_compact_connector_span(step, frame));
        }
        spans.push(timeline_compact_step_span(step, frame));
    }
    spans
}

pub(super) fn timeline_separator(width: u16) -> Line<'static> {
    Line::styled(
        "─".repeat(width as usize),
        Style::default().fg(Color::DarkGray),
    )
}

pub(super) fn timeline_motion_frame(app: &MonitorApp) -> usize {
    app.active_run
        .as_ref()
        .map(|run| (run.started_at.elapsed().as_millis() / 150) as usize)
        .unwrap_or(0)
}

pub(super) fn spinner_frame(frames: &'static [char], frame: usize) -> char {
    frames[frame % frames.len()]
}

fn timeline_spinner_symbol(frame: usize) -> char {
    spinner_frame(&['|', '/', '-', '\\'], frame)
}

fn timeline_pending_symbol(frame: usize) -> char {
    spinner_frame(&['o', 'O'], frame)
}

fn timeline_pulse_color(frame: usize) -> Color {
    if frame.is_multiple_of(2) {
        Color::LightCyan
    } else {
        Color::Cyan
    }
}

fn timeline_connector_span(next_step: &TimelineStep, frame: usize) -> Span<'static> {
    let style = if matches!(
        next_step.state,
        TimelineStepState::Current | TimelineStepState::Stopping
    ) {
        Style::default()
            .fg(timeline_pulse_color(frame))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    Span::styled(" ─ ", style)
}

fn timeline_compact_connector_span(next_step: &TimelineStep, frame: usize) -> Span<'static> {
    let style = if matches!(
        next_step.state,
        TimelineStepState::Current | TimelineStepState::Stopping
    ) {
        Style::default()
            .fg(timeline_pulse_color(frame))
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    Span::styled("─", style)
}

pub(super) fn timeline_step_spans(step: &TimelineStep, frame: usize) -> Vec<Span<'static>> {
    let (icon, fg, bg, modifier) = match step.state {
        TimelineStepState::Done => ("✓".to_string(), Color::Black, Color::Green, Modifier::BOLD),
        TimelineStepState::Current => (
            timeline_spinner_symbol(frame).to_string(),
            Color::Black,
            timeline_pulse_color(frame),
            Modifier::BOLD,
        ),
        TimelineStepState::Pending => (
            timeline_pending_symbol(frame).to_string(),
            Color::DarkGray,
            Color::Reset,
            Modifier::empty(),
        ),
        TimelineStepState::Failed => (
            "×".to_string(),
            Color::Black,
            Color::LightRed,
            Modifier::BOLD,
        ),
        TimelineStepState::Stopping => (
            "!".to_string(),
            Color::Black,
            if frame.is_multiple_of(2) {
                Color::Yellow
            } else {
                Color::LightRed
            },
            Modifier::BOLD,
        ),
    };
    let badge_style = if matches!(step.state, TimelineStepState::Pending) {
        Style::default().fg(fg)
    } else {
        Style::default().fg(fg).bg(bg).add_modifier(modifier)
    };
    let label_style = match step.state {
        TimelineStepState::Done => Style::default().fg(Color::Green),
        TimelineStepState::Current => Style::default()
            .fg(timeline_pulse_color(frame))
            .add_modifier(Modifier::BOLD),
        TimelineStepState::Pending => Style::default().fg(Color::DarkGray),
        TimelineStepState::Failed => Style::default()
            .fg(Color::LightRed)
            .add_modifier(Modifier::BOLD),
        TimelineStepState::Stopping => Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    };

    vec![
        Span::styled(timeline_badge_cell(&icon), badge_style),
        Span::raw(" "),
        Span::styled(step.label.to_string(), label_style),
    ]
}

pub(super) fn timeline_badge_cell(icon: &str) -> String {
    let len = icon.chars().count();
    if len >= TIMELINE_BADGE_WIDTH {
        return icon.to_string();
    }
    let padding = TIMELINE_BADGE_WIDTH - len;
    let left = padding / 2;
    let right = padding - left;
    format!("{}{}{}", " ".repeat(left), icon, " ".repeat(right))
}

fn timeline_compact_step_span(step: &TimelineStep, frame: usize) -> Span<'static> {
    let (icon, fg, bg, modifier) = match step.state {
        TimelineStepState::Done => ("✓".to_string(), Color::Black, Color::Green, Modifier::BOLD),
        TimelineStepState::Current => (
            timeline_spinner_symbol(frame).to_string(),
            Color::Black,
            timeline_pulse_color(frame),
            Modifier::BOLD,
        ),
        TimelineStepState::Pending => (
            timeline_pending_symbol(frame).to_string(),
            Color::DarkGray,
            Color::Reset,
            Modifier::empty(),
        ),
        TimelineStepState::Failed => (
            "×".to_string(),
            Color::Black,
            Color::LightRed,
            Modifier::BOLD,
        ),
        TimelineStepState::Stopping => (
            "!".to_string(),
            Color::Black,
            if frame.is_multiple_of(2) {
                Color::Yellow
            } else {
                Color::LightRed
            },
            Modifier::BOLD,
        ),
    };
    let style = if matches!(step.state, TimelineStepState::Pending) {
        Style::default().fg(fg)
    } else {
        Style::default().fg(fg).bg(bg).add_modifier(modifier)
    };
    let icon = if matches!(
        step.state,
        TimelineStepState::Current | TimelineStepState::Stopping
    ) {
        timeline_compact_badge_cell(&icon)
    } else {
        icon
    };
    Span::styled(icon, style)
}

fn timeline_compact_badge_cell(icon: &str) -> String {
    let len = icon.chars().count();
    if len >= TIMELINE_COMPACT_BADGE_WIDTH {
        return icon.to_string();
    }
    let padding = TIMELINE_COMPACT_BADGE_WIDTH - len;
    let left = padding / 2;
    let right = padding - left;
    format!("{}{}{}", " ".repeat(left), icon, " ".repeat(right))
}

fn line_width(line: &Line<'_>) -> usize {
    spans_width(&line.spans)
}

fn spans_width(spans: &[Span<'_>]) -> usize {
    spans
        .iter()
        .map(|span| span.content.as_ref().width_cjk())
        .sum()
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum StageProgressState {
    Running,
    Complete,
    Failed,
    Stopping,
}

#[derive(Clone, Copy)]
struct StageSpec {
    label: &'static str,
    markers: &'static [&'static str],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TimelineStepState {
    Done,
    Current,
    Pending,
    Failed,
    Stopping,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct TimelineStep {
    pub(super) label: &'static str,
    pub(super) state: TimelineStepState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RunTimeline {
    pub(super) steps: Vec<TimelineStep>,
    pub(super) done: usize,
    pub(super) total: usize,
}

fn run_timeline(app: &MonitorApp) -> Option<RunTimeline> {
    if let Some(run) = &app.active_run {
        return timeline_for_action(
            run.action,
            &app.run_output,
            if run.cancel_requested {
                StageProgressState::Stopping
            } else {
                StageProgressState::Running
            },
        );
    }

    let record = app.last_run.as_ref()?;
    if record.ok {
        return timeline_for_action(record.action, &app.run_output, StageProgressState::Complete);
    }
    timeline_for_action(record.action, &app.run_output, StageProgressState::Failed)
}

pub(super) fn timeline_for_action(
    action: MonitorAction,
    output: &[LogEntry],
    state: StageProgressState,
) -> Option<RunTimeline> {
    let specs = action_stage_specs(action);
    if specs.is_empty() {
        return None;
    }

    let mut done = 0;
    for spec in &specs {
        if stage_done(spec, output) {
            done += 1;
        } else {
            break;
        }
    }
    if matches!(state, StageProgressState::Complete) {
        done = specs.len();
    }

    let steps = specs
        .iter()
        .enumerate()
        .map(|(idx, spec)| {
            let step_state = if idx < done {
                TimelineStepState::Done
            } else if idx == done && done < specs.len() {
                match state {
                    StageProgressState::Failed => TimelineStepState::Failed,
                    StageProgressState::Stopping => TimelineStepState::Stopping,
                    _ => TimelineStepState::Current,
                }
            } else {
                TimelineStepState::Pending
            };
            TimelineStep {
                label: spec.label,
                state: step_state,
            }
        })
        .collect::<Vec<_>>();

    Some(RunTimeline {
        steps,
        done,
        total: specs.len(),
    })
}

fn stage_done(spec: &StageSpec, output: &[LogEntry]) -> bool {
    output.iter().any(|entry| {
        let text = strip_ansi_codes(&entry.text).to_ascii_lowercase();
        spec.markers
            .iter()
            .any(|marker| text.contains(&marker.to_ascii_lowercase()))
    })
}

fn action_stage_specs(action: MonitorAction) -> Vec<StageSpec> {
    match action {
        MonitorAction::Show => vec![StageSpec {
            label: "Config",
            markers: &["config"],
        }],
        MonitorAction::Reference => vec![
            StageSpec {
                label: "Read",
                markers: &["fetched data"],
            },
            StageSpec {
                label: "FFT",
                markers: &["reference fft"],
            },
            StageSpec {
                label: "Fit",
                markers: &["reference fit", "reference signal fitted"],
            },
            StageSpec {
                label: "Plot",
                markers: &["reference plot completed"],
            },
        ],
        MonitorAction::Sensor => vec![
            StageSpec {
                label: "Read",
                markers: &["fetched data"],
            },
            StageSpec {
                label: "Integrate",
                markers: &["sensor integrations completed"],
            },
            StageSpec {
                label: "Plot",
                markers: &["sensor integral plot completed"],
            },
        ],
        MonitorAction::Li => vec![
            StageSpec {
                label: "Read",
                markers: &["fetched data"],
            },
            StageSpec {
                label: "Lock-in",
                markers: &["lock-in processing completed"],
            },
            StageSpec {
                label: "Save",
                markers: &["lock-in results"],
            },
            StageSpec {
                label: "Plot",
                markers: &["lock-in plot completed"],
            },
        ],
        MonitorAction::Phase => vec![
            StageSpec {
                label: "Fit",
                markers: &["phase rotation"],
            },
            StageSpec {
                label: "Save",
                markers: &["phase-rotated results"],
            },
            StageSpec {
                label: "Plot",
                markers: &["phase plot completed"],
            },
        ],
        MonitorAction::Kerr => vec![
            StageSpec {
                label: "Calculate",
                markers: &["kerr analysis completed"],
            },
            StageSpec {
                label: "Save",
                markers: &["kerr analysis results"],
            },
        ],
        MonitorAction::Analyze => vec![
            StageSpec {
                label: "Read",
                markers: &["fetched data"],
            },
            StageSpec {
                label: "Reference",
                markers: &["reference plot completed", "reference signal fitted"],
            },
            StageSpec {
                label: "Sensor",
                markers: &["sensor integrations completed"],
            },
            StageSpec {
                label: "Lock-in",
                markers: &["lock-in processing completed"],
            },
            StageSpec {
                label: "Phase",
                markers: &["phase analysis completed"],
            },
            StageSpec {
                label: "Kerr",
                markers: &["kerr analysis completed"],
            },
        ],
        #[cfg(feature = "hw")]
        MonitorAction::Single => vec![StageSpec {
            label: "Single",
            markers: &["single"],
        }],
        #[cfg(feature = "hw")]
        MonitorAction::Trigger => vec![StageSpec {
            label: "Trigger",
            markers: &["trigger"],
        }],
        #[cfg(feature = "hw")]
        MonitorAction::Autoshot => vec![
            StageSpec {
                label: "Single",
                markers: &["single"],
            },
            StageSpec {
                label: "Trigger",
                markers: &["trigger"],
            },
        ],
        #[cfg(feature = "hw")]
        MonitorAction::Fetch => vec![StageSpec {
            label: "Fetch",
            markers: &["fetched data", "fetched raw WORD"],
        }],
        #[cfg(feature = "hw")]
        MonitorAction::Automeasure => vec![
            StageSpec {
                label: "Single",
                markers: &["single"],
            },
            StageSpec {
                label: "Trigger",
                markers: &["trigger"],
            },
            StageSpec {
                label: "Fetch",
                markers: &["fetched data", "fetched raw WORD"],
            },
        ],
        #[cfg(feature = "hw")]
        MonitorAction::Process => vec![
            StageSpec {
                label: "Fetch",
                markers: &["fetched data", "fetched raw WORD"],
            },
            StageSpec {
                label: "Reference",
                markers: &["reference plot completed", "reference signal fitted"],
            },
            StageSpec {
                label: "Sensor",
                markers: &["sensor integrations completed"],
            },
            StageSpec {
                label: "Lock-in",
                markers: &["lock-in processing completed"],
            },
            StageSpec {
                label: "Phase",
                markers: &["phase analysis completed"],
            },
            StageSpec {
                label: "Kerr",
                markers: &["kerr analysis completed"],
            },
        ],
        #[cfg(feature = "hw")]
        MonitorAction::Auto => vec![
            StageSpec {
                label: "Single",
                markers: &["single"],
            },
            StageSpec {
                label: "Trigger",
                markers: &["trigger"],
            },
            StageSpec {
                label: "Fetch",
                markers: &["fetched data"],
            },
            StageSpec {
                label: "Reference",
                markers: &["reference plot completed", "reference signal fitted"],
            },
            StageSpec {
                label: "Sensor",
                markers: &["sensor integrations completed"],
            },
            StageSpec {
                label: "Lock-in",
                markers: &["lock-in processing completed"],
            },
            StageSpec {
                label: "Phase",
                markers: &["phase analysis completed"],
            },
            StageSpec {
                label: "Kerr",
                markers: &["kerr analysis completed"],
            },
        ],
    }
}
