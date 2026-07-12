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
    };
    let entries = LogEntry::from_event(&event);
    let rendered = visual_output_lines(&entries, 52, None, None);

    assert_eq!(visual_output_line_count(&entries, 52), rendered.len());
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
    assert!(rendered[1].starts_with("         ~  command"));
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

    assert!(rendered.contains('+'));
    assert!(rendered.contains("done"));
    assert!(!rendered.contains("[  OK   ]"));
    assert!(!rendered.contains("\x1b"));
}

#[test]
fn event_icons_have_stable_single_cell_width() {
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
        assert_eq!(kind.marker().width_cjk(), 1, "{:?}", kind);
    }
}

#[test]
fn display_padding_uses_cjk_width() {
    assert_eq!(pad_display_width("abc", 5), "abc  ");
    assert_eq!(pad_display_width("○○", 5), "○○ ");
}

#[test]
fn latest_event_feed_line_animates_when_running() {
    let entries = vec![LogEntry::new(OutputStream::Stdout, "[  OK   ] done")];

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

    assert!(first_text.starts_with("|  "));
    assert!(second_text.starts_with("/  "));
    assert_ne!(first_text, second_text);
}

#[test]
fn latest_wrapped_event_line_keeps_live_highlight_on_continuation() {
    let entries = vec![LogEntry::new(
        OutputStream::System,
        "pmoke --config config.toml fetch Fetch oscilloscope data using the configured output format.",
    )];

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
    let entries = vec![LogEntry::new(
        OutputStream::Stdout,
        "│ output     Fetch oscilloscope data using the configured output format.",
    )];

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

    assert!(rendered.contains("↳ stride_samples=100"));
    assert!(!rendered.contains("│            stride_samples"));
}

#[test]
fn visual_output_line_count_does_not_overcount_exact_width() {
    let entries = vec![LogEntry::new(OutputStream::Stdout, "abcdefghijklm")];

    assert_eq!(visual_output_line_count(&entries, 16), 1);
    assert_eq!(visual_output_line_count(&entries, 15), 2);
}

#[test]
fn visual_output_line_count_uses_cjk_display_width() {
    let entries = vec![LogEntry::new(OutputStream::Stdout, "○○○○○○○")];

    assert_eq!(visual_output_line_count(&entries, 17), 1);
    assert_eq!(visual_output_line_count(&entries, 16), 2);
    assert_eq!(
        visual_output_line_count(&entries, 16),
        visual_output_lines(&entries, 16, None, None).len()
    );
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
