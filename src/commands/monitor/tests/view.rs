use super::*;

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
    assert!(wide_text.contains("warn"));
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
