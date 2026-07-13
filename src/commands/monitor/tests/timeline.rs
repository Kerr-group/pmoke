use super::*;

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
fn strips_csi_ansi_codes() {
    assert_eq!(strip_ansi_codes("\x1b[1;36mLock-in\x1b[0m"), "Lock-in");
}

#[test]
fn wide_dashboard_layout_keeps_activity_visible() {
    let area = Rect::new(0, 0, 120, 28);
    let layout = UiLayout::new(area);

    assert!(layout.activity.height >= 5);
}

#[test]
fn workflow_panel_width_fits_command_rows_without_fixed_padding() {
    let area = Rect::new(0, 0, 120, 28);
    let layout = UiLayout::new(area);

    assert!(layout.workflow.width < 36);
    assert_eq!(layout.workflow.width, workflow_panel_width(area.width));
    assert!(layout.activity.width >= 40);
}

#[test]
fn output_table_width_fits_inside_live_output_text_area() {
    let area = Rect::new(0, 0, 120, 28);
    let log_content = output_log_content_area(area).expect("output area exists");
    let table_width = output_table_width_for_area(area).expect("table width exists");

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
fn activity_uses_every_available_log_row_without_the_removed_inner_header() {
    assert_eq!(output_visible_rows(Rect::new(0, 0, 80, 8)), 8);
}

#[test]
fn output_layout_hides_timeline_when_too_short() {
    let sections = output_inner_layout(Rect::new(0, 0, 80, 6));

    assert_eq!(sections.status.height, 1);
    assert_eq!(sections.timeline.height, 0);
    assert!(sections.log.height > 0);
}

#[test]
fn timeline_separator_uses_full_available_width() {
    let line = timeline_separator(8);

    assert_eq!(line.spans[0].content.as_ref(), "────────");
}

#[test]
fn current_timeline_step_keeps_its_label_and_pulses_only_its_color() {
    let step = TimelineStep {
        label: "Lock-in",
        state: TimelineStepState::Current,
    };

    let first = timeline_step_spans(&step, 0);
    let second = timeline_step_spans(&step, 1);

    assert_eq!(first[0].content, second[0].content);
    assert_eq!(first[0].content.as_ref(), "Lock-in");
    assert_eq!(first[2].content.as_ref(), "running");
    assert_eq!(first[0].style.bg, None);
    assert_ne!(first[0].style.fg, second[0].style.fg);
}

#[test]
fn timeline_states_use_flat_semantic_labels_without_background_boxes() {
    for (state, expected) in [
        (TimelineStepState::Done, "Read"),
        (TimelineStepState::Current, "Read · running"),
        (TimelineStepState::Pending, "Read"),
        (TimelineStepState::Failed, "Read · failed"),
        (TimelineStepState::Stopping, "Read · stopping"),
    ] {
        let spans = timeline_step_spans(
            &TimelineStep {
                label: "Read",
                state,
            },
            0,
        );
        let rendered = spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert_eq!(rendered, expected);
        assert!(spans.iter().all(|span| span.style.bg.is_none()));
    }
}

#[test]
fn pending_timeline_step_is_a_static_muted_stage_without_repeated_next_text() {
    let step = TimelineStep {
        label: "Read",
        state: TimelineStepState::Pending,
    };

    let first = timeline_step_spans(&step, 0);
    let second = timeline_step_spans(&step, 1);

    assert_eq!(first[0].content, second[0].content);
    assert_eq!(first[0].content.as_ref(), "Read");
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].style.fg, Some(Color::DarkGray));
}

#[test]
fn compact_pending_timeline_step_is_static_and_clear() {
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

    assert_eq!(first_text, "...─...");
    assert_eq!(second_text, "...─...");
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
fn compact_current_timeline_step_uses_plain_status_text() {
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

    assert_eq!(rendered, "RUN─...");
    assert_eq!(lines[0].spans[0].content.as_ref(), "RUN");
    assert_eq!(lines[0].spans[2].content.as_ref(), "...");
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
    assert_eq!(rendered.matches("OK").count(), 2);
    assert_eq!(rendered.matches("...").count(), 3);
    assert!(rendered.contains("RUN"));
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
