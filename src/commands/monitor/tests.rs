use super::*;
use crate::config::{
    Channel, ConfigDiagnostic, DiagnosticKind, Fetch, Kerr, KerrType, Lockin, LockinLpfKind, Phase,
    Plot, Pulse, Reference, Roles, Screenshot, Window,
};

fn test_app() -> MonitorApp {
    MonitorApp::new(
        "config.toml".to_string(),
        ConfigLoad::Diagnostics(ConfigDiagnostics {
            version: None,
            warnings: Vec::new(),
            diagnostics: Vec::new(),
            normalized: None,
        }),
    )
}

fn ready_test_app(channel_count: u8) -> MonitorApp {
    let signal_ch = (1..=channel_count.min(6)).collect::<Vec<_>>();
    let window = Window {
        start: 0.0,
        end: 1.0,
    };
    MonitorApp::new(
        "config.toml".to_string(),
        ConfigLoad::Ready {
            config: Config {
                version: 3,
                instruments: None,
                fetch: Fetch::default(),
                screenshot: Screenshot::default(),
                plot: Plot::default(),
                source_path: "config.toml".into(),
                legacy_timebase: None,
                roles: Roles {
                    sensor_ch: vec![1],
                    reference_ch: 1,
                    signal_ch,
                },
                channels: (1..=channel_count)
                    .map(|index| Channel {
                        index,
                        factor: None,
                        scale_to_abs_max: None,
                        label: Some(format!("channel {index}")),
                        unit_out: None,
                    })
                    .collect(),
                pulse: Pulse {
                    bg_window_before: window,
                    bg_window_after: window,
                },
                reference: Reference {
                    fft_window: window,
                    stride_samples: 1,
                    window_samples: 1,
                },
                lockin: Lockin {
                    workers: 1,
                    stride_samples: 1,
                    lpf_kind: LockinLpfKind::FirZeroPhase,
                    lpf_half_window_cycles: 1.0,
                    lpf_cutoff_hz: Some(1.0),
                    lpf_cutoff_ref_ratio: None,
                    lpf_stopband_atten_db: 60.0,
                    lpf_sync_average_cycles: 1.0,
                    lpf_iir_order: 2,
                    lpf_debug_output: false,
                    lpf_debug_label: None,
                    lpf_debug_overwrite: false,
                    snr_background_window: None,
                    snr_signal_window: None,
                },
                phase: Phase {
                    m_omega_t0_offset: Vec::new(),
                },
                kerr: Kerr {
                    use_sensor_ch: 1,
                    kerr_type: KerrType::Standard,
                    factor: 1.0,
                },
            },
            warnings: Vec::new(),
        },
    )
}

#[test]
fn keeps_lockin_settings_in_live_output() {
    let mut app = test_app();
    app.push_output(
            OutputStream::Stdout,
            "\x1b[1mLock-in settings\x1b[0m\n  \x1b[36m•\x1b[0m lpf_kind = SyncIirZeroPhase\n  • tap_count = 17121\n[  OK   ] done",
        );

    assert_eq!(
        app.run_output
            .iter()
            .map(|entry| strip_ansi_codes(&entry.text))
            .collect::<Vec<_>>(),
        vec![
            "Lock-in settings".to_string(),
            "  • lpf_kind = SyncIirZeroPhase".to_string(),
            "  • tap_count = 17121".to_string(),
            "[  OK   ] done".to_string(),
        ]
    );
}

#[test]
fn tui_tick_uses_60fps_while_effects_are_running() {
    let mut app = test_app();

    assert_eq!(tui_frame_tick(&app), TUI_IDLE_TICK);
    app.push_output(OutputStream::Stdout, "[ INFO ] running");

    assert_eq!(tui_frame_tick(&app), TUI_ANIMATION_TICK);
    assert_eq!(TUI_ANIMATION_TICK, Duration::from_millis(16));
}

#[test]
fn strips_csi_ansi_codes() {
    assert_eq!(strip_ansi_codes("\x1b[1;36mLock-in\x1b[0m"), "Lock-in");
}

#[test]
fn wide_actions_layout_keeps_output_visible() {
    let area = Rect::new(0, 0, 120, 28);
    let (_, _, output) = actions_full_layout(area);

    assert!(output.height >= 6);
}

#[test]
fn actions_panel_width_fits_command_rows_without_fixed_padding() {
    let area = Rect::new(0, 0, 120, 28);
    let (commands, _, output) = actions_full_layout(area);

    assert!(commands.width < 36);
    assert_eq!(commands.width, actions_panel_width(area.width));
    assert!(output.width >= 40);
}

#[test]
fn output_table_width_fits_inside_live_output_text_area() {
    let mut app = test_app();
    app.active_tab = 0;
    let area = Rect::new(0, 0, 120, 28);
    let log_content = output_log_content_area(&app, area).expect("output area exists");
    let table_width = output_table_width_for_area(&app, area).expect("table width exists");

    assert!(table_width <= log_content.width.saturating_sub(OUTPUT_PREFIX_WIDTH + 1));
    assert!(table_width >= 24);
}

#[test]
fn output_layout_uses_status_and_log_regions_only() {
    let sections = output_inner_layout(Rect::new(0, 0, 80, 20));

    assert_eq!(sections.status.height, 1);
    assert_eq!(sections.timeline.height, 3);
    assert!(sections.log.height > 0);
}

#[test]
fn output_layout_hides_timeline_when_too_short() {
    let sections = output_inner_layout(Rect::new(0, 0, 80, 6));

    assert_eq!(sections.status.height, 1);
    assert_eq!(sections.timeline.height, 0);
    assert!(sections.log.height > 0);
}

