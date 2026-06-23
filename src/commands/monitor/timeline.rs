use ratatui::{
    Frame,
    layout::Rect,
    prelude::{Color, Line, Modifier, Span, Style},
    widgets::Paragraph,
};
use tui_spinner::FluxFrames;

use super::{
    LogEntry, MonitorAction, MonitorApp, TIMELINE_BADGE_WIDTH, centered_text, strip_ansi_codes,
};

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
    let mut step_spans = Vec::new();
    for (idx, step) in timeline.steps.iter().enumerate() {
        if idx > 0 {
            step_spans.push(timeline_connector_span(step, motion_frame));
        }
        step_spans.extend(timeline_step_spans(step, motion_frame));
    }

    let lines = if area.height >= 3 {
        vec![
            Line::from(vec![
                Span::styled(
                    " RUN TIMELINE ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                if app.command_running() {
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
                    format!("{}/{} complete", timeline.done, timeline.total),
                    Style::default().fg(Color::Gray),
                ),
            ]),
            Line::from(step_spans),
            timeline_separator(area.width),
        ]
    } else if area.height >= 2 {
        vec![
            Line::from(vec![
                Span::styled(
                    " RUN TIMELINE ",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" "),
                if app.command_running() {
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
                    format!("{}/{} complete", timeline.done, timeline.total),
                    Style::default().fg(Color::Gray),
                ),
            ]),
            Line::from(step_spans),
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
        compact.extend(step_spans);
        vec![Line::from(compact)]
    };
    frame.render_widget(Paragraph::new(lines), area);
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
    spinner_frame(FluxFrames::BRAILLE, frame)
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
            "○".to_string(),
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
    centered_text(icon, TIMELINE_BADGE_WIDTH)
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
