use super::*;

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
fn tui_tick_uses_sixty_hz_while_command_is_running() {
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
    assert_eq!(TUI_ANIMATION_TICK, Duration::from_nanos(16_666_667));
}

#[test]
fn reduced_and_disabled_motion_lower_the_running_tick_rate() {
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

    app.motion_mode = MotionMode::Reduced;
    assert_eq!(tui_frame_tick(&app), TUI_REDUCED_MOTION_TICK);
    app.motion_mode = MotionMode::Off;
    assert_eq!(tui_frame_tick(&app), TUI_IDLE_TICK);
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
    assert_eq!(app.focus, FocusPane::Output);

    app.focus_messages();
    assert_eq!(app.inspector_view, InspectorView::Diagnostics);
    assert_eq!(app.focus, FocusPane::Inspector);

    app.focus_files();
    assert_eq!(app.inspector_view, InspectorView::Artifacts);
    assert_eq!(app.focus, FocusPane::Inspector);

    app.focus_status();
    assert_eq!(app.focus, FocusPane::Inspector);

    app.focus_actions();
    assert_eq!(app.focus, FocusPane::Commands);
}

#[test]
fn clicking_workflow_moves_focus_from_output_to_commands() {
    let mut app = test_app();
    app.push_output(OutputStream::Stdout, "one");
    app.focus_output();
    let area = Rect::new(0, 0, 120, 28);
    let workflow = dashboard_layout(area).workflow;

    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: workflow.x + 1,
            row: workflow.y + 1,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();

    assert_eq!(app.focus, FocusPane::Commands);
}

#[test]
fn clicking_inspector_only_changes_focus() {
    let mut app = test_app();
    app.focus_output();
    let area = Rect::new(0, 0, 120, 28);
    let inspector = dashboard_layout(area).inspector;

    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: inspector.x + 1,
            row: inspector.y + 1,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();

    assert_eq!(app.focus, FocusPane::Inspector);
    assert_eq!(app.inspector_view, InspectorView::Summary);
}

#[test]
fn keyboard_navigation_cycles_visible_dashboard_panes() {
    let mut app = test_app();
    assert_eq!(app.focus, FocusPane::Commands);

    focus_next_pane(&mut app);
    assert_eq!(app.focus, FocusPane::Inspector);

    focus_next_pane(&mut app);
    assert_eq!(app.focus, FocusPane::Output);

    focus_next_pane(&mut app);
    assert_eq!(app.focus, FocusPane::Commands);

    focus_previous_pane(&mut app);
    assert_eq!(app.focus, FocusPane::Output);
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
    let area = Rect::new(0, 0, 120, 28);
    let panel = dashboard_layout(area).inspector;

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
fn mouse_wheel_scrolls_the_visible_inspector_without_moving_output() {
    let mut app = ready_test_app(20);
    let area = Rect::new(0, 0, 120, 28);
    let inspector = dashboard_layout(area).inspector;
    app.inspector_view = InspectorView::Config;
    app.run_output_scroll = 7;
    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: inspector.x + 1,
            row: inspector.bottom() - 1,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();
    assert!(app.config_scroll > 0);
    assert!(app.config_scroll <= config_scroll_max(&app, inspector));
    assert_eq!(app.run_output_scroll, 7);
}

#[test]
fn command_panel_border_click_focuses_commands_without_changing_selection() {
    let mut app = test_app();
    app.workflow_cursor = 2;
    let area = Rect::new(0, 0, 120, 28);
    let commands = dashboard_layout(area).workflow;

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
        assert_eq!(app.workflow_cursor, 2);
        assert_eq!(app.focus, FocusPane::Commands);
    }
}

#[test]
fn command_panel_content_click_focuses_and_selects_the_clicked_action() {
    let mut app = test_app();
    app.workflow_cursor = 2;
    app.focus_output();
    let area = Rect::new(0, 0, 120, 28);
    let commands = dashboard_layout(area).workflow;

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
    assert_eq!(app.workflow_cursor, 0);
}

#[test]
fn clicking_inspector_focuses_inspector() {
    let mut app = test_app();
    app.focus_output();
    let area = Rect::new(0, 0, 120, 28);
    let inspector = dashboard_layout(area).inspector;

    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: inspector.x + 1,
            row: inspector.y + 1,
            modifiers: KeyModifiers::NONE,
        },
    )
    .unwrap();

    assert_eq!(app.focus, FocusPane::Inspector);
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
    let output = dashboard_layout(area).activity;
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
    let layout = dashboard_layout(area);
    let selectable =
        output_selectable_area(layout.activity).expect("output log should be selectable");

    handle_mouse(
        &mut app,
        area,
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: layout.inspector.x + 1,
            row: layout.inspector.y + 1,
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

    assert_eq!(app.focus, FocusPane::Inspector);
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
    let selected_before_focus_switch = app.output_selected;
    app.focus_messages();
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
    assert_eq!(app.focus, FocusPane::Inspector);
    assert_eq!(app.output_selected, selected_before_focus_switch);
}