#[test]
fn latest_event_feed_effect_area_targets_last_visible_row() {
    let area = latest_event_feed_effect_area(Rect::new(3, 5, 40, 8), 12, 8, 0)
        .expect("latest row should be visible");

    assert_eq!(area, Rect::new(3, 12, 39, 1));
    assert_eq!(
        latest_event_feed_effect_area(Rect::new(3, 5, 40, 8), 12, 8, 1),
        None
    );
}

#[test]
fn pushing_output_starts_event_feed_effect() {
    let mut app = test_app();

    app.push_output(OutputStream::Stdout, "[ INFO ] running");

    assert!(app.effects.is_running());
}

#[test]
fn hidden_event_feed_effects_advance_to_idle() {
    let mut app = test_app();
    app.push_output(OutputStream::Stdout, "[ INFO ] running");
    let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 4, 1));

    process_event_feed_effects(
        &mut app,
        FxDuration::from_millis(EVENT_FEED_EFFECT_MS * 2),
        &mut buffer,
        None,
    );

    assert!(!app.effects.is_running());
    assert_eq!(tui_frame_tick(&app), TUI_IDLE_TICK);
}

#[test]
fn timeline_separator_uses_full_available_width() {
    let line = timeline_separator(8);

    assert_eq!(line.spans[0].content.as_ref(), "────────");
}

#[test]
fn current_timeline_step_animates_with_motion_frame() {
    let step = TimelineStep {
        label: "Lock-in",
        state: TimelineStepState::Current,
    };

    let first = timeline_step_spans(&step, 0);
    let second = timeline_step_spans(&step, 1);

    assert_ne!(first[0].content, second[0].content);
    assert_eq!(first[0].content.as_ref(), "  ◜  ");
    assert_eq!(second[0].content.as_ref(), "  ◝  ");
}

#[test]
fn timeline_badges_are_centered_in_fixed_cells() {
    assert_eq!(timeline_badge_cell("◜"), "  ◜  ");
    assert_eq!(timeline_badge_cell("◝"), "  ◝  ");
    assert_eq!(timeline_badge_cell("✓"), "  ✓  ");
    assert_eq!(timeline_badge_cell("░"), "  ░  ");
    assert_eq!(timeline_badge_cell("▒"), "  ▒  ");
    assert_eq!(timeline_badge_cell("!"), "  !  ");
}

#[test]
fn pending_timeline_step_animates_in_centered_cell() {
    let step = TimelineStep {
        label: "Read",
        state: TimelineStepState::Pending,
    };

    let first = timeline_step_spans(&step, 0);
    let second = timeline_step_spans(&step, 1);

    assert_ne!(first[0].content, second[0].content);
    assert_eq!(first[0].content.as_ref(), "  ░  ");
    assert_eq!(second[0].content.as_ref(), "  ▒  ");
}

#[test]
fn compact_pending_timeline_step_animates_in_centered_cells() {
    let steps = vec![
        TimelineStep {
            label: "Read",
            state: TimelineStepState::Pending,
        },
        TimelineStep {
            label: "Reference",
            state: TimelineStepState::Pending,
        },
    ];

    let first = timeline_step_lines(&steps, 7, 2, 0);
    let second = timeline_step_lines(&steps, 7, 2, 1);
    let first_text = first[0]
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    let second_text = second[0]
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert_eq!(first_text, " ░ ─ ░ ");
    assert_eq!(second_text, " ▒ ─ ▒ ");
    assert_eq!(
        unicode_width::UnicodeWidthStr::width(first[0].spans[0].content.as_ref()),
        3
    );
    assert_eq!(
        unicode_width::UnicodeWidthStr::width(first[0].spans[2].content.as_ref()),
        3
    );
    assert_eq!(
        unicode_width::UnicodeWidthStr::width(second[0].spans[0].content.as_ref()),
        3
    );
    assert_eq!(
        unicode_width::UnicodeWidthStr::width(second[0].spans[2].content.as_ref()),
        3
    );
}

#[test]
fn compact_current_timeline_step_uses_centered_cell() {
    let steps = vec![
        TimelineStep {
            label: "Read",
            state: TimelineStepState::Current,
        },
        TimelineStep {
            label: "Reference",
            state: TimelineStepState::Pending,
        },
    ];

    let lines = timeline_step_lines(&steps, 12, 2, 0);
    let rendered = lines[0]
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert_eq!(rendered, " ◜ ─ ░ ");
    assert_eq!(lines[0].spans[0].content.as_ref(), " ◜ ");
    assert_eq!(lines[0].spans[2].content.as_ref(), " ░ ");
    assert_eq!(
        unicode_width::UnicodeWidthStr::width(lines[0].spans[0].content.as_ref()),
        3
    );
    assert_eq!(
        unicode_width::UnicodeWidthStr::width(lines[0].spans[2].content.as_ref()),
        3
    );
    assert!(
        lines[0]
            .spans
            .iter()
            .map(|span| unicode_width::UnicodeWidthStr::width(span.content.as_ref()))
            .sum::<usize>()
            <= 12
    );
}

#[test]
fn narrow_timeline_wraps_compact_steps_without_dropping_stages() {
    let steps = vec![
        TimelineStep {
            label: "Read",
            state: TimelineStepState::Done,
        },
        TimelineStep {
            label: "Reference",
            state: TimelineStepState::Done,
        },
        TimelineStep {
            label: "Sensor",
            state: TimelineStepState::Current,
        },
        TimelineStep {
            label: "Lock-in",
            state: TimelineStepState::Pending,
        },
        TimelineStep {
            label: "Phase",
            state: TimelineStepState::Pending,
        },
        TimelineStep {
            label: "Kerr",
            state: TimelineStepState::Pending,
        },
    ];

    let lines = timeline_step_lines(&steps, 14, 3, 0);
    let rendered = lines
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert_eq!(lines.len(), 2);
    assert!(lines.iter().all(|line| {
        line.spans
            .iter()
            .map(|span| unicode_width::UnicodeWidthStr::width(span.content.as_ref()))
            .sum::<usize>()
            <= 14
    }));
    assert_eq!(rendered.chars().filter(|ch| *ch == '✓').count(), 2);
    assert_eq!(rendered.chars().filter(|ch| *ch == '░').count(), 3);
    assert!(rendered.contains('◜'));
}

