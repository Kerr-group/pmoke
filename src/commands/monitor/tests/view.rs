use super::*;
use ratatui::backend::TestBackend;

fn render_dashboard_text(width: u16, height: u16, app: &mut MonitorApp) -> String {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|frame| render(frame, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn dashboard_render_keeps_core_regions_at_wide_and_compact_sizes() {
    for (width, height) in [(120, 30), (72, 24)] {
        let mut app = test_app();
        app.push_output(OutputStream::System, "dashboard render probe");
        let rendered = render_dashboard_text(width, height, &mut app);

        assert!(rendered.contains("WORKFLOW"), "{width}x{height}");
        assert!(rendered.contains("INSPECTOR"), "{width}x{height}");
        assert!(rendered.contains("ACTIVITY"), "{width}x{height}");
        assert!(
            rendered.contains("dashboard render probe"),
            "{width}x{height}"
        );
        assert!(!rendered.contains(" Config  Messages  Files "));
    }
}

#[test]
fn tiny_dashboard_hides_inspector_but_preserves_workflow_and_activity() {
    let mut app = test_app();
    app.push_output(OutputStream::System, "tiny output");

    let rendered = render_dashboard_text(40, 14, &mut app);

    assert!(rendered.contains("WORKFLOW"));
    assert!(rendered.contains("ACTIVITY"));
    assert!(rendered.contains("tiny output"));
    assert!(!rendered.contains("INSPECTOR"));
}

#[test]
fn header_omits_legacy_pipeline_counter() {
    let app = test_app();
    let rendered = header_spans(&app, 120)
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();

    assert!(!rendered.contains("PIPE"));
}

#[test]
fn context_bar_shows_current_directory_for_every_focus_pane() {
    let mut app = test_app();
    app.current_dir = "/workspace/pmoke".to_string();

    for focus in [FocusPane::Commands, FocusPane::Inspector, FocusPane::Output] {
        app.focus = focus;
        let rendered = context_bar_spans(&app, 120)
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();

        assert!(
            rendered.contains("◆ cwd /workspace/pmoke"),
            "focus {focus:?}"
        );
        assert!(rendered.contains("config config.toml"), "focus {focus:?}");
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

    app.current_dir = "/workspace/papipupepo/pmoke".to_string();
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
fn activity_title_distinguishes_live_and_paused_states() {
    let mut app = test_app();
    assert!(activity_title(&app, 0, 80).contains("ACTIVITY · LIVE · READY"));

    app.new_output_events = 12;
    assert!(activity_title(&app, 4, 80).contains("PAUSED · 12 NEW · G follow"));

    let narrow = activity_title(&app, 4, 24);
    assert!(narrow.contains("ACTIVITY"));
    assert!(narrow.width_cjk() <= 22);
}

#[test]
fn activity_header_uses_a_static_frame_when_motion_is_off() {
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
    app.motion_mode = MotionMode::Off;

    let title = activity_title(&app, 0, 80);
    assert!(title.contains("LIVE · Analyze all"));
    assert!(!title.contains("LIVE LOG"));
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
    assert_eq!(base64_encode("hello".as_bytes()), "aGVsbG8=");
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
