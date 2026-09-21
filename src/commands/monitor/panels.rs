use super::*;

pub(super) fn render_config(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    let Some((cfg, _)) = app.ready_config() else {
        let text = Paragraph::new("Configuration is not runnable. Open Messages for diagnostics.")
            .block(accent_panel(" CONFIG "))
            .wrap(Wrap { trim: true });
        frame.render_widget(text, area);
        return;
    };

    let (overview_area, channels_area) = config_panel_layout(area);

    let summary = vec![
        vec!["Version".to_string(), cfg.version.to_string()],
        vec![
            "Roles".to_string(),
            format!(
                "sensor={:?}, reference=ch{}, signal={:?}",
                cfg.roles.sensor_ch, cfg.roles.reference_ch, cfg.roles.signal_ch
            ),
        ],
        vec![
            "Lock-in".to_string(),
            format!(
                "{}, workers={}, stride={}",
                cfg.lockin.estimator_name(),
                cfg.lockin.workers,
                cfg.lockin.stride_samples
            ),
        ],
        vec![
            "Moke".to_string(),
            format!("{:?}, factor={}", cfg.moke.moke_type, cfg.moke.factor),
        ],
    ];
    frame.render_widget(
        two_col_table(summary, " OVERVIEW ", overview_area.width),
        overview_area,
    );

    let visible_rows = table_visible_rows(channels_area);
    let inner_width = channels_area.width.saturating_sub(6) as usize;
    let channel_width = 8;
    let role_width = 16;
    let unit_width = 10;
    let factor_width = 14;
    let label_width = inner_width
        .saturating_sub(channel_width + role_width + unit_width + factor_width)
        .max(8);
    let total = cfg.channels.len();
    let start = app.config_scroll.min(total.saturating_sub(visible_rows));
    let end = (start + visible_rows).min(total);
    let rows = cfg
        .channels
        .iter()
        .skip(start)
        .take(visible_rows)
        .map(|channel| {
            Row::new(vec![
                format!("ch{}", channel.index),
                fit_text(&channel_role(cfg, channel.index), role_width),
                fit_text(
                    &channel.label.clone().unwrap_or_else(|| "-".to_string()),
                    label_width,
                ),
                fit_text(
                    &channel.unit_out.clone().unwrap_or_else(|| "-".to_string()),
                    unit_width,
                ),
                fit_text(
                    &channel
                        .factor
                        .map(|factor| format!("{factor:.4e}"))
                        .unwrap_or_else(|| "-".to_string()),
                    factor_width,
                ),
            ])
        })
        .collect::<Vec<_>>();
    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(18),
            Constraint::Percentage(25),
            Constraint::Length(12),
            Constraint::Percentage(25),
        ],
    )
    .header(
        Row::new(vec!["Channel", "Role", "Label", "Unit", "Factor"]).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(accent_panel(visible_range_title(
        "CHANNELS", start, end, total,
    )));
    frame.render_widget(table, channels_area);
}

pub(super) fn message_lines(app: &MonitorApp) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if let Some((_, warnings)) = app.ready_config() {
        if warnings.is_empty() {
            lines.push(Line::styled(
                "No warnings.",
                Style::default().fg(Color::Green),
            ));
        } else {
            for warning in warnings {
                lines.push(Line::from(vec![
                    Span::styled("WARN ", Style::default().fg(Color::Yellow)),
                    Span::raw(warning.message.clone()),
                ]));
            }
        }
    }

    if let Some(diag) = app.diagnostics() {
        lines.push(Line::styled(
            format!(
                "Config version: {}",
                diag.version.map_or("-".to_string(), |v| v.to_string())
            ),
            Style::default().fg(Color::Gray),
        ));
        for warning in &diag.warnings {
            lines.push(Line::from(vec![
                Span::styled("WARN ", Style::default().fg(Color::Yellow)),
                Span::raw(warning.message.clone()),
            ]));
        }
        for issue in &diag.diagnostics {
            let path = issue.path.as_deref().unwrap_or("-");
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{} ", issue.kind),
                    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!("[{path}] "), Style::default().fg(Color::Gray)),
                Span::raw(issue.message.clone()),
            ]));
            if let Some(suggestion) = &issue.suggestion {
                lines.push(Line::from(vec![
                    Span::styled("  hint ", Style::default().fg(Color::Cyan)),
                    Span::raw(suggestion.clone()),
                ]));
            }
        }
    }

    lines
}

pub(super) fn message_visual_lines(app: &MonitorApp, width: u16) -> Vec<Line<'static>> {
    wrap_styled_lines(message_lines(app), width.max(1))
}