#[test]
fn wide_timeline_keeps_labeled_steps() {
    let steps = vec![
        TimelineStep {
            label: "Read",
            state: TimelineStepState::Done,
        },
        TimelineStep {
            label: "Reference",
            state: TimelineStepState::Current,
        },
    ];

    let lines = timeline_step_lines(&steps, 80, 3, 0);
    let rendered = lines[0]
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert_eq!(lines.len(), 1);
    assert!(rendered.contains("Read"));
    assert!(rendered.contains("Reference"));
}

#[test]
fn selected_output_text_strips_ansi_codes() {
    let mut app = test_app();
    app.push_output(OutputStream::Stdout, "\x1b[31mhello\x1b[0m");
    app.output_selected = Some(0);

    assert_eq!(app.selected_output_text().as_deref(), Some("hello"));
}

#[test]
fn classifies_common_output_lines_for_highlighting() {
    assert_eq!(
        classify_log_entry(OutputStream::Stdout, "[  OK   ] done"),
        LogKind::Success
    );
    assert_eq!(
        classify_log_entry(OutputStream::Stdout, "[ INFO  ] f_ref = 1.0 MHz"),
        LogKind::Info
    );
    assert_eq!(
        classify_log_entry(OutputStream::Stdout, "🛠️ Fit result:"),
        LogKind::Fit
    );
    assert_eq!(
        classify_log_entry(OutputStream::Stderr, "UserWarning: slow legend"),
        LogKind::Warning
    );
    assert_eq!(
        classify_log_entry(
            OutputStream::Stderr,
            "[ WARN ] legacy config v2: [timebase] is deprecated"
        ),
        LogKind::Warning
    );
    assert_eq!(
        classify_log_entry(OutputStream::Stderr, "(warn) read timeout status"),
        LogKind::Warning
    );
    assert_eq!(
        classify_log_entry(OutputStream::Stderr, "(info) gpib.conf was applied"),
        LogKind::Info
    );
    assert_eq!(
        classify_log_entry(OutputStream::Stderr, "Error: raw metadata is missing"),
        LogKind::Error
    );
    assert_eq!(
        classify_log_entry(OutputStream::Stdout, "╭─────────┬────────╮"),
        LogKind::Section
    );
    assert_eq!(
        classify_log_entry(OutputStream::Stdout, "│ Setting ┆ Value  │"),
        LogKind::Metric
    );
}

#[test]
fn analyze_timeline_marks_done_current_and_pending_steps() {
    let output = vec![
        LogEntry {
            stream: OutputStream::Stdout,
            text: "[ READ  ] fetched data: 4 channels".to_string(),
        },
        LogEntry {
            stream: OutputStream::Stdout,
            text: "[  OK   ] reference plot completed".to_string(),
        },
    ];

    let timeline =
        timeline_for_action(MonitorAction::Analyze, &output, StageProgressState::Running)
            .expect("analyze has timeline stages");

    assert_eq!(timeline.done, 2);
    assert_eq!(timeline.total, 6);
    assert_eq!(timeline.steps[0].state, TimelineStepState::Done);
    assert_eq!(timeline.steps[1].state, TimelineStepState::Done);
    assert_eq!(timeline.steps[2].state, TimelineStepState::Current);
    assert_eq!(timeline.steps[3].state, TimelineStepState::Pending);
}

#[cfg(feature = "hw")]
#[test]
fn failed_screenshot_timeline_marks_stage_failed_instead_of_pending() {
    let timeline = timeline_for_action(MonitorAction::Screenshot, &[], StageProgressState::Failed)
        .expect("screenshot has a timeline stage");

    assert_eq!(timeline.done, 0);
    assert_eq!(timeline.total, 1);
    assert_eq!(timeline.steps[0].state, TimelineStepState::Failed);
}

#[test]
fn visual_output_lines_strip_ansi_and_add_badges() {
    let entries = vec![LogEntry {
        stream: OutputStream::Stdout,
        text: "\x1b[32m[  OK   ] done\x1b[0m".to_string(),
    }];

    let lines = visual_output_lines(&entries, 80, None, None);
    let rendered = lines[0]
        .line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(rendered.contains("OK"));
    assert!(rendered.contains("done"));
    assert!(!rendered.contains("[  OK   ]"));
    assert!(!rendered.contains("\x1b"));
}

#[test]
fn event_feed_badges_are_centered_in_fixed_cells() {
    assert_eq!(event_badge_cell(LogKind::Success), "  OK  ");
    assert_eq!(event_badge_cell(LogKind::System), " SYS  ");
    assert_eq!(event_badge_cell(LogKind::Save), " SAVE ");
}

#[test]
fn display_padding_uses_cjk_width() {
    assert_eq!(pad_display_width("abc", 5), "abc  ");
    assert_eq!(pad_display_width("○○", 5), "○○ ");
}

#[test]
fn latest_event_feed_line_animates_when_running() {
    let entries = vec![LogEntry {
        stream: OutputStream::Stdout,
        text: "[  OK   ] done".to_string(),
    }];

    let first = visual_output_lines_with_motion(&entries, 80, None, None, true, 0);
    let second = visual_output_lines_with_motion(&entries, 80, None, None, true, 1);
    let first_text = first[0]
        .line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    let second_text = second[0]
        .line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(first_text.starts_with("▁ "));
    assert!(second_text.starts_with("▃ "));
    assert_ne!(first_text, second_text);
}

