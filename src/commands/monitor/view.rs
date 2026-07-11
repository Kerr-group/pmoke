use super::*;

pub(super) fn render(frame: &mut Frame<'_>, app: &mut MonitorApp, effect_delta: FxDuration) {
    let area = frame.area();
    frame.render_widget(Clear, area);

    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(10),
            Constraint::Length(1),
        ])
        .split(area);

    render_header(frame, app, outer[0]);
    render_body(frame, app, outer[1], effect_delta);
    render_footer(frame, app, outer[2]);

    if app.show_help {
        render_help_overlay(frame, app, area);
    }
}

pub(super) fn render_header(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);
    let header =
        Paragraph::new(Line::from(header_spans(app, area.width))).alignment(Alignment::Left);
    frame.render_widget(header, chunks[0]);

    frame.render_widget(
        Paragraph::new(Line::from(context_bar_spans(app, area.width))),
        chunks[1],
    );
}

pub(super) fn header_spans(app: &MonitorApp, width: u16) -> Vec<Span<'static>> {
    let (status, color) = app.status();
    let run = fit_text(&run_label(app), width.saturating_sub(33) as usize);
    vec![
        Span::styled(
            " pmoke ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled("●", Style::default().fg(color)),
        Span::raw(" "),
        Span::styled(
            status,
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  │  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            run,
            Style::default()
                .fg(run_status_color(app))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  │  ", Style::default().fg(Color::DarkGray)),
        Span::styled(app.elapsed(), Style::default().fg(Color::DarkGray)),
    ]
}

pub(super) fn context_bar_spans(app: &MonitorApp, width: u16) -> Vec<Span<'static>> {
    let width = width as usize;
    if width == 0 {
        return Vec::new();
    }

    let mut spans = if width < CONTEXT_DETAILS_MIN_WIDTH {
        let full_prefix = "◆ cwd ";
        let icon_prefix = "◆ ";
        let prefix = if width >= full_prefix.cell_width() as usize {
            full_prefix
        } else if width >= icon_prefix.cell_width() as usize {
            icon_prefix
        } else {
            ""
        };
        vec![
            Span::styled(
                prefix,
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                fit_context_path(
                    &app.current_dir,
                    width.saturating_sub(prefix.cell_width() as usize),
                ),
                Style::default().fg(Color::White),
            ),
        ]
    } else {
        let fixed_width = [" └─ ", "◆", " cwd ", "  │  ", "config "]
            .iter()
            .map(|part| part.cell_width() as usize)
            .sum::<usize>();
        let available = width.saturating_sub(fixed_width);
        let config_budget = (app.config_path.cell_width() as usize).min(available / 3);
        let cwd_width =
            (app.current_dir.cell_width() as usize).min(available.saturating_sub(config_budget));
        let config_width =
            (app.config_path.cell_width() as usize).min(available.saturating_sub(cwd_width));
        vec![
            Span::styled(" └─ ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                "◆",
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" cwd ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                fit_context_path(&app.current_dir, cwd_width),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  │  ", Style::default().fg(Color::DarkGray)),
            Span::styled("config ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                fit_context_path(&app.config_path, config_width),
                Style::default().fg(Color::Gray),
            ),
        ]
    };

    let used = spans
        .iter()
        .map(|span| span.content.cell_width() as usize)
        .sum::<usize>();
    let remaining = width.saturating_sub(used);
    if remaining > 0 {
        let rule = "─";
        let rule_width = (rule.cell_width() as usize).max(1);
        let rule_area = remaining.saturating_sub(1);
        let filler = format!(
            " {}{}",
            rule.repeat(rule_area / rule_width),
            " ".repeat(rule_area % rule_width)
        );
        spans.push(Span::styled(filler, Style::default().fg(Color::DarkGray)));
    }
    spans
}

pub(super) fn fit_context_path(path: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if path.cell_width() as usize <= width {
        return path.to_string();
    }

    let prefix = if width > 3 { "..." } else { "" };
    let tail_width = width.saturating_sub(prefix.len());
    let mut tail = Vec::new();
    let mut used = 0usize;
    for ch in path.chars().rev() {
        let mut encoded = [0; 4];
        let ch_width = ch.encode_utf8(&mut encoded).cell_width() as usize;
        if used.saturating_add(ch_width) > tail_width {
            break;
        }
        tail.push(ch);
        used += ch_width;
    }
    tail.reverse();
    format!("{prefix}{}", tail.into_iter().collect::<String>())
}