pub(super) fn wrap_styled_lines(lines: Vec<Line<'static>>, width: u16) -> Vec<Line<'static>> {
    let mut wrapped = Vec::new();
    for line in lines {
        let line_style = line.style;
        let alignment = line.alignment;
        let mut current_spans = Vec::new();
        let mut current_width = 0u16;

        for span in line.spans {
            let mut chunk = String::new();
            for ch in span.content.chars() {
                let ch_width =
                    u16::try_from(UnicodeWidthChar::width(ch).unwrap_or(0)).unwrap_or(u16::MAX);
                if current_width > 0 && current_width.saturating_add(ch_width) > width {
                    if !chunk.is_empty() {
                        current_spans.push(Span::styled(std::mem::take(&mut chunk), span.style));
                    }
                    wrapped.push(Line {
                        style: line_style,
                        alignment,
                        spans: std::mem::take(&mut current_spans),
                    });
                    current_width = 0;
                }
                chunk.push(ch);
                current_width = current_width.saturating_add(ch_width);
            }
            if !chunk.is_empty() {
                current_spans.push(Span::styled(chunk, span.style));
            }
        }

        wrapped.push(Line {
            style: line_style,
            alignment,
            spans: current_spans,
        });
    }
    wrapped
}

pub(super) fn render_messages(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    let lines = message_visual_lines(app, area.width.saturating_sub(2));
    let max_scroll = lines
        .len()
        .saturating_sub(area.height.saturating_sub(2) as usize);
    let scroll = app.messages_scroll.min(max_scroll);

    let paragraph = Paragraph::new(lines)
        .block(accent_panel(" MESSAGES ").border_style(focus_border_style(
            app,
            FocusPane::Inspector,
            Color::DarkGray,
        )))
        .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0));
    frame.render_widget(paragraph, area);
}

pub(super) fn render_files(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    let artifacts = artifact_rows(app.ready_config().map(|(cfg, _)| cfg));
    let visible_rows = table_visible_rows(area);
    let inner_width = area.width.saturating_sub(6) as usize;
    let name_width = percent_width(inner_width, 22);
    let path_width = percent_width(inner_width, 38);
    let size_width = percent_width(inner_width, 14);
    let modified_width = percent_width(inner_width, 14);
    let state_width = percent_width(inner_width, 12);
    let total = artifacts.len();
    let start = app.files_scroll.min(total.saturating_sub(visible_rows));
    let end = (start + visible_rows).min(total);
    let rows = artifacts
        .into_iter()
        .skip(start)
        .take(visible_rows)
        .map(|artifact| {
            Row::new(vec![
                fit_text(&artifact.name, name_width),
                fit_path(&artifact.path, path_width),
                fit_text(&artifact.size, size_width),
                fit_text(&artifact.modified, modified_width),
                fit_text(artifact.state, state_width),
            ])
            .style(Style::default().fg(artifact.color))
        })
        .collect::<Vec<_>>();

    let table = Table::new(
        rows,
        [
            Constraint::Percentage(22),
            Constraint::Percentage(38),
            Constraint::Percentage(14),
            Constraint::Percentage(14),
            Constraint::Percentage(12),
        ],
    )
    .header(
        Row::new(vec!["Artifact", "Path", "Size", "Modified", "State"]).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(
        accent_panel(visible_range_title("FILES", start, end, total)).border_style(
            focus_border_style(app, FocusPane::Inspector, Color::DarkGray),
        ),
    );
    frame.render_widget(table, area);
}

pub(super) fn render_help_overlay(frame: &mut Frame<'_>, app: &MonitorApp, area: Rect) {
    let popup = centered_rect(70, 70, area);
    let selected = app.selected_action();
    let mut lines = vec![
        Line::from(vec![
            Span::styled(
                "pmoke TUI",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("  "),
            Span::styled("? / q", Style::default().fg(Color::Yellow)),
            Span::raw(" close"),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Selected  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                selected.label(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::styled(selected.description(), Style::default().fg(Color::Gray)),
        Line::raw(""),
    ];
    // FR-02: every row below is generated from the keybinding registry, so
    // the overlay cannot drift from the implementation. `help_sections`
    // owns the grouping; only the header above and the notes below are
    // curated text (neither is a key binding).
    for (title, rows) in help_sections() {
        lines.push(Line::styled(
            title.to_string(),
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        for binding in rows {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{:<22}", binding.label()),
                    Style::default().fg(Color::Cyan),
                ),
                Span::styled(binding.description, Style::default().fg(Color::Gray)),
            ]));
        }
        lines.push(Line::raw(""));
    }
    lines.push(Line::from(vec![
        Span::styled("Mouse  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            if app.mouse_capture {
                "capture on (click/drag/wheel); PMOKE_MOUSE=off disables"
            } else {
                "capture off (PMOKE_MOUSE=off)"
            },
            Style::default().fg(Color::Gray),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Keymap  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "fixed in S1; prefs file planned at $XDG_CONFIG_HOME/pmoke/keymap.toml",
            Style::default().fg(Color::Gray),
        ),
    ]));
    lines.push(Line::from(vec![
        Span::styled("`:`  ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            "reserved for a future command palette; history stays on [ ]",
            Style::default().fg(Color::Gray),
        ),
    ]));

    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .title(" Help ")
                    .borders(Borders::ALL)
                    .border_type(BorderType::Plain)
                    .border_style(Style::default().fg(Color::Cyan)),
            )
            .wrap(Wrap { trim: true }),
        popup,
    );
}