#[test]
fn latest_wrapped_event_line_keeps_live_highlight_on_continuation() {
    let entries = vec![LogEntry {
        stream: OutputStream::System,
        text: "pmoke --config config.toml fetch Fetch oscilloscope data using the configured output format.".to_string(),
    }];

    let lines = visual_output_lines_with_motion(&entries, 36, None, None, true, 0);

    assert!(lines.len() >= 2);
    assert_eq!(
        lines[1].line.spans[0].style.fg,
        Some(event_feed_pulse_color(0))
    );
    assert_eq!(
        lines[1].line.spans[1].style.fg,
        Some(event_feed_pulse_color(0))
    );
}

#[test]
fn latest_wrapped_metric_line_keeps_live_highlight_on_continuation() {
    let entries = vec![LogEntry {
        stream: OutputStream::Stdout,
        text: "│ output     Fetch oscilloscope data using the configured output format."
            .to_string(),
    }];

    let lines = visual_output_lines_with_motion(&entries, 34, None, None, true, 0);

    assert!(lines.len() >= 2);
    assert_eq!(
        lines[1].line.spans[0].style.fg,
        Some(event_feed_pulse_color(0))
    );
    assert_eq!(
        lines[1].line.spans[3].style.fg,
        Some(event_feed_pulse_color(0))
    );
}

#[test]
fn display_output_text_removes_cli_badges_from_status_lines() {
    assert_eq!(
        display_output_text(LogKind::Success, "[  OK  ] lock-in plot completed").as_deref(),
        Some("lock-in plot completed")
    );
    assert_eq!(
        display_output_text(LogKind::Save, "[ SAVE ] lock-in results for signals [4]").as_deref(),
        Some("lock-in results for signals [4]")
    );
    assert_eq!(
        display_output_text(
            LogKind::Warning,
            "[ WARN ] legacy config v2: [timebase] is deprecated"
        )
        .as_deref(),
        Some("legacy config v2: [timebase] is deprecated")
    );
}

#[test]
fn display_output_text_reframes_cli_tables_for_tui() {
    assert_eq!(
        display_output_text(LogKind::Section, "╭─────────┬────────╮"),
        None
    );
    assert_eq!(
        display_output_text(LogKind::Metric, "│ Setting ┆ Value  │"),
        None
    );
    assert_eq!(
        display_output_text(LogKind::Metric, "│ cutoff ┆ 2.3e4 Hz │").as_deref(),
        Some("cutoff  →  2.3e4 Hz")
    );
    assert_eq!(
        display_output_text(
            LogKind::Metric,
            "│ Channel ┆ Role ┆ Label ┆ Unit ┆ Factor │"
        ),
        None
    );
    assert_eq!(
        display_output_text(LogKind::Metric, "│ ch3 ┆ reference ┆ - ┆ - ┆ - │").as_deref(),
        Some("ch3  →  reference  /  -  /  -  /  -")
    );
}

#[test]
fn display_output_text_reframes_compact_panels_for_tui() {
    assert_eq!(
        display_output_text(LogKind::Section, "╭─ Lock-in settings").as_deref(),
        Some("Lock-in settings")
    );
    assert_eq!(
        classify_log_entry(OutputStream::Stdout, "│ cutoff     2.3e4 Hz"),
        LogKind::Metric
    );
    assert_eq!(
        display_output_text(LogKind::Metric, "│ cutoff     2.3e4 Hz").as_deref(),
        Some("cutoff  →  2.3e4 Hz")
    );
    assert_eq!(
        display_output_text(LogKind::Metric, "│            stride_samples=100").as_deref(),
        Some("stride_samples=100")
    );
    assert_eq!(display_output_text(LogKind::Section, "╰─"), None);
}

#[test]
fn compact_panel_continuation_renders_without_raw_pipe() {
    let entries = vec![LogEntry {
        stream: OutputStream::Stdout,
        text: "│            stride_samples=100".to_string(),
    }];

    let lines = visual_output_lines(&entries, 80, None, None);
    let rendered = lines[0]
        .line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(rendered.contains("↳ stride_samples=100"));
    assert!(!rendered.contains("│            stride_samples"));
}

#[test]
fn visual_output_line_count_does_not_overcount_exact_width() {
    let entries = vec![LogEntry {
        stream: OutputStream::Stdout,
        text: "abcdefghijklm".to_string(),
    }];

    assert_eq!(visual_output_line_count(&entries, 26), 1);
    assert_eq!(visual_output_line_count(&entries, 25), 2);
}

#[test]
fn visual_output_line_count_uses_cjk_display_width() {
    let entries = vec![LogEntry {
        stream: OutputStream::Stdout,
        text: "○○○○○○○".to_string(),
    }];

    assert_eq!(visual_output_line_count(&entries, 27), 1);
    assert_eq!(visual_output_line_count(&entries, 26), 2);
    assert_eq!(
        visual_output_line_count(&entries, 26),
        visual_output_lines(&entries, 26, None, None).len()
    );
}

#[test]
fn selected_output_status_uses_wrapped_visual_line_range() {
    let entries = vec![
        LogEntry {
            stream: OutputStream::Stdout,
            text: "abcdefghijklmnopqrstuvwxyz".to_string(),
        },
        LogEntry {
            stream: OutputStream::Stdout,
            text: "tail".to_string(),
        },
    ];

    let lines = visual_output_lines(&entries, 26, None, None);

    assert_eq!(visual_entry_range(&lines, 0), Some((0, 1)));
    assert_eq!(visual_selection_range(&lines, Some((0, 0))), Some((0, 1)));
    assert_eq!(
        output_selection_status(visual_selection_range(&lines, Some((0, 0)))),
        "selected 1-2 / 2 lines"
    );
    assert_eq!(
        output_selection_status(visual_selection_range(&lines, Some((1, 1)))),
        "selected 3"
    );
}