pub(super) fn render_body(
    frame: &mut Frame<'_>,
    app: &mut MonitorApp,
    area: Rect,
    effect_delta: FxDuration,
) {
    let layout = UiLayout::new(area);
    render_command_palette(frame, app, layout.workflow);
    render_inspector(frame, app, layout.inspector);
    render_run_output(frame, app, layout.activity, effect_delta);
}

pub(super) fn render_footer(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    let focus = match app.focus {
        FocusPane::Commands => "workflow",
        FocusPane::Inspector => "inspector",
        FocusPane::Output => "activity",
    };
    let line = Line::from(vec![
        Span::styled(" Enter ", Style::default().fg(Color::Cyan)),
        Span::raw("run  "),
        Span::styled("Tab ", Style::default().fg(Color::Cyan)),
        Span::raw("focus  "),
        Span::styled("i ", Style::default().fg(Color::Cyan)),
        Span::raw("inspect  "),
        Span::styled("? ", Style::default().fg(Color::Cyan)),
        Span::raw("help  "),
        Span::styled(format!("[{focus}]"), Style::default().fg(Color::DarkGray)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

pub(super) fn process_event_feed_effects(
    app: &mut MonitorApp,
    effect_delta: FxDuration,
    buffer: &mut ratatui::buffer::Buffer,
    area: Option<Rect>,
) {
    if app.effects.is_running() {
        app.effects
            .process_effects(effect_delta, buffer, area.unwrap_or_default());
    }
}

pub(super) fn render_command_palette(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    let (list_area, description_area) = workflow_layout(area);
    let selected = app.workflow_cursor;
    let entries = app.workflow_entries();
    let visible_rows = list_area.height.saturating_sub(2).max(1) as usize;
    let start = selected.saturating_sub(visible_rows / 2);
    let items = entries
        .iter()
        .enumerate()
        .skip(start)
        .take(visible_rows)
        .map(|(idx, entry)| {
            let is_selected = idx == selected;
            let WorkflowEntry::Action(action) = entry else {
                let WorkflowEntry::Group(group) = entry else {
                    unreachable!()
                };
                let collapsed = app.collapsed_groups.contains(group);
                return ListItem::new(Line::from(vec![
                    Span::styled(
                        if is_selected { "▌ " } else { "  " },
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::styled(
                        if collapsed { "▸" } else { "▾" },
                        Style::default().fg(Color::Cyan),
                    ),
                    Span::raw(" "),
                    Span::styled(
                        group.label(),
                        Style::default()
                            .fg(if is_selected {
                                Color::White
                            } else {
                                Color::DarkGray
                            })
                            .add_modifier(Modifier::BOLD),
                    ),
                ]));
            };
            let runnable = action_runnable(*action, &app.load);
            let accent_color = if runnable { Color::Cyan } else { Color::Red };
            let selected_style = if is_selected {
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::Gray)
            };
            let marker = if is_selected { "▌" } else { " " };
            let icon = if runnable { "●" } else { "·" };
            let icon_style = if is_selected {
                Style::default()
                    .fg(accent_color)
                    .add_modifier(Modifier::BOLD)
            } else if runnable {
                Style::default().fg(Color::DarkGray)
            } else {
                Style::default().fg(Color::Red)
            };
            let badge = if let Some(run) = app
                .active_run
                .as_ref()
                .filter(|run| run.label == action.label())
            {
                if run.cancel_requested { "STP" } else { "RUN" }
            } else if runnable {
                "OK "
            } else {
                "-- "
            };
            let badge_style = if is_selected && badge == "STP" {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::LightRed)
                    .add_modifier(Modifier::BOLD)
            } else if is_selected {
                Style::default()
                    .fg(Color::Black)
                    .bg(accent_color)
                    .add_modifier(Modifier::BOLD)
            } else if badge == "RUN" || badge == "OK " {
                Style::default().fg(Color::Green)
            } else if badge == "STP" {
                Style::default().fg(Color::LightRed)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("{marker} "),
                    if is_selected {
                        Style::default()
                            .fg(accent_color)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    },
                ),
                Span::styled(
                    "  ",
                    if is_selected {
                        Style::default()
                            .fg(accent_color)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    },
                ),
                Span::raw(" "),
                Span::styled(icon, icon_style),
                Span::raw("  "),
                Span::styled(action.command_name(), selected_style),
                Span::raw(" "),
                Span::styled(badge.trim(), badge_style),
            ]))
        })
        .collect::<Vec<_>>();

    let title = if entries.is_empty() {
        format!(" WORKFLOW /{} · NO MATCHES ", app.action_query)
    } else if app.search_mode || !app.action_query.is_empty() {
        format!(" WORKFLOW /{} ", app.action_query)
    } else {
        format!(" WORKFLOW {:02}/{} ", selected + 1, entries.len())
    };

    frame.render_widget(
        List::new(items).block(accent_panel(title).border_style(focus_border_style(
            app,
            FocusPane::Commands,
            Color::DarkGray,
        ))),
        list_area,
    );
    render_command_description(frame, app, description_area);
}