pub(super) fn run_label(app: &MonitorApp) -> String {
    if app.history_view.is_some()
        && let Some(record) = app.visible_run_record()
    {
        format!(
            "HISTORY {} {} {} {}",
            if record.ok { "DONE" } else { "FAIL" },
            record.label,
            format_duration(record.elapsed),
            record.result
        )
    } else if let Some(run) = &app.active_run {
        format!(
            "{} {} {}",
            if run.cancel_requested {
                "STOPPING"
            } else {
                "RUN"
            },
            run.label,
            format_live_duration(run.started_at.elapsed())
        )
    } else if let Some(record) = app.visible_run_record() {
        format!(
            "{} {} {} {}",
            if record.ok { "DONE" } else { "FAIL" },
            record.label,
            format_duration(record.elapsed),
            record.result
        )
    } else {
        "IDLE".to_string()
    }
}

pub(super) struct ArtifactRow {
    name: String,
    path: String,
    size: String,
    modified: String,
    state: &'static str,
    color: Color,
}

pub(super) fn artifact_rows(cfg: Option<&Config>) -> Vec<ArtifactRow> {
    let mut files: Vec<(String, String)> = Vec::new();

    if let Some(cfg) = cfg {
        let resolver = cfg.resolver();
        files.push((
            "waveform CSV".to_string(),
            resolver.waveform_csv().display().to_string(),
        ));
        if uses_raw_waveform_artifact(cfg) {
            files.push((
                "acquisition".to_string(),
                resolver.acquisition_manifest().display().to_string(),
            ));
        }
        for &ch in cfg.phase_signal_ch() {
            files.push((
                format!("li ch{ch}"),
                resolver.lockin_xy_csv(ch).display().to_string(),
            ));
            files.push((
                format!("rotated ch{ch}"),
                resolver.lockin_rotated_csv(ch).display().to_string(),
            ));
        }
        files.push((
            "moke".to_string(),
            resolver.moke_csv().display().to_string(),
        ));
    }

    files
        .into_iter()
        .map(|(name, path)| {
            let status = file_status(&path);
            ArtifactRow {
                name,
                path,
                size: status.size,
                modified: status.modified,
                state: status.state,
                color: status.color,
            }
        })
        .collect()
}

pub(super) fn uses_raw_waveform_artifact(cfg: &Config) -> bool {
    matches!(cfg.fetch.output, FetchOutput::Raw | FetchOutput::CsvAndRaw)
        || matches!(
            cfg.fetch.analysis_input,
            FetchAnalysisInput::Raw | FetchAnalysisInput::Auto
        )
}

pub(super) struct FileStatus {
    state: &'static str,
    size: String,
    modified: String,
    color: Color,
}

pub(super) fn file_status(path: &str) -> FileStatus {
    match fs::metadata(path) {
        Ok(meta) => FileStatus {
            state: "ready",
            size: if meta.is_file() {
                format_file_size(meta.len())
            } else {
                "dir".to_string()
            },
            modified: meta
                .modified()
                .ok()
                .and_then(|time| time.elapsed().ok())
                .map(format_age)
                .unwrap_or_else(|| "-".to_string()),
            color: Color::Green,
        },
        Err(_) => FileStatus {
            state: "missing",
            size: "-".to_string(),
            modified: "-".to_string(),
            color: Color::DarkGray,
        },
    }
}

fn format_file_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub(super) fn two_col_table(
    rows: Vec<Vec<String>>,
    title: &'static str,
    width: u16,
) -> Table<'static> {
    let value_width = width.saturating_sub(22) as usize;
    Table::new(
        rows.into_iter().map(|row| {
            let item = row.first().cloned().unwrap_or_default();
            let value = row.get(1).cloned().unwrap_or_default();
            Row::new(vec![fit_text(&item, 14), fit_text(&value, value_width)])
        }),
        [Constraint::Length(16), Constraint::Min(20)],
    )
    .header(
        Row::new(vec!["Item", "Value"]).style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
    )
    .block(accent_panel(title))
}

pub(super) fn channel_role(cfg: &Config, ch: u8) -> String {
    let mut roles = Vec::new();
    if cfg.roles.sensor_ch.contains(&ch) {
        roles.push("sensor");
    }
    if cfg.roles.reference_ch == ch {
        roles.push("reference");
    }
    if cfg.roles.signal_ch.contains(&ch) {
        roles.push("signal");
    }
    if roles.is_empty() {
        "-".to_string()
    } else {
        roles.join(", ")
    }
}

pub(super) fn accent_panel(title: impl Into<String>) -> Block<'static> {
    Block::default()
        .title(title.into())
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(Color::DarkGray))
}

pub(super) fn focus_border_style(app: &MonitorApp, pane: FocusPane, fallback: Color) -> Style {
    if app.focus == pane {
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(fallback)
    }
}