#[test]
fn output_selection_skips_entries_that_are_not_rendered() {
    let mut app = test_app();
    app.push_output(
        OutputStream::Stdout,
        "╭────────\nvisible one\n╰─\nvisible two\n╰─",
    );

    app.focus_output();
    assert_eq!(app.output_selected, Some(3));

    app.select_previous_output(false);
    assert_eq!(app.output_selected, Some(1));

    app.select_next_output(false);
    assert_eq!(app.output_selected, Some(3));

    app.select_first_output(false);
    assert_eq!(app.output_selected, Some(1));

    app.select_last_output(false);
    assert_eq!(app.output_selected, Some(3));
}

#[test]
fn selected_output_text_skips_entries_that_are_not_rendered() {
    let mut app = test_app();
    app.push_output(
        OutputStream::Stdout,
        "╭────────\nvisible one\n╰─\nvisible two\n╰─",
    );
    app.output_selected = Some(3);
    app.output_selection_anchor = Some(1);

    assert_eq!(
        app.selected_output_text().as_deref(),
        Some("visible one\nvisible two")
    );
    assert_eq!(app.output_selection_line_count(), 2);
}

#[test]
fn output_scroll_clamps_when_all_lines_are_visible() {
    let mut app = test_app();
    app.push_output(OutputStream::Stdout, "one\ntwo\nthree");
    app.focus_output();
    app.run_output_scroll = 99;

    clamp_output_scroll(&mut app, Rect::new(0, 0, 120, 28));

    assert_eq!(app.run_output_scroll, 0);
}