pub(super) fn render_inspector(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    match app.inspector_view {
        InspectorView::Summary => render_inspector_summary(frame, app, area),
        InspectorView::Config => render_config(frame, app, area),
        InspectorView::Diagnostics => render_messages(frame, app, area),
        InspectorView::Artifacts => render_files(frame, app, area),
    }
}

fn render_inspector_summary(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    if app.selected_workflow_entry().is_none() {
        frame.render_widget(
            Paragraph::new("No workflow action matches the current search.")
                .style(Style::default().fg(Color::Yellow))
                .block(
                    accent_panel(format!(" INSPECTOR · {} ", app.inspector_view.label()))
                        .border_style(focus_border_style(
                            app,
                            FocusPane::Inspector,
                            Color::DarkGray,
                        )),
                )
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    if let Some(WorkflowEntry::Group(group)) = app.selected_workflow_entry() {
        let collapsed = app.collapsed_groups.contains(&group);
        let lines = vec![
            Line::styled(
                group.label(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Line::styled(
                if collapsed {
                    "Enter to expand this workflow group."
                } else {
                    "Enter to collapse this workflow group."
                },
                Style::default().fg(Color::Gray),
            ),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .block(
                    accent_panel(format!(" INSPECTOR · {} ", app.inspector_view.label()))
                        .border_style(focus_border_style(
                            app,
                            FocusPane::Inspector,
                            Color::DarkGray,
                        )),
                )
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }
    let action = app.selected_action();
    let readiness = action_readiness(action, &app.load);
    let color = if readiness.is_ok() {
        Color::Green
    } else {
        Color::Red
    };
    let state = if readiness.is_ok() {
        "READY"
    } else {
        "BLOCKED"
    };
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                format!(" {state} "),
                Style::default()
                    .fg(Color::Black)
                    .bg(color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled(
                action.label(),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::styled(action.description(), Style::default().fg(Color::Gray)),
        Line::from(vec![
            Span::styled("command  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!(
                    "pmoke --config {} {}",
                    app.config_path,
                    action.command_name()
                ),
                Style::default().fg(Color::Cyan),
            ),
        ]),
    ];
    if let Err(reason) = readiness {
        lines.push(Line::from(vec![
            Span::styled("reason   ", Style::default().fg(Color::DarkGray)),
            Span::styled(reason, Style::default().fg(Color::LightRed)),
        ]));
    }
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                accent_panel(format!(" INSPECTOR · {} ", app.inspector_view.label())).border_style(
                    focus_border_style(app, FocusPane::Inspector, Color::DarkGray),
                ),
            )
            .wrap(Wrap { trim: true }),
        area,
    );
}

pub(super) fn render_command_description(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    if area.height == 0 {
        return;
    }

    let Some(entry) = app.selected_workflow_entry() else {
        frame.render_widget(
            Paragraph::new("Type to refine the search; Esc clears it.")
                .style(Style::default().fg(Color::Gray))
                .block(accent_panel(" DETAIL ")),
            area,
        );
        return;
    };
    let action = app.selected_action();
    if let WorkflowEntry::Group(group) = entry {
        frame.render_widget(
            Paragraph::new("Enter toggles this group.")
                .style(Style::default().fg(Color::Gray))
                .block(accent_panel(format!(" DETAIL {} ", group.label()))),
            area,
        );
        return;
    }
    frame.render_widget(
        Paragraph::new(action.description())
            .style(Style::default().fg(Color::Gray))
            .block(accent_panel(format!(" DETAIL {} ", action.command_name())))
            .wrap(Wrap { trim: true }),
        area,
    );
}

pub(super) fn render_run_output(
    frame: &mut Frame<'_>,
    app: &mut MonitorApp,
    area: Rect,
    effect_delta: FxDuration,
) {
    let block_base = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(focus_border_style(
            app,
            FocusPane::Output,
            run_status_color(app),
        ));
    let inner = block_base.inner(area);
    let output_sections = output_inner_layout(inner);
    let log_area = output_sections.log;
    let log_width = log_area.width.saturating_sub(1);
    let visual_lines_for_layout = if app.run_output.is_empty() {
        Vec::new()
    } else {
        visual_output_lines(&app.run_output, log_width, None, None)
    };
    let visual_line_count = visual_lines_for_layout.len();
    let selected_visual_range =
        visual_selection_range(&visual_lines_for_layout, app.output_selection_range());
    let visible_rows = output_visible_rows(log_area);
    let effective_scroll = effective_output_scroll(app, log_area, visual_line_count);
    let title = if app.run_output.is_empty() {
        " OUTPUT ".to_string()
    } else if effective_scroll == 0 {
        format!(" OUTPUT latest · {visual_line_count} lines ")
    } else {
        format!(" OUTPUT -{effective_scroll} lines ")
    };

    let block = block_base.title(title);
    frame.render_widget(block, area);

    render_output_status_bar(
        frame,
        app,
        output_sections.status,
        visual_line_count,
        effective_scroll,
        selected_visual_range,
    );
    render_run_timeline(frame, app, output_sections.timeline);

    if log_area.height == 0 || log_area.width == 0 {
        process_event_feed_effects(app, effect_delta, frame.buffer_mut(), None);
        return;
    }

    let lines = if app.run_output.is_empty() {
        vec![Line::styled(
            "  ready",
            Style::default().fg(Color::DarkGray),
        )]
    } else {
        let visual_lines = visual_output_lines_with_motion(
            &app.run_output,
            log_width,
            app.output_selection_range(),
            app.output_selected,
            app.command_running(),
            timeline_motion_frame(app),
        );
        let end = visual_lines.len().saturating_sub(effective_scroll);
        let start = end.saturating_sub(visible_rows);
        visual_lines
            .into_iter()
            .skip(start)
            .take(end.saturating_sub(start))
            .map(|line| line.line)
            .collect()
    };
    let log_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(log_area);
    frame.render_widget(
        output_header(
            log_chunks[0].width,
            app.command_running(),
            timeline_motion_frame(app),
        ),
        log_chunks[0],
    );
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }),
        log_chunks[1],
    );

    let effect_area = latest_event_feed_effect_area(
        log_chunks[1],
        visual_line_count,
        visible_rows,
        effective_scroll,
    );
    process_event_feed_effects(app, effect_delta, frame.buffer_mut(), effect_area);

    render_output_scrollbar(
        frame.buffer_mut(),
        log_chunks[1],
        visual_line_count,
        visible_rows,
        effective_scroll,
    );
}

