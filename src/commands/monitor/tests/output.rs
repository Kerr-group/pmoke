use super::*;

#[test]
fn structured_events_keep_one_logical_count_and_render_elapsed_time() {
    let mut app = test_app();
    app.push_structured_output(UiEvent {
        event_type: "event".to_string(),
        sequence: 42,
        elapsed_ms: 1_234,
        level: EventLevel::Warning,
        kind: EventKind::Section,
        stage: Some("lockin".to_string()),
        message: "Lock-in settings".to_string(),
        fields: vec![
            ("output rate".to_string(), "500 kHz".to_string()),
            ("window".to_string(), "1.71 us".to_string()),
        ],
        progress_id: None,
        progress_current: None,
        progress_total: None,
        duration_ms: None,
    });

    assert_eq!(app.run_output.len(), 3);
    assert_eq!(app.visible_event_count(), 1);
    assert_eq!(app.visible_warning_count(), 1);
    let rendered = visual_output_lines(app.visible_output(), 80, None, None)
        .into_iter()
        .map(|line| {
            line.line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();
    assert!(rendered[0].starts_with("00:01.2  "));
    assert!(rendered[1].starts_with("         "));
    assert!(rendered[1].contains("├─ output rate"));
    assert!(rendered[2].contains("└─ window"));
}

#[test]
fn structured_event_line_count_matches_rendering_with_elapsed_prefix() {
    let event = UiEvent {
        event_type: "event".to_string(),
        sequence: 43,
        elapsed_ms: u64::MAX,
        level: EventLevel::Info,
        kind: EventKind::Status,
        stage: None,
        message: "a message that wraps once the elapsed prefix is reserved".to_string(),
        fields: Vec::new(),
        progress_id: None,
        progress_current: None,
        progress_total: None,
        duration_ms: None,
    };
    let entries = LogEntry::from_event(&event);
    let rendered = visual_output_lines(&entries, 52, None, None);

    assert_eq!(rendered.len(), 2, "reserve nine cells for elapsed time");
    let first = rendered[0]
        .line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert!(first.starts_with("99:59.9  "));
}

#[test]
fn system_events_align_with_the_elapsed_column() {
    let event = UiEvent {
        event_type: "event".to_string(),
        sequence: 44,
        elapsed_ms: 200,
        level: EventLevel::Success,
        kind: EventKind::Status,
        stage: None,
        message: "complete".to_string(),
        fields: Vec::new(),
        progress_id: None,
        progress_current: None,
        progress_total: None,
        duration_ms: None,
    };
    let mut entries = LogEntry::from_event(&event);
    entries.push(LogEntry::new(OutputStream::System, "command finished"));
    let rendered = visual_output_lines(&entries, 80, None, None)
        .into_iter()
        .map(|line| {
            line.line
                .spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>();

    assert!(rendered[0].starts_with("00:00.2  "));
    assert!(rendered[1].starts_with("         SYS   command"));
}

#[test]
fn paused_activity_counts_new_events_and_follow_clears_them() {
    let mut app = test_app();
    app.push_output(OutputStream::Stdout, "first");
    app.run_output_scroll = 1;
    app.push_output(OutputStream::Stdout, "second");
    assert_eq!(app.new_output_events, 1);
    app.follow_output();
    assert_eq!(app.new_output_events, 0);
    assert_eq!(app.run_output_scroll, 0);
}

#[test]
fn carriage_return_progress_replaces_the_previous_transient_line() {
    let mut app = test_app();
    app.push_progress(OutputStream::Stdout, "fetch 10%");
    app.push_progress(OutputStream::Stdout, "fetch 20%");
    assert_eq!(app.run_output.len(), 1);
    assert_eq!(app.run_output[0].text, "fetch 20%");
    app.push_output(OutputStream::Stdout, "fetch complete");
    app.push_progress(OutputStream::Stdout, "next 1%");
    assert_eq!(app.run_output.len(), 3);
}

#[test]
fn structured_progress_updates_in_place_and_completes_the_same_event() {
    let mut app = test_app();
    let progress = |sequence, current| UiEvent {
        event_type: "event".to_string(),
        sequence,
        elapsed_ms: current * 100,
        level: EventLevel::Info,
        kind: EventKind::Progress,
        stage: Some("lockin".to_string()),
        message: "Lock-in ch3".to_string(),
        fields: Vec::new(),
        progress_id: Some("lockin:ch3".to_string()),
        progress_current: Some(current),
        progress_total: Some(6),
        duration_ms: None,
    };
    app.push_structured_output(progress(1, 1));
    app.run_output_scroll = 1;
    app.push_structured_output(progress(2, 4));

    assert_eq!(app.run_output.len(), 1);
    assert_eq!(app.visible_event_count(), 1);
    assert_eq!(app.new_output_events, 0);
    assert!(app.run_output[0].text.contains("4/6 · 66%"));

    app.push_structured_output(UiEvent {
        event_type: "event".to_string(),
        sequence: 3,
        elapsed_ms: 700,
        level: EventLevel::Success,
        kind: EventKind::Status,
        stage: Some("lockin".to_string()),
        message: "Lock-in ch3 completed".to_string(),
        fields: Vec::new(),
        progress_id: Some("lockin:ch3".to_string()),
        progress_current: None,
        progress_total: None,
        duration_ms: Some(700),
    });

    assert_eq!(app.run_output.len(), 1);
    assert_eq!(app.run_output[0].kind, LogKind::Success);
    assert!(!app.run_output[0].transient);
    assert!(app.run_output[0].progress_id.is_none());
    assert!(app.run_output[0].text.contains("completed · 0.7s"));
}

#[test]
fn selecting_a_structured_field_selects_the_whole_logical_event() {
    let mut app = test_app();
    app.push_structured_output(UiEvent {
        event_type: "event".to_string(),
        sequence: 10,
        elapsed_ms: 100,
        level: EventLevel::Info,
        kind: EventKind::Section,
        stage: None,
        message: "Reference".to_string(),
        fields: vec![
            ("frequency".to_string(), "1 MHz".to_string()),
            ("phase".to_string(), "0.1 rad".to_string()),
        ],
        progress_id: None,
        progress_current: None,
        progress_total: None,
        duration_ms: None,
    });
    app.output_selected = Some(1);

    assert_eq!(app.output_selection_range(), Some((0, 2)));
    let copied = app.selected_output_text().unwrap();
    assert!(copied.contains("Reference"));
    assert!(copied.contains("frequency"));
    assert!(copied.contains("phase"));
}

#[test]
fn live_highlight_targets_the_latest_renderable_entry() {
    let entries = vec![
        LogEntry::new(OutputStream::Stdout, "[  OK  ] complete"),
        LogEntry::new(OutputStream::Stdout, "╰────────╯"),
    ];
    let lines = visual_output_lines_with_motion(&entries, 80, None, None, true, 0);
    assert_eq!(lines.len(), 1);
    assert_ne!(lines[0].line.spans[0].content.as_ref(), "✓ ");
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
        classify_log_entry(OutputStream::Stdout, "Error: raw metadata is missing"),
        LogKind::Error
    );
    assert_eq!(
        classify_log_entry(OutputStream::Stdout, "Traceback (most recent call last):"),
        LogKind::Error
    );
    assert_eq!(
        classify_log_entry(OutputStream::Stdout, "[ SKIP ] reference plot: no data"),
        LogKind::Skipped
    );
    assert_eq!(
        classify_log_entry(OutputStream::Stdout, "enbw_match_error=1.0e-3 Hz"),
        LogKind::Plain
    );
    assert_eq!(
        classify_log_entry(OutputStream::Stdout, "0 failed"),
        LogKind::Plain
    );
    assert_eq!(
        classify_log_entry(OutputStream::Stdout, "No warnings."),
        LogKind::Plain
    );
    assert_eq!(
        classify_log_entry(OutputStream::Stdout, "Warnings"),
        LogKind::Section
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
fn stderr_warning_continuation_keeps_warning_level() {
    let mut app = test_app();

    app.push_output(
        OutputStream::Stderr,
        "/tmp/plot.py:10: UserWarning: slow legend",
    );
    app.push_output(OutputStream::Stderr, "  plt.legend()");
    app.push_output(OutputStream::Stderr, "  [ ERROR ] explicit failure");
    app.push_output(OutputStream::Stderr, "Error: plotting failed");

    assert_eq!(
        app.run_output
            .iter()
            .map(|entry| entry.kind)
            .collect::<Vec<_>>(),
        vec![
            LogKind::Warning,
            LogKind::Warning,
            LogKind::Error,
            LogKind::Error
        ]
    );
}

#[test]
fn blank_stderr_line_ends_warning_continuation() {
    let mut app = test_app();

    app.push_output(
        OutputStream::Stderr,
        "UserWarning: warning body\n\n  independent stderr",
    );

    assert_eq!(app.run_output[0].kind, LogKind::Warning);
    assert_eq!(app.run_output[1].kind, LogKind::Error);
}

#[test]
fn stderr_info_continuation_keeps_info_level() {
    let mut app = test_app();

    app.push_output(OutputStream::Stderr, "(info) driver configuration");
    app.push_output(OutputStream::Stderr, "  /dev/gpib0 is ready");

    assert_eq!(app.run_output[0].kind, LogKind::Info);
    assert_eq!(app.run_output[1].kind, LogKind::Info);
}

#[test]
fn stream_reader_frames_records_across_short_reads() {
    struct ChunkedReader {
        bytes: Vec<u8>,
        position: usize,
        chunk_size: usize,
    }

    impl std::io::Read for ChunkedReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            if self.position == self.bytes.len() {
                return Ok(0);
            }
            let end = (self.position + self.chunk_size).min(self.bytes.len());
            let len = (end - self.position).min(buf.len());
            buf[..len].copy_from_slice(&self.bytes[self.position..self.position + len]);
            self.position += len;
            Ok(len)
        }
    }

    let reader = ChunkedReader {
        bytes: b"[ WARN ] one\r\nprogress 1\rprogress 2\nlast".to_vec(),
        position: 0,
        chunk_size: 3,
    };
    let (tx, rx) = std::sync::mpsc::sync_channel(8);
    let handle = spawn_stream_reader(reader, OutputStream::Stderr, tx);
    handle.join().unwrap();

    let records = rx
        .into_iter()
        .filter_map(|event| match event {
            RunEvent::Output(stream, text) => Some(("output", stream, text)),
            RunEvent::Progress(stream, text) => Some(("progress", stream, text)),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        records,
        vec![
            ("output", OutputStream::Stderr, "[ WARN ] one".to_string()),
            ("progress", OutputStream::Stderr, "progress 1".to_string()),
            ("output", OutputStream::Stderr, "progress 2".to_string()),
            ("output", OutputStream::Stderr, "last".to_string()),
        ]
    );
}

#[test]
fn stream_reader_decodes_structured_stdout_and_falls_back_for_plain_text() {
    let input = concat!(
        "{\"type\":\"event\",\"sequence\":9,\"elapsed_ms\":12,",
        "\"level\":\"success\",\"kind\":\"status\",",
        "\"message\":\"done\"}\nplain\n"
    );
    let (tx, rx) = std::sync::mpsc::sync_channel(4);
    spawn_stream_reader(
        std::io::Cursor::new(input.as_bytes().to_vec()),
        OutputStream::Stdout,
        tx,
    )
    .join()
    .unwrap();
    let events = rx.into_iter().collect::<Vec<_>>();
    assert!(matches!(
        &events[0],
        RunEvent::Structured(event)
            if event.sequence == 9 && event.message == "done"
    ));
    assert!(matches!(
        &events[1],
        RunEvent::Output(OutputStream::Stdout, text) if text == "plain"
    ));
}

#[test]
fn analyze_timeline_marks_done_current_and_pending_steps() {
    let output = vec![
        LogEntry::new(OutputStream::Stdout, "[ READ  ] fetched data: 4 channels"),
        LogEntry::new(OutputStream::Stdout, "[  OK   ] reference plot completed"),
    ];

    let timeline =
        timeline_for_action(MonitorAction::Analyze, &output, StageProgressState::Running)
            .expect("analyze has timeline stages");

    assert_eq!(timeline.done, 2);
    assert_eq!(timeline.total, 7);
    assert_eq!(timeline.steps[0].state, TimelineStepState::Done);
    assert_eq!(timeline.steps[1].state, TimelineStepState::Done);
    assert_eq!(timeline.steps[2].state, TimelineStepState::Current);
    assert_eq!(timeline.steps[3].state, TimelineStepState::Pending);
}

#[cfg(feature = "hw-core")]
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
    let entries = vec![LogEntry::new(
        OutputStream::Stdout,
        "\x1b[32m[  OK   ] done\x1b[0m",
    )];

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
fn event_labels_are_clear_and_fit_the_fixed_tag_column() {
    for kind in [
        LogKind::Plain,
        LogKind::System,
        LogKind::Success,
        LogKind::Info,
        LogKind::Read,
        LogKind::Save,
        LogKind::Fit,
        LogKind::Skipped,
        LogKind::Warning,
        LogKind::Error,
        LogKind::Section,
    ] {
        assert!(kind.label().width_cjk() <= 5, "{:?}", kind);
    }
}

#[test]
fn display_padding_uses_cjk_width() {
    assert_eq!(fit_padded_column("abc", 5), "abc  ");
    assert_eq!(fit_padded_column("○○", 5), "○○ ");
}

#[test]
fn latest_event_feed_line_animates_when_running() {
    let mut entry = LogEntry::new(OutputStream::Stdout, "working");
    entry.transient = true;
    let entries = vec![entry];

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

    assert!(first_text.starts_with("RUN   "));
    assert_eq!(first_text, second_text);
    assert_ne!(
        first[0].line.spans[0].style.fg,
        second[0].line.spans[0].style.fg
    );
}

#[test]
fn completed_latest_event_keeps_its_semantic_label_while_command_is_running() {
    let entries = vec![LogEntry::new(
        OutputStream::Stdout,
        "[ WARN ] check scaling",
    )];
    let lines = visual_output_lines_with_motion(&entries, 80, None, None, true, 0);
    let text = lines[0]
        .line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(text.starts_with("WARN  "));
}

#[test]
fn latest_wrapped_event_line_keeps_live_highlight_on_continuation() {
    let mut entry = LogEntry::new(
        OutputStream::System,
        "pmoke --config config.toml fetch Fetch oscilloscope data using the configured output format.",
    );
    entry.transient = true;
    let entries = vec![entry];

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
    let mut entry = LogEntry::new(
        OutputStream::Stdout,
        "│ output     Fetch oscilloscope data using the configured output format.",
    );
    entry.transient = true;
    let entries = vec![entry];

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
    assert_eq!(
        display_output_text(LogKind::Skipped, "[ SKIP ] reference plot: no data").as_deref(),
        Some("reference plot: no data")
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
    let entries = vec![LogEntry::new(
        OutputStream::Stdout,
        "│            stride_samples=100",
    )];

    let lines = visual_output_lines(&entries, 80, None, None);
    let rendered = lines[0]
        .line
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(rendered.contains("└─ stride_samples=100"));
    assert!(!rendered.contains("│            stride_samples"));
}

#[test]
fn visual_output_line_count_does_not_overcount_exact_width() {
    let entries = vec![LogEntry::new(OutputStream::Stdout, "abcdefghijklm")];

    assert_eq!(visual_output_lines(&entries, 19, None, None).len(), 1);
    assert_eq!(visual_output_lines(&entries, 18, None, None).len(), 2);
}

#[test]
fn visual_output_line_count_uses_cjk_display_width() {
    let entries = vec![LogEntry::new(OutputStream::Stdout, "○○○○○○○")];

    assert_eq!(visual_output_lines(&entries, 20, None, None).len(), 1);
    assert_eq!(visual_output_lines(&entries, 19, None, None).len(), 2);
}

#[test]
fn activity_renders_the_whole_keycap_and_the_last_character_at_the_wrap_edge() {
    use super::harness::{TINY, area_for, assert_continue, ch, drive, render_text};
    let area = area_for(TINY);
    let mut app = test_app();
    app.push_output(OutputStream::Stdout, "pmoke --config 12345*️⃣.toml show");
    assert_continue(&drive(&mut app, area, &[ch('o'), ch('g')]));
    let first = render_text(&mut app, TINY);
    assert!(first.contains("12345*️⃣"));
    assert_continue(&drive(&mut app, area, &[ch('G')]));
    let last = render_text(&mut app, TINY);
    assert!(
        last.contains("│ w "),
        "the final w must be reachable: {last}"
    );
    app.push_output(OutputStream::Stdout, &format!("{}*️⃣!", "x".repeat(30)));
    assert!(
        render_text(&mut app, TINY).contains("*️⃣"),
        "the cluster must wrap intact"
    );
}

#[test]
fn wrapped_events_and_fields_occupy_exactly_the_rows_used_for_scrolling() {
    let mut app = test_app();
    app.push_structured_output(UiEvent {
        event_type: "event".into(),
        sequence: 1,
        elapsed_ms: 1500,
        level: EventLevel::Info,
        kind: EventKind::Section,
        stage: None,
        message: "long section ".repeat(14),
        fields: vec![("long structured field name".into(), "abcdefghij".repeat(15))],
        progress_id: None,
        progress_current: None,
        progress_total: None,
        duration_ms: None,
    });
    app.push_output(
        OutputStream::Stdout,
        &"abcdefghijklmnopqrstuvwxyz".repeat(8),
    );
    use super::harness::{COMPACT, TINY, WIDE, area_for, assert_continue, ch, drive};
    for size in [TINY, COMPACT, WIDE] {
        let area = area_for(size);
        let text = output_log_content_area(area).unwrap();
        let width = text.width;
        let rows = visual_output_lines(app.visible_output(), width, None, None);
        assert_continue(&drive(&mut app, area, &[ch('o'), ch('g')]));
        assert_eq!(
            app.run_output_scroll,
            rows.len().saturating_sub(usize::from(text.height)),
            "{size:?}"
        );
        for row in &rows {
            assert!(
                row.line.width() <= width as usize,
                "{width} cols: {}",
                row.line
            );
        }
    }
}

#[test]
fn wide_activity_preserves_distinct_doctor_field_names() {
    use super::harness::{harness_app, render_text};
    let keys = [
        "config.reference.stride_samples",
        "config.reference.window_samples",
    ];
    for structured in [true, false] {
        let mut app = harness_app();
        for key in keys {
            push_test_column(&mut app, key, "100", structured);
        }
        let frame = render_text(&mut app, (90, 32));
        for key in keys {
            assert!(
                frame
                    .lines()
                    .any(|row| row.contains(key) && row.contains("100")),
                "structured={structured}, missing {key}: {frame}"
            );
        }
    }
}

#[test]
fn activity_keeps_the_value_after_a_halfwidth_voiced_sensor_label() {
    use super::harness::{area_for, harness_app, render_text};
    let size = (40, 30);
    assert_eq!(output_log_content_area(area_for(size)).unwrap().width, 37);
    for structured in [true, false] {
        let mut app = harness_app();
        push_test_column(&mut app, "ch1 ｶﾞ", "ABCDEFGHIJKLMNOPQRST", structured);
        let frame = render_text(&mut app, size);
        let text = frame.split("ACTIVITY ·").nth(1).unwrap();
        assert!(text.contains("ch1 ｶﾞ"));
        let capitals: String = text.chars().filter(char::is_ascii_uppercase).collect();
        assert!(
            capitals.contains("ABCDEFGHIJKLMNOPQRST"),
            "structured={structured}: {frame}"
        );
    }
}

fn push_test_column(app: &mut MonitorApp, key: &str, value: &str, structured: bool) {
    if structured {
        let mut event = crate::ui::parse_jsonl_event(r#"{"type":"event","sequence":1,"elapsed_ms":1,"level":"info","kind":"section","message":"config"}"#).unwrap();
        event.fields.push((key.into(), value.into()));
        app.push_structured_output(event);
    } else {
        app.push_output(OutputStream::Stdout, &format!("│ {key}  {value}"));
    }
}

#[test]
fn resizing_until_the_tail_fits_does_not_resume_a_paused_log() {
    use super::harness::{area_for, assert_continue, ch, drive, key, render_text};
    let small = (40, 17);
    let tall = (40, 20);
    let mut app = test_app();
    for i in 1..=10 {
        app.push_output(OutputStream::Stdout, &format!("event-{i:02}"));
    }
    assert_eq!(output_log_content_area(area_for(small)).unwrap().height, 3);
    assert_eq!(output_log_content_area(area_for(tall)).unwrap().height, 6);
    assert_continue(&drive(&mut app, area_for(small), &[ch('o'), ch('G')]));
    assert_continue(&drive(&mut app, area_for(small), &[ch('k'); 5]));
    let events = |frame: &str| -> Vec<String> {
        frame
            .lines()
            .filter_map(|line| line.split_once("event-").map(|(_, n)| n[..2].to_owned()))
            .collect()
    };
    let before = events(&render_text(&mut app, small));
    assert_eq!(before, ["05", "06", "07"]);
    let expanded = render_text(&mut app, tall);
    assert_eq!(events(&expanded), ["05", "06", "07", "08", "09", "10"]);
    assert_eq!(events(&render_text(&mut app, small)), before);
    assert!(expanded.contains("PAUSED"));
    render_text(&mut app, tall);
    app.push_output(OutputStream::Stdout, "event-11");
    let narrow = render_text(&mut app, small);
    assert!(narrow.contains("PAUSED · 1 NEW"));
    assert_eq!(events(&narrow), before);
    assert_continue(&drive(&mut app, area_for(small), &[key(KeyCode::End)]));
    assert_eq!(events(&render_text(&mut app, small)), ["09", "10", "11"]);
}

#[test]
fn paused_text_stays_visible_when_the_first_timestamp_changes_wrapping() {
    use super::harness::{area_for, assert_continue, ch, drive, key, render_text};
    let size = (55, 14);
    let area = area_for(size);
    let mut app = test_app();
    app.push_output(
        OutputStream::Stdout,
        &format!("{}HERE{}", "x".repeat(178), "y".repeat(200)),
    );
    assert_continue(&drive(&mut app, area, &[ch('o'), ch('G')]));
    let tail = render_text(&mut app, size);
    let row = tail.lines().position(|line| line.contains("yyyy")).unwrap();
    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 15,
            row: row as u16,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();
    assert!(render_text(&mut app, size).contains("HERE"));
    let event = crate::ui::parse_jsonl_event(r#"{"type":"event","sequence":1,"elapsed_ms":1200,"level":"info","kind":"status","message":"timed-tail"}"#).unwrap();
    app.push_structured_output(event);
    let after = render_text(&mut app, size);
    assert!(
        after.contains("HERE"),
        "lost the visible text when elapsed appeared: {after}"
    );
    assert!(after.contains("PAUSED"));
    for width in [50, 61, 55, 52, 55] {
        render_text(&mut app, (width, 14));
    }
    assert!(
        render_text(&mut app, size).contains("HERE"),
        "redraws must not move the anchor"
    );
    assert_continue(&drive(&mut app, area, &[key(KeyCode::End)]));
    let tail = render_text(&mut app, size);
    assert!(tail.contains("00:01.2") && tail.contains("timed-tail"));
}

#[test]
fn paused_activity_keeps_visible_rows_when_long_events_arrive_or_change() {
    use super::harness::{COMPACT, area_for, assert_continue, ch, drive, key, render_text};
    let mut app = test_app();
    app.push_output(
        OutputStream::Stdout,
        &(0..1000)
            .map(|i| format!("original-{i:03}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    assert_continue(&drive(
        &mut app,
        area_for(COMPACT),
        &[ch('o'), key(KeyCode::PageUp)],
    ));
    let visible = |app: &mut MonitorApp| {
        render_text(app, COMPACT)
            .lines()
            .filter_map(|s| s.split_once("original-").map(|(_, n)| n[..3].to_owned()))
            .collect::<Vec<_>>()
    };
    let before = visible(&mut app);
    assert!(!before.is_empty());
    app.push_output(OutputStream::Stdout, &"long event ".repeat(90));
    assert_eq!(
        visible(&mut app),
        before,
        "appending wrapped rows and trimming the oldest event"
    );
    app.push_progress(OutputStream::Stderr, "short");
    app.push_progress(OutputStream::Stderr, &"long progress ".repeat(100));
    assert_eq!(
        visible(&mut app),
        before,
        "a growing progress record below the viewport"
    );
    assert_continue(&drive(&mut app, area_for(COMPACT), &[key(KeyCode::End)]));
    app.push_output(OutputStream::Stdout, "latest-after-follow");
    assert!(render_text(&mut app, COMPACT).contains("latest-after-follow"));
}

#[test]
fn moving_down_at_a_wrapped_tail_keeps_following_output() {
    use super::harness::{TINY, area_for, assert_continue, ch, drive, key, render_text};
    let mut app = test_app();
    app.push_output(
        OutputStream::Stdout,
        &format!("{} tail-marker", "x".repeat(40)),
    );
    let area = area_for(TINY);
    assert_continue(&drive(&mut app, area, &[ch('o'), ch('G')]));
    assert!(render_text(&mut app, TINY).contains("tail-marker"));
    for event in [ch('j'), key(KeyCode::Down), ch('J')] {
        assert_continue(&drive(&mut app, area, &[event]));
        assert!(render_text(&mut app, TINY).contains("tail-marker"));
        assert_eq!(app.run_output_scroll, 0);
    }
    app.push_output(OutputStream::Stdout, "new-tail");
    assert!(render_text(&mut app, TINY).contains("new-tail"));
}

#[test]
fn first_and_last_output_keys_reach_the_ends_of_multirow_events() {
    use super::harness::{COMPACT, area_for, assert_continue, ch, drive, render_text};
    let mut app = test_app();
    app.push_output(
        OutputStream::Stdout,
        &format!(
            "start-marker {}\n{} end-marker",
            "a".repeat(700),
            "b".repeat(700)
        ),
    );
    let area = area_for(COMPACT);
    for (key, marker) in [
        ('g', "start-marker"),
        ('g', "start-marker"),
        ('G', "end-marker"),
    ] {
        assert_continue(&drive(&mut app, area, &[ch('o'), ch(key)]));
        let frame = render_text(&mut app, COMPACT);
        assert!(frame.contains(marker), "{key}: {frame}");
    }
    assert_eq!(app.run_output_scroll, 0);
}

#[test]
fn end_displays_the_tail_after_a_wrapped_activity_event() {
    use super::harness::{COMPACT, area_for, assert_continue, ch, drive, key, render_text};
    let mut app = test_app();
    app.push_output(
        OutputStream::Stdout,
        &format!("{}\ntail-event", "a".repeat(700)),
    );
    let area = area_for(COMPACT);
    assert_continue(&drive(
        &mut app,
        area,
        &[ch('o'), key(KeyCode::PageUp), key(KeyCode::End)],
    ));
    let frame = render_text(&mut app, COMPACT);
    assert!(frame.contains("tail-event"), "{frame}");
}

#[test]
fn selected_output_status_uses_wrapped_visual_line_range() {
    let entries = vec![
        LogEntry::new(OutputStream::Stdout, "abcdefghijklmnopqrstuvwxyz"),
        LogEntry::new(OutputStream::Stdout, "tail"),
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
    let rows = output_log_content_area(area)
        .expect("output area exists")
        .height;
    let max_scroll = 80 - usize::from(rows);

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
fn retimed_stage_completions_render_without_transient_state() {
    let mut app = test_app();
    // Signal completion: Success/Save with compute+save attribution already
    // in the message (no duration_ms field on plain saved events).
    app.push_structured_output(UiEvent {
        event_type: "event".to_string(),
        sequence: 50,
        elapsed_ms: 1_500,
        level: EventLevel::Success,
        kind: EventKind::Save,
        stage: None,
        message: "signal means for channels [4] (compute 1.20s, save 3.00ms)".to_string(),
        fields: Vec::new(),
        progress_id: None,
        progress_current: None,
        progress_total: None,
        duration_ms: None,
    });
    assert_eq!(app.run_output.len(), 1);
    assert_eq!(app.run_output[0].kind, LogKind::Save);
    assert!(!app.run_output[0].transient);
    assert!(
        app.run_output[0]
            .text
            .contains("signal means for channels [4] (compute 1.20s, save 3.00ms)")
    );

    // Joint GLS completion under the boxcar marker: Success/Status with a
    // progress duration replaces the transient progress line in place.
    let progress = |sequence: u64, current: u64| UiEvent {
        event_type: "event".to_string(),
        sequence,
        elapsed_ms: current * 100,
        level: EventLevel::Info,
        kind: EventKind::Progress,
        stage: None,
        message: "joint GLS lock-in ch3".to_string(),
        fields: Vec::new(),
        progress_id: Some("progress:9".to_string()),
        progress_current: Some(current),
        progress_total: Some(1),
        duration_ms: None,
    };
    app.push_structured_output(progress(51, 0));
    app.push_structured_output(progress(52, 1));
    assert_eq!(app.run_output.len(), 2);
    assert!(app.run_output[1].transient);
    app.push_structured_output(UiEvent {
        event_type: "event".to_string(),
        sequence: 53,
        elapsed_ms: 2_500,
        level: EventLevel::Success,
        kind: EventKind::Status,
        stage: None,
        message: "lock-in processing completed (joint GLS, 2 workers, 2.30s)".to_string(),
        fields: Vec::new(),
        progress_id: Some("progress:9".to_string()),
        progress_current: None,
        progress_total: None,
        duration_ms: Some(2_300),
    });

    assert_eq!(app.run_output.len(), 2);
    assert_eq!(app.run_output[1].kind, LogKind::Success);
    assert!(!app.run_output[1].transient);
    assert!(app.run_output[1].progress_id.is_none());
    assert!(
        app.run_output[1]
            .text
            .contains("lock-in processing completed (joint GLS, 2 workers, 2.30s) · 2.3s")
    );
}