#[test]
fn output_scroll_clamps_to_renderable_max() {
    let mut app = test_app();
    app.push_output(
        OutputStream::Stdout,
        &(0..80)
            .map(|idx| format!("line {idx:02}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    app.focus_output();
    app.run_output_scroll = 999;
    let area = Rect::new(0, 0, 120, 28);
    let max_scroll = max_output_scroll_for_area(&app, area).expect("output area exists");

    clamp_output_scroll(&mut app, area);

    assert_eq!(app.run_output_scroll, max_scroll);
}

#[test]
fn output_scrollbar_reaches_bottom_at_latest() {
    let visual_line_count = 80;
    let visible_rows = 10;
    let track_height = 10;

    let (thumb_start, thumb_len) =
        output_scrollbar_thumb(visual_line_count, visible_rows, 0, track_height).unwrap();

    assert_eq!(thumb_start + thumb_len, track_height as usize);
}

#[test]
fn output_scrollbar_reaches_top_at_oldest() {
    let visual_line_count = 80;
    let visible_rows = 10;
    let track_height = 10;
    let max_scroll = visual_line_count - visible_rows;

    let (thumb_start, _thumb_len) =
        output_scrollbar_thumb(visual_line_count, visible_rows, max_scroll, track_height).unwrap();

    assert_eq!(thumb_start, 0);
}

#[test]
fn header_omits_pipeline_counter() {
    let app = test_app();
    let rendered = header_spans(&app, 120)
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(!rendered.contains("PIPE"));
}

#[test]
fn context_bar_shows_current_directory_on_every_tab() {
    let mut app = test_app();
    app.current_dir = "/workspace/pmoke".to_string();

    for tab in 0..TAB_TITLES.len() {
        app.active_tab = tab;
        let rendered = context_bar_spans(&app, 120)
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(rendered.contains("◆ cwd /workspace/pmoke"), "tab {tab}");
        assert!(rendered.contains("config config.toml"), "tab {tab}");
    }
}

#[test]
fn narrow_context_bar_prioritizes_current_directory_and_fits_width() {
    let mut app = test_app();
    app.current_dir = "/very/long/workspace/path/to/pmoke".to_string();
    let width = 24;

    let spans = context_bar_spans(&app, width);
    let rendered = spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(rendered.contains("cwd"));
    assert!(rendered.ends_with("pmoke"));
    assert!(rendered.cell_width() <= width);
    assert!(!rendered.contains("config"));

    app.current_dir = "/workspace/ﾊﾟﾋﾟﾌﾟﾍﾟﾎﾟ/pmoke".to_string();
    for checked_width in 0..160 {
        let rendered_width = context_bar_spans(&app, checked_width)
            .iter()
            .map(|span| span.content.cell_width())
            .sum::<u16>();
        assert_eq!(rendered_width, checked_width, "width {checked_width}");
    }
}

#[test]
fn context_bar_only_adds_config_when_cwd_keeps_useful_space() {
    let mut app = test_app();
    app.current_dir = "/very/long/workspace/path/to/pmoke".to_string();

    let narrow = context_bar_spans(&app, (CONTEXT_DETAILS_MIN_WIDTH - 1) as u16)
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    let wide = context_bar_spans(&app, CONTEXT_DETAILS_MIN_WIDTH as u16)
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(!narrow.contains("config"));
    assert!(narrow.contains("path/to/pmoke"));
    assert!(wide.contains("config"));
    assert!(wide.contains("path/to/pmoke"));
}

#[test]
fn output_header_omits_badge_legend_when_narrow() {
    let narrow_text = output_header_spans(24)
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    let wide_text = output_header_spans(80)
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(!narrow_text.contains("error"));
    assert!(wide_text.contains("analysis output"));
    assert!(wide_text.contains("error"));
}

#[test]
fn event_feed_header_animates_when_running() {
    let first = output_header_spans_with_motion(80, true, 0)
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    let second = output_header_spans_with_motion(80, true, 1)
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(first.contains("▁ live"));
    assert!(second.contains("▃ live"));
    assert_ne!(first, second);
}

#[test]
fn selected_output_text_copies_selected_range() {
    let mut app = test_app();
    app.push_output(OutputStream::Stdout, "one\ntwo\n\x1b[31mthree\x1b[0m");
    app.output_selected = Some(2);
    app.output_selection_anchor = Some(0);

    assert_eq!(
        app.selected_output_text().as_deref(),
        Some("one\ntwo\nthree")
    );
    assert_eq!(app.output_selection_line_count(), 3);
}

#[test]
fn output_navigation_collapses_or_extends_range() {
    let mut app = test_app();
    app.push_output(OutputStream::Stdout, "one\ntwo\nthree");
    app.output_selected = Some(1);
    app.enter_output_line_visual_mode();

    app.select_next_output(true);
    assert_eq!(app.output_selection_range(), Some((1, 2)));

    app.select_previous_output(false);
    assert_eq!(app.output_selected, Some(1));
    assert_eq!(app.output_selection_range(), Some((1, 1)));
}

#[test]
fn visual_line_yank_clears_selection_like_neovim() {
    let mut app = test_app();
    app.push_output(OutputStream::Stdout, "one\ntwo\nthree");
    app.output_selected = Some(1);

    app.enter_output_line_visual_mode();
    assert_eq!(app.output_selection_anchor, Some(1));

    app.select_next_output(true);
    app.finish_output_yank(13, ClipboardMethod::Osc52);

    assert_eq!(app.output_selected, Some(2));
    assert_eq!(app.output_selection_anchor, None);
    assert_eq!(
        app.copy_status.as_deref(),
        Some("copied 2 lines / 13 chars via terminal")
    );
}

#[test]
fn base64_encoder_matches_standard_vectors() {
    assert_eq!(base64_encode(b""), "");
    assert_eq!(base64_encode(b"f"), "Zg==");
    assert_eq!(base64_encode(b"fo"), "Zm8=");
    assert_eq!(base64_encode(b"foo"), "Zm9v");
    assert_eq!(base64_encode("日本語".as_bytes()), "5pel5pys6Kqe");
}

#[test]
fn vim_output_selection_auto_scrolls_to_selected_line() {
    let mut app = test_app();
    app.push_output(
        OutputStream::Stdout,
        &(0..80)
            .map(|idx| format!("line {idx:02}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    app.focus_output();
    let area = Rect::new(0, 0, 120, 28);

    app.output_selected = Some(0);
    app.run_output_scroll = 0;
    ensure_selected_output_visible(&mut app, area);
    assert!(app.run_output_scroll > 0);

    app.output_selected = Some(app.run_output.len() - 1);
    app.run_output_scroll = 80;
    ensure_selected_output_visible(&mut app, area);
    assert_eq!(app.run_output_scroll, 0);
}

#[test]
fn cancel_command_marks_run_and_sends_stop_signal() {
    let mut app = test_app();
    let (_event_tx, event_rx) = mpsc::channel();
    let (cancel_tx, cancel_rx) = mpsc::channel();
    app.active_run = Some(ActiveRun {
        action: MonitorAction::Analyze,
        label: "Analyze all",
        started_at: Instant::now(),
        receiver: event_rx,
        cancel: cancel_tx,
        cancel_requested: false,
    });

    app.cancel_command(CancelReason::CtrlC);

    assert!(app.active_run.as_ref().unwrap().cancel_requested);
    assert_eq!(cancel_rx.try_recv().unwrap(), CancelReason::CtrlC);
    assert!(
        app.run_output
            .iter()
            .any(|entry| entry.text.contains("Stopping command via Ctrl+C"))
    );
}

#[test]
fn tui_tick_uses_60fps_while_command_is_running() {
    let mut app = test_app();
    let (_event_tx, event_rx) = mpsc::channel();
    let (cancel_tx, _cancel_rx) = mpsc::channel();
    app.active_run = Some(ActiveRun {
        action: MonitorAction::Analyze,
        label: "Analyze all",
        started_at: Instant::now(),
        receiver: event_rx,
        cancel: cancel_tx,
        cancel_requested: false,
    });

    assert_eq!(tui_frame_tick(&app), TUI_ANIMATION_TICK);
}

#[test]
fn escape_does_not_cancel_running_command() {
    let mut app = test_app();
    let (_event_tx, event_rx) = mpsc::channel();
    let (cancel_tx, cancel_rx) = mpsc::channel();
    app.active_run = Some(ActiveRun {
        action: MonitorAction::Analyze,
        label: "Analyze all",
        started_at: Instant::now(),
        receiver: event_rx,
        cancel: cancel_tx,
        cancel_requested: false,
    });

    app.escape_current_mode();

    assert!(!app.active_run.as_ref().unwrap().cancel_requested);
    assert!(cancel_rx.try_recv().is_err());
    assert!(
        app.run_output
            .iter()
            .all(|entry| !entry.text.contains("Stopping command"))
    );
}

#[test]
fn ctrl_c_interrupt_cancels_running_command() {
    let mut app = test_app();
    let (_event_tx, event_rx) = mpsc::channel();
    let (cancel_tx, cancel_rx) = mpsc::channel();
    app.active_run = Some(ActiveRun {
        action: MonitorAction::Analyze,
        label: "Analyze all",
        started_at: Instant::now(),
        receiver: event_rx,
        cancel: cancel_tx,
        cancel_requested: false,
    });
    app.show_help = true;

    app.interrupt_current_operation();

    assert!(!app.show_help);
    assert!(app.active_run.as_ref().unwrap().cancel_requested);
    assert_eq!(cancel_rx.try_recv().unwrap(), CancelReason::CtrlC);
    assert!(
        app.run_output
            .iter()
            .any(|entry| entry.text.contains("Stopping command via Ctrl+C"))
    );
}

#[test]
fn ctrl_c_interrupt_clears_visual_output_selection_when_idle() {
    let mut app = test_app();
    app.push_output(OutputStream::Stdout, "one\ntwo\nthree");
    app.output_selected = Some(2);
    app.output_selection_anchor = Some(0);
    app.focus = FocusPane::Output;

    app.interrupt_current_operation();

    assert_eq!(app.output_selected, Some(2));
    assert_eq!(app.output_selection_anchor, None);
    assert_eq!(app.copy_status.as_deref(), Some("selection cleared"));
    assert_eq!(app.focus, FocusPane::Output);
}

#[test]
fn escape_clears_output_mode_before_quit() {
    let mut app = test_app();
    app.push_output(OutputStream::Stdout, "one\ntwo\nthree");
    app.output_selected = Some(2);
    app.output_selection_anchor = Some(0);
    app.focus = FocusPane::Output;

    app.escape_current_mode();
    assert_eq!(app.output_selection_anchor, None);
    assert_eq!(app.focus, FocusPane::Output);

    app.escape_current_mode();
    assert_eq!(app.focus, FocusPane::Commands);
}

#[test]
fn output_g_and_g_jump_within_output_focus() {
    let mut app = test_app();
    app.push_output(OutputStream::Stdout, "one\ntwo\nthree");
    app.focus_output();

    app.select_first_output(false);
    assert_eq!(app.focus, FocusPane::Output);
    assert_eq!(app.output_selected, Some(0));

    app.select_last_output(false);
    assert_eq!(app.focus, FocusPane::Output);
    assert_eq!(app.output_selected, Some(2));
    assert_eq!(app.run_output_scroll, 0);
}

#[test]
fn direct_focus_commands_select_expected_panes() {
    let mut app = test_app();

    app.focus_output();
    assert_eq!(app.active_tab, 0);
    assert_eq!(app.focus, FocusPane::Output);

    app.focus_messages();
    assert_eq!(app.active_tab, 2);
    assert_eq!(app.focus, FocusPane::Messages);

    app.focus_files();
    assert_eq!(app.active_tab, 3);
    assert_eq!(app.focus, FocusPane::Files);

    app.focus_status();
    assert_eq!(app.active_tab, 0);
    assert_eq!(app.focus, FocusPane::Status);

    app.focus_actions();
    assert_eq!(app.active_tab, 0);
    assert_eq!(app.focus, FocusPane::Commands);
}

#[test]
fn clicking_actions_tab_moves_focus_from_output_to_commands() {
    let mut app = test_app();
    app.push_output(OutputStream::Stdout, "one");
    app.focus_output();
    let area = Rect::new(0, 0, 120, 28);
    let tabs = UiLayout::new(area, app.active_tab).tabs;

    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: tabs.x + 4,
            row: tabs.y,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();

    assert_eq!(app.active_tab, 0);
    assert_eq!(app.focus, FocusPane::Commands);
}

#[test]
fn mouse_tab_hit_testing_matches_rendered_tab_widths() {
    let area = Rect::new(3, 5, 120, 1);
    let cases = [
        (4, 0, FocusPane::Commands),
        (16, 1, FocusPane::Config),
        (27, 2, FocusPane::Messages),
        (40, 3, FocusPane::Files),
    ];

    for (column, expected_tab, expected_focus) in cases {
        let mut app = test_app();
        app.focus_output();

        select_tab_at(&mut app, area, column);

        assert_eq!(app.active_tab, expected_tab, "column {column}");
        assert_eq!(app.focus, expected_focus, "column {column}");
    }

    let mut app = test_app();
    app.focus_output();
    select_tab_at(&mut app, area, 80);
    assert_eq!(app.active_tab, 0);
    assert_eq!(app.focus, FocusPane::Output);
}

#[test]
fn keyboard_tab_navigation_updates_focus_with_active_tab() {
    let mut app = test_app();
    app.focus_output();

    select_previous_tab(&mut app);
    assert_eq!(app.active_tab, 0);
    assert_eq!(app.focus, FocusPane::Output);

    select_next_tab(&mut app);
    assert_eq!(app.active_tab, 1);
    assert_eq!(app.focus, FocusPane::Config);

    select_next_tab(&mut app);
    assert_eq!(app.active_tab, 2);
    assert_eq!(app.focus, FocusPane::Messages);

    select_previous_tab(&mut app);
    assert_eq!(app.active_tab, 1);
    assert_eq!(app.focus, FocusPane::Config);
}

#[test]
fn mouse_wheel_scrolls_visible_messages_without_moving_hidden_output() {
    let mut app = test_app();
    let ConfigLoad::Diagnostics(diagnostics) = &mut app.load else {
        panic!("test app should contain diagnostics");
    };
    diagnostics.diagnostics = (0..20)
        .map(|index| ConfigDiagnostic {
            kind: DiagnosticKind::Validation,
            path: Some(format!("item.{index}")),
            message: format!("diagnostic message {index}"),
            suggestion: None,
        })
        .collect();
    app.push_output(
        OutputStream::Stdout,
        &(0..30)
            .map(|index| format!("output {index}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    app.run_output_scroll = 10;
    app.focus_messages();
    let area = Rect::new(0, 0, 80, 14);
    let panel = UiLayout::new(area, app.active_tab).active_panel;

    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: panel.x + 1,
            row: panel.y + 1,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();

    assert!(app.messages_scroll > 0);
    assert_eq!(app.run_output_scroll, 10);
}

#[test]
fn message_wrapping_preserves_text_styles_and_display_width() {
    let warning_style = Style::default().fg(Color::Yellow);
    let lines = vec![Line::from(vec![
        Span::styled("WARN ", warning_style),
        Span::raw("日本語abcdef"),
    ])];

    let wrapped = wrap_styled_lines(lines, 8);

    assert!(wrapped.len() > 1);
    assert!(wrapped.iter().all(|line| line.width() <= 8));
    assert_eq!(
        wrapped
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<String>(),
        "WARN 日本語abcdef"
    );
    assert_eq!(wrapped[0].spans[0].style, warning_style);
}

#[test]
fn mouse_wheel_on_non_actions_tabs_never_scrolls_hidden_output() {
    let mut app = test_app();
    app.push_output(
        OutputStream::Stdout,
        &(0..30)
            .map(|index| format!("output {index}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    let area = Rect::new(0, 0, 80, 14);

    for tab in [1, 3] {
        activate_tab(&mut app, tab);
        app.run_output_scroll = 10;
        let panel = UiLayout::new(area, app.active_tab).active_panel;

        handle_mouse(
            &mut app,
            area,
            MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: panel.x + 1,
                row: panel.y + 1,
                modifiers: KeyModifiers::NONE,
            },
        )
        .unwrap();

        assert_eq!(app.run_output_scroll, 10, "tab {tab}");
    }
}

#[test]
fn mouse_wheel_scrolls_overflowing_config_and_files_tables() {
    let mut app = ready_test_app(20);
    let area = Rect::new(0, 0, 80, 14);

    activate_tab(&mut app, 1);
    let config_panel = UiLayout::new(area, app.active_tab).active_panel;
    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: config_panel.x + 1,
            row: config_panel.bottom() - 1,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();
    assert!(app.config_scroll > 0);
    assert!(app.config_scroll <= config_scroll_max(&app, config_panel));

    activate_tab(&mut app, 3);
    let files_panel = UiLayout::new(area, app.active_tab).active_panel;
    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: files_panel.x + 1,
            row: files_panel.y + 1,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();
    assert!(app.files_scroll > 0);
    assert!(app.files_scroll <= files_scroll_max(&app, files_panel));
}

#[test]
fn command_panel_border_click_focuses_commands_without_changing_selection() {
    let mut app = test_app();
    app.selected_action = 2;
    let area = Rect::new(0, 0, 120, 28);
    let commands = UiLayout::new(area, 0).command_palette;

    for (column, row) in [
        (commands.x + 2, commands.y),
        (commands.x, commands.y + 2),
        (commands.right() - 1, commands.y + 2),
    ] {
        app.focus_output();
        handle_mouse(
            &mut app,
            area,
            MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            },
        )
        .unwrap();
        assert_eq!(app.selected_action, 2);
        assert_eq!(app.focus, FocusPane::Commands);
    }
}

#[test]
fn command_panel_content_click_focuses_and_selects_the_clicked_action() {
    let mut app = test_app();
    app.selected_action = 2;
    app.focus_output();
    let area = Rect::new(0, 0, 120, 28);
    let commands = UiLayout::new(area, app.active_tab).command_palette;

    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: commands.x + 1,
            row: commands.y + 1,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();

    assert_eq!(app.focus, FocusPane::Commands);
    assert_eq!(app.selected_action, 0);
}

#[test]
fn clicking_status_panel_focuses_status() {
    let mut app = test_app();
    app.focus_output();
    let area = Rect::new(0, 0, 120, 28);
    let status = UiLayout::new(area, app.active_tab).run_status;

    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: status.x + 1,
            row: status.y + 1,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();

    assert_eq!(app.active_tab, 0);
    assert_eq!(app.focus, FocusPane::Status);
}

#[test]
fn output_scrollbar_click_focuses_without_selecting_a_line() {
    let mut app = test_app();
    app.push_output(
        OutputStream::Stdout,
        &(0..30)
            .map(|index| format!("output {index}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    app.output_selected = Some(0);
    app.focus_commands();
    let area = Rect::new(0, 0, 120, 28);
    let output = UiLayout::new(area, app.active_tab).run_output;
    let selectable = output_selectable_area(output).expect("output log should be selectable");

    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: selectable.right(),
            row: selectable.y + 2,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();

    assert_eq!(app.focus, FocusPane::Output);
    assert_eq!(app.output_selected, Some(0));
    assert!(!app.output_mouse_drag_active);
}

#[test]
fn output_drag_only_extends_a_drag_started_on_an_output_line() {
    let mut app = test_app();
    app.push_output(
        OutputStream::Stdout,
        &(0..30)
            .map(|index| format!("output {index}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    app.output_selected = Some(0);
    let area = Rect::new(0, 0, 120, 28);
    let layout = UiLayout::new(area, app.active_tab);
    let selectable =
        output_selectable_area(layout.run_output).expect("output log should be selectable");

    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: layout.run_status.x + 1,
            row: layout.run_status.y + 1,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();
    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: selectable.x,
            row: selectable.y + 1,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();

    assert_eq!(app.focus, FocusPane::Status);
    assert_eq!(app.output_selected, Some(0));
    assert_eq!(app.output_selection_anchor, None);

    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: selectable.x,
            row: selectable.y,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();
    let drag_start = app.output_selected;
    assert!(app.output_mouse_drag_active);

    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: selectable.x,
            row: selectable.y + 1,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();

    assert_ne!(app.output_selected, drag_start);
    assert_eq!(app.output_selection_anchor, drag_start);

    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: selectable.x,
            row: selectable.y + 1,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();
    assert!(!app.output_mouse_drag_active);

    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: selectable.x,
            row: selectable.y,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();
    let selected_before_tab_switch = app.output_selected;
    activate_tab(&mut app, 2);
    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: selectable.x,
            row: selectable.y + 1,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();
    assert_eq!(app.active_tab, 2);
    assert_eq!(app.focus, FocusPane::Messages);
    assert_eq!(app.output_selected, selected_before_tab_switch);
}