pub(super) fn render_output_scrollbar(
    buffer: &mut Buffer,
    area: Rect,
    visual_line_count: usize,
    visible_rows: usize,
    effective_scroll: usize,
) {
    let Some((thumb_start, thumb_len)) = output_scrollbar_thumb(
        visual_line_count,
        visible_rows,
        effective_scroll,
        area.height,
    ) else {
        return;
    };
    if area.width == 0 {
        return;
    }

    let x = area.right().saturating_sub(1);
    let thumb_end = thumb_start.saturating_add(thumb_len);
    for row in 0..area.height as usize {
        let (symbol, style) = if (thumb_start..thumb_end).contains(&row) {
            (
                "█",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            ("│", Style::default().fg(Color::DarkGray))
        };
        buffer.set_string(x, area.y + row as u16, symbol, style);
    }
}

pub(super) fn output_scrollbar_thumb(
    visual_line_count: usize,
    visible_rows: usize,
    effective_scroll: usize,
    track_height: u16,
) -> Option<(usize, usize)> {
    let track_height = track_height as usize;
    if visual_line_count <= visible_rows || visible_rows == 0 || track_height == 0 {
        return None;
    }

    let max_scroll = visual_line_count.saturating_sub(visible_rows);
    let position_from_top = max_scroll.saturating_sub(effective_scroll.min(max_scroll));
    let thumb_len = visible_rows
        .saturating_mul(track_height)
        .div_ceil(visual_line_count)
        .clamp(1, track_height);
    let max_thumb_start = track_height.saturating_sub(thumb_len);
    let thumb_start = position_from_top
        .saturating_mul(max_thumb_start)
        .saturating_add(max_scroll / 2)
        .checked_div(max_scroll)
        .unwrap_or(0);

    Some((thumb_start, thumb_len))
}

pub(super) fn output_header(width: u16, running: bool, frame: usize) -> Paragraph<'static> {
    Paragraph::new(Line::from(output_header_spans_with_motion(
        width, running, frame,
    )))
}

#[cfg(test)]
pub(super) fn output_header_spans(width: u16) -> Vec<Span<'static>> {
    output_header_spans_with_motion(width, false, 0)
}

pub(super) fn output_header_spans_with_motion(
    width: u16,
    running: bool,
    frame: usize,
) -> Vec<Span<'static>> {
    let scanner = event_feed_spinner_symbol(frame);
    let scanner_style = Style::default()
        .fg(event_feed_pulse_color(frame))
        .add_modifier(Modifier::BOLD);
    let mut spans = vec![
        Span::styled(
            " EVENT FEED ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        if running {
            Span::styled(format!("{scanner} live"), scanner_style)
        } else {
            Span::styled("analysis output", Style::default().fg(Color::Gray))
        },
    ];

    if width >= 60 {
        spans.extend([
            Span::raw("  "),
            Span::styled("●", Style::default().fg(Color::Green)),
            Span::raw(" "),
            Span::styled("ok", Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled("●", Style::default().fg(Color::Cyan)),
            Span::raw(" "),
            Span::styled("info", Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled("●", Style::default().fg(Color::Yellow)),
            Span::raw(" "),
            Span::styled("warn", Style::default().fg(Color::DarkGray)),
            Span::raw(" "),
            Span::styled("●", Style::default().fg(Color::LightRed)),
            Span::raw(" "),
            Span::styled("error", Style::default().fg(Color::DarkGray)),
        ]);
    }

    spans
}

pub(super) fn event_feed_spinner_symbol(frame: usize) -> char {
    spinner_frame(FluxFrames::PISTON, frame)
}

pub(super) fn event_feed_pulse_color(frame: usize) -> Color {
    if frame.is_multiple_of(2) {
        Color::LightCyan
    } else {
        Color::Cyan
    }
}

pub(super) fn render_output_status_bar(
    frame: &mut Frame<'_>,
    app: &MonitorApp,
    area: Rect,
    visual_line_count: usize,
    effective_scroll: usize,
    selected_visual_range: Option<(usize, usize)>,
) {
    let (state, color) = if let Some(run) = &app.active_run {
        if run.cancel_requested {
            ("STOPPING", Color::LightRed)
        } else {
            ("RUNNING", Color::Yellow)
        }
    } else if app
        .last_run
        .as_ref()
        .map(|record| record.ok)
        .unwrap_or(true)
    {
        ("READY", Color::Green)
    } else {
        ("FAILED", Color::Red)
    };
    let scroll = if effective_scroll == 0 {
        "latest".to_string()
    } else {
        format!("-{effective_scroll} lines")
    };
    let selection = output_selection_status(selected_visual_range);

    let mut spans = vec![
        Span::styled(
            format!(" {state} "),
            Style::default()
                .fg(Color::Black)
                .bg(color)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled("lines ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            visual_line_count.to_string(),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled("  /  ", Style::default().fg(Color::DarkGray)),
        Span::styled(selection, Style::default().fg(Color::Gray)),
        Span::styled("  /  ", Style::default().fg(Color::DarkGray)),
        Span::styled(scroll, Style::default().fg(Color::Cyan)),
    ];
    if let Some(status) = &app.copy_status {
        spans.extend([
            Span::styled("  │  ", Style::default().fg(Color::DarkGray)),
            Span::styled(status.clone(), Style::default().fg(Color::Cyan)),
        ]);
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}
