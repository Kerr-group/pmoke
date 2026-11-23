use super::*;
use ratatui::backend::TestBackend;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl_key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::CONTROL)
}

fn char_key(ch: char) -> KeyEvent {
    key(KeyCode::Char(ch))
}

fn rendered_help(width: u16, height: u16, app: &mut MonitorApp) -> String {
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

fn footer_text(app: &MonitorApp) -> String {
    footer_spans(app, 120)
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

fn running_app() -> MonitorApp {
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
    app
}

/// FR-01: the registry must be unambiguous — in every UI state, matching
/// bindings for one key have distinct specificity (modal > search > pane >
/// global) and first-match resolves to the most specific one.
#[test]
fn registry_resolves_deterministically_in_every_ui_state() {
    fn pane_context(focus: FocusPane) -> KeyContext {
        match focus {
            FocusPane::Commands => KeyContext::Commands,
            FocusPane::Runs => KeyContext::Runs,
            FocusPane::Inspector => KeyContext::Inspector,
            FocusPane::Output => KeyContext::Output,
        }
    }
    fn specificity(binding: &KeyBinding, focus: FocusPane, search: bool, help: bool) -> Option<u8> {
        if help {
            return binding
                .contexts
                .contains(&KeyContext::HelpOpen)
                .then_some(3);
        }
        let mut rank = None;
        if binding.contexts.contains(&KeyContext::Global) {
            rank = Some(0);
        }
        if binding.contexts.contains(&pane_context(focus)) {
            rank = Some(rank.unwrap_or(0).max(1));
        }
        if search && binding.contexts.contains(&KeyContext::Searching) {
            rank = Some(rank.unwrap_or(0).max(2));
        }
        rank
    }
    let focuses = [
        FocusPane::Commands,
        FocusPane::Runs,
        FocusPane::Inspector,
        FocusPane::Output,
    ];
    for focus in focuses {
        for search_mode in [false, true] {
            for show_help in [false, true] {
                for binding in REGISTRY {
                    for spec in binding.keys {
                        let event = KeyEvent::new(spec.code, spec.modifiers);
                        let matches = REGISTRY
                            .iter()
                            .filter(|candidate| {
                                candidate.keys.iter().any(|candidate_spec| {
                                    candidate_spec.code == event.code
                                        && event.modifiers.contains(candidate_spec.modifiers)
                                }) && specificity(candidate, focus, search_mode, show_help)
                                    .is_some()
                            })
                            .map(|candidate| {
                                (
                                    candidate.action,
                                    specificity(candidate, focus, search_mode, show_help)
                                        .expect("matched"),
                                )
                            })
                            .collect::<Vec<_>>();
                        let mut ranks = matches.iter().map(|(_, rank)| rank).collect::<Vec<_>>();
                        ranks.sort_unstable();
                        ranks.dedup();
                        assert_eq!(
                            ranks.len(),
                            matches.len(),
                            "ambiguous: {event:?} focus={focus:?} \
                             search={search_mode} help={show_help}"
                        );
                        if !matches.is_empty() {
                            let best = matches.iter().max_by_key(|(_, rank)| rank).unwrap().0;
                            assert_eq!(
                                resolve_key(&event, focus, search_mode, show_help),
                                Some(best),
                                "priority: {event:?} focus={focus:?} \
                                 search={search_mode} help={show_help}"
                            );
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn resolve_global_keys_from_idle_commands_focus() {
    let focus = FocusPane::Commands;
    assert_eq!(
        resolve_key(&char_key('?'), focus, false, false),
        Some(TuiAction::ToggleHelp)
    );
    assert_eq!(
        resolve_key(&char_key('q'), focus, false, false),
        Some(TuiAction::RequestQuit)
    );
    assert_eq!(
        resolve_key(&char_key('r'), focus, false, false),
        Some(TuiAction::Refresh)
    );
    assert_eq!(
        resolve_key(&char_key('['), focus, false, false),
        Some(TuiAction::HistoryPrev)
    );
    assert_eq!(
        resolve_key(&char_key(']'), focus, false, false),
        Some(TuiAction::HistoryNext)
    );
    assert_eq!(
        resolve_key(&char_key('/'), focus, false, false),
        Some(TuiAction::BeginSearch)
    );
    assert_eq!(
        resolve_key(&key(KeyCode::End), focus, false, false),
        Some(TuiAction::FollowOutput)
    );
    assert_eq!(
        resolve_key(&ctrl_key(KeyCode::Char('c')), focus, false, false),
        Some(TuiAction::Interrupt)
    );
    assert_eq!(
        resolve_key(&key(KeyCode::Esc), focus, false, false),
        Some(TuiAction::Escape)
    );
}

#[test]
fn resolve_pane_specific_keys_beat_their_global_fallback() {
    assert_eq!(
        resolve_key(&key(KeyCode::Enter), FocusPane::Output, false, false),
        Some(TuiAction::CopySelection)
    );
    assert_eq!(
        resolve_key(&key(KeyCode::Enter), FocusPane::Commands, false, false),
        Some(TuiAction::RunSelected)
    );
    assert_eq!(
        resolve_key(&key(KeyCode::Enter), FocusPane::Runs, false, false),
        Some(TuiAction::PinRun)
    );
    assert_eq!(
        resolve_key(&char_key('k'), FocusPane::Output, false, false),
        Some(TuiAction::OutputPrev)
    );
    assert_eq!(
        resolve_key(&char_key('k'), FocusPane::Inspector, false, false),
        Some(TuiAction::InspectorUp)
    );
    assert_eq!(
        resolve_key(&char_key('k'), FocusPane::Runs, false, false),
        Some(TuiAction::RunsPrev)
    );
    assert_eq!(
        resolve_key(&char_key('j'), FocusPane::Runs, false, false),
        Some(TuiAction::RunsNext)
    );
    assert_eq!(
        resolve_key(&char_key('k'), FocusPane::Commands, false, false),
        Some(TuiAction::CommandsPrev)
    );
    assert_eq!(
        resolve_key(&char_key('g'), FocusPane::Output, false, false),
        Some(TuiAction::OutputFirst)
    );
    assert_eq!(
        resolve_key(&char_key('g'), FocusPane::Commands, false, false),
        Some(TuiAction::CommandsFirst)
    );
    assert_eq!(
        resolve_key(&char_key('g'), FocusPane::Runs, false, false),
        Some(TuiAction::RunsFirst)
    );
    assert_eq!(
        resolve_key(&char_key('G'), FocusPane::Runs, false, false),
        Some(TuiAction::RunsLast)
    );
}

#[test]
fn resolve_search_state_prefers_search_bindings() {
    assert_eq!(
        resolve_key(&key(KeyCode::Enter), FocusPane::Commands, true, false),
        Some(TuiAction::SearchCommit)
    );
    assert_eq!(
        resolve_key(&key(KeyCode::Backspace), FocusPane::Commands, true, false),
        Some(TuiAction::SearchBackspace)
    );
    assert_eq!(
        resolve_key(&key(KeyCode::Esc), FocusPane::Commands, true, false),
        Some(TuiAction::Escape)
    );
    // `?` and `/` have no Searching context, so the registry still reports
    // their global binding; the event loop types them into the query instead
    // (text-entry precedence, pinned end-to-end below).
    assert_eq!(
        resolve_key(&char_key('?'), FocusPane::Commands, true, false),
        Some(TuiAction::ToggleHelp)
    );
    assert_eq!(
        resolve_key(&char_key('/'), FocusPane::Commands, true, false),
        Some(TuiAction::BeginSearch)
    );
    // `Ctrl+C` still interrupts while searching.
    assert_eq!(
        resolve_key(
            &ctrl_key(KeyCode::Char('c')),
            FocusPane::Commands,
            true,
            false
        ),
        Some(TuiAction::Interrupt)
    );
}

#[test]
fn resolve_help_modal_only_answers_close_keys() {
    for focus in [
        FocusPane::Commands,
        FocusPane::Runs,
        FocusPane::Inspector,
        FocusPane::Output,
    ] {
        assert_eq!(
            resolve_key(&char_key('?'), focus, false, true),
            Some(TuiAction::ToggleHelp)
        );
        assert_eq!(
            resolve_key(&char_key('q'), focus, false, true),
            Some(TuiAction::CloseHelp)
        );
        assert_eq!(
            resolve_key(&key(KeyCode::Esc), focus, false, true),
            Some(TuiAction::Escape)
        );
        assert_eq!(
            resolve_key(&ctrl_key(KeyCode::Char('c')), focus, false, true),
            Some(TuiAction::Interrupt)
        );
        assert_eq!(resolve_key(&char_key('r'), focus, false, true), None);
        assert_eq!(resolve_key(&key(KeyCode::Enter), focus, false, true), None);
        assert_eq!(resolve_key(&char_key('j'), focus, false, true), None);
    }
}

#[test]
fn resolve_numbered_focus_and_inspector_tabs() {
    // FR-03: 1-4 focus panes globally (S2 claims `4` for the runs browser)...
    assert_eq!(
        resolve_key(&char_key('1'), FocusPane::Commands, false, false),
        Some(TuiAction::FocusPane(FocusPane::Commands))
    );
    assert_eq!(
        resolve_key(&char_key('2'), FocusPane::Commands, false, false),
        Some(TuiAction::FocusPane(FocusPane::Inspector))
    );
    assert_eq!(
        resolve_key(&char_key('3'), FocusPane::Output, false, false),
        Some(TuiAction::FocusPane(FocusPane::Output))
    );
    // ...while the inspector is focused, 1-5 select its tabs instead.
    assert_eq!(
        resolve_key(&char_key('1'), FocusPane::Inspector, false, false),
        Some(TuiAction::InspectorTab(InspectorView::Summary))
    );
    assert_eq!(
        resolve_key(&char_key('2'), FocusPane::Inspector, false, false),
        Some(TuiAction::InspectorTab(InspectorView::Config))
    );
    assert_eq!(
        resolve_key(&char_key('3'), FocusPane::Inspector, false, false),
        Some(TuiAction::InspectorTab(InspectorView::Diagnostics))
    );
    assert_eq!(
        resolve_key(&char_key('4'), FocusPane::Inspector, false, false),
        Some(TuiAction::InspectorTab(InspectorView::Artifacts))
    );
    // S3 claims `5` for the REPORTS tab; it is inspector-local only, so
    // `5` stays unbound everywhere else.
    assert_eq!(
        resolve_key(&char_key('5'), FocusPane::Inspector, false, false),
        Some(TuiAction::InspectorTab(InspectorView::Reports))
    );
    for focus in [FocusPane::Commands, FocusPane::Runs, FocusPane::Output] {
        assert_eq!(
            resolve_key(&char_key('5'), focus, false, false),
            None,
            "5 is unbound outside the inspector ({focus:?})"
        );
    }
    // `4` is claimed by the S2 run browser outside the inspector, where it
    // focuses the Runs section; inside the inspector it still selects tabs.
    assert_eq!(
        resolve_key(&char_key('4'), FocusPane::Commands, false, false),
        Some(TuiAction::FocusRuns)
    );
    assert_eq!(
        resolve_key(&char_key('4'), FocusPane::Runs, false, false),
        Some(TuiAction::FocusRuns)
    );
    assert_eq!(
        resolve_key(&char_key('4'), FocusPane::Output, false, false),
        Some(TuiAction::FocusRuns)
    );
    assert_eq!(
        resolve_key(&char_key('b'), FocusPane::Commands, false, false),
        Some(TuiAction::FocusRuns)
    );
    assert_eq!(
        resolve_key(&char_key('b'), FocusPane::Output, false, false),
        Some(TuiAction::FocusRuns)
    );
}

#[test]
fn colon_stays_reserved_for_a_future_command_palette() {
    // FR-08: adopting `:` requires relocating `[`/`]` as a set, so `:` is
    // unbound in every UI state.
    for focus in [
        FocusPane::Commands,
        FocusPane::Runs,
        FocusPane::Inspector,
        FocusPane::Output,
    ] {
        for search_mode in [false, true] {
            for show_help in [false, true] {
                assert_eq!(
                    resolve_key(&char_key(':'), focus, search_mode, show_help),
                    None,
                    "focus={focus:?} search={search_mode} help={show_help}"
                );
            }
        }
    }
}

#[test]
fn help_overlay_lists_every_registered_binding() {
    // FR-02 drift test, implementation side: every registry row appears in
    // the generated `?` sections by key label and description.
    for (_, rows) in help_sections() {
        assert!(!rows.is_empty());
    }
    // NOTE: `REGISTRY` is a `const` (fresh copy per use), so coverage is
    // compared by value, not by address.
    let covered: Vec<KeyBinding> = help_sections()
        .iter()
        .flat_map(|(_, rows)| rows.iter().copied().copied())
        .collect();
    assert_eq!(
        covered.len(),
        REGISTRY.len(),
        "every registry row appears in exactly one help section"
    );
    for binding in REGISTRY {
        assert!(
            covered.contains(binding),
            "help is missing {:?}",
            binding.action
        );
    }
    let mut app = test_app();
    app.show_help = true;
    // Tall enough that the 70%-height popup fits every generated row plus
    // the curated notes; clipping would hide the drift the test guards.
    // (S2 added the RUNS section, so the floor moved from 100 to 120.)
    let rendered = rendered_help(120, 120, &mut app);
    for binding in REGISTRY {
        assert!(
            rendered.contains(&binding.label()),
            "help overlay is missing key label {}",
            binding.label()
        );
        assert!(
            rendered.contains(binding.description),
            "help overlay is missing description {:?}",
            binding.description
        );
    }
    assert!(rendered.contains("GLOBAL"));
    assert!(rendered.contains("WORKFLOW"));
    assert!(rendered.contains("RUNS"));
    assert!(rendered.contains("INSPECTOR"));
    assert!(rendered.contains("ACTIVITY"));
    assert!(rendered.contains("SEARCH"));
}

#[test]
fn footer_hints_match_the_registry_per_focus() {
    use super::harness::{TINY, area_for};
    let mut app = test_app();
    let area = area_for(TINY);
    for (pane, next, hint) in [
        (FocusPane::Commands, FocusPane::Runs, "[/] Filter"),
        (FocusPane::Runs, FocusPane::Inspector, "[/] Filter"),
        (FocusPane::Inspector, FocusPane::Output, "[i] Tabs"),
        (FocusPane::Output, FocusPane::Commands, "[End] Follow"),
    ] {
        assert_eq!(app.focus, pane);
        let footer = footer_text(&app);
        for expected in [hint, "[Tab] Pane", "[?] Help", "[q] Quit"] {
            assert!(footer.contains(expected), "{pane:?}: {footer}");
        }
        assert_eq!(
            footer.contains("[Enter] Run show"),
            matches!(pane, FocusPane::Commands | FocusPane::Inspector)
        );
        assert_eq!(
            handle_key(&mut app, key(KeyCode::Tab), area).unwrap(),
            KeyOutcome::Continue
        );
        assert_eq!(app.focus, next);
    }
    assert!(!app.command_running());
}

#[test]
fn footer_shows_quit_confirmation_state() {
    let mut app = running_app();
    assert!(!app.request_quit());
    assert!(app.quit_confirm_pending);
    let text = footer_text(&app);
    assert!(text.contains("[q] Stop + quit"));
    assert!(text.contains("[Esc] Stay"));
}

#[test]
fn empty_reports_restores_its_runs_hint_when_an_armed_quit_outlives_the_child() {
    use super::harness::{WIDE, area_for, render_text};
    let mut app = running_app();
    let area = area_for(WIDE);
    for k in [char_key('2'), char_key('5'), char_key('q')] {
        assert_eq!(handle_key(&mut app, k, area).unwrap(), KeyOutcome::Continue);
    }
    let confirming = render_text(&mut app, WIDE);
    assert!(confirming.contains("[q] Stop + quit"));
    assert!(!confirming.contains("press b to focus"));
    // Finish between frames without a keypress. Keep the completion rescan
    // away from any real configuration or recorded runs.
    let missing = std::env::temp_dir().join(format!("pmoke_empty_reports_{}", std::process::id()));
    assert!(!missing.exists());
    app.config_path = missing.join("config.toml").display().to_string();
    app.finish_run(true, "done".into());
    assert!(!app.command_running());
    let idle = render_text(&mut app, WIDE);
    assert!(
        idle.contains("press b to focus") && idle.contains("[q] Quit"),
        "{idle}"
    );
    handle_key(&mut app, char_key('b'), area).unwrap();
    assert_eq!(app.focus, FocusPane::Runs);
}

#[test]
fn footer_drops_the_whole_run_hint_if_it_misses_by_a_cell() {
    let mut app = test_app();
    app.workflow_fixture = Some(vec![MonitorAction::Doctor]);
    let hint = "[Enter] Run doctor";
    let minimum = ("[?] Help ".len() + hint.len()) as u16;
    for width in [0, minimum - 1, minimum, 40] {
        let line = Line::from(footer_spans(&app, width));
        assert!(line.width() <= usize::from(width));
        let text = line.to_string();
        assert_eq!(text.contains(hint), width >= minimum, "{text}");
        if width < minimum {
            assert!(
                !text.contains("[Enter") && !text.contains("doctor"),
                "{text}"
            );
        }
    }
}

#[test]
fn search_hints_follow_plain_keys_even_if_a_modified_q_armed_quit() {
    let mut app = running_app();
    let area = super::harness::area_for(super::harness::TINY);
    handle_key(&mut app, char_key('/'), area).unwrap();
    handle_key(
        &mut app,
        KeyEvent::new(KeyCode::Char('q'), KeyModifiers::ALT),
        area,
    )
    .unwrap();
    assert!(app.search_mode && app.quit_confirm_pending);
    let text = footer_text(&app);
    assert!(
        text.contains("[Enter] Apply") && !text.contains("Stay"),
        "{text}"
    );
    let frame = super::harness::render_text(&mut app, super::harness::WIDE);
    assert!(!frame.contains("Press q again"), "{frame}");
    assert_eq!(
        handle_key(&mut app, char_key('q'), area).unwrap(),
        KeyOutcome::Continue
    );
    assert_eq!(app.action_query, "q");
}

#[test]
fn footer_copy_accounts_for_the_selection_enter_will_make() {
    let mut app = test_app();
    app.write_clipboard = |text| {
        assert_eq!(text, "use this line");
        Ok(ClipboardMethod::Osc52)
    };
    app.spawn_runner = |_, _, _| panic!("copy must not start a command");
    let area = super::harness::area_for(super::harness::TINY);
    app.push_output(OutputStream::Stdout, "use this line");
    for _ in 0..3 {
        handle_key(&mut app, key(KeyCode::Tab), area).unwrap();
    }
    assert_eq!(app.focus, FocusPane::Output);
    // Tab focuses without selecting. Copy itself selects the last renderable event.
    assert!(app.output_selected.is_none());
    assert!(footer_text(&app).contains("[Enter] Copy"));
    handle_key(&mut app, key(KeyCode::Enter), area).unwrap();
    assert!(
        app.copy_status
            .as_deref()
            .unwrap()
            .starts_with("copied 1 lines")
    );
    app.run_output.clear();
    assert!(!footer_text(&app).contains("[Enter]"));
}

#[test]
fn tiny_search_can_clear_the_other_filter_without_losing_apply_or_stop() {
    use super::harness::{TINY, area_for, render_text};
    let mut app = running_app();
    let area = area_for(TINY);
    for k in [
        char_key('b'),
        char_key('/'),
        char_key('r'),
        key(KeyCode::Enter),
        char_key('a'),
        char_key('/'),
        char_key('s'),
    ] {
        handle_key(&mut app, k, area).unwrap();
    }
    assert_eq!((&*app.run_query, &*app.action_query), ("r", "s"));
    assert!(footer_text(&app).contains("[Esc] Clear runs"));
    let frame = render_text(&mut app, TINY);
    let footer = frame.lines().last().unwrap();
    for hint in ["[Enter] Apply", "[Ctrl+C] Stop", "[Esc] Clear"] {
        assert!(footer.contains(hint), "{footer}");
    }
    handle_key(&mut app, key(KeyCode::Esc), area).unwrap();
    assert!(app.run_query.is_empty() && app.search_mode);
    handle_key(&mut app, key(KeyCode::Esc), area).unwrap();
    assert!(app.action_query.is_empty() && !app.search_mode);
}

#[test]
fn inspector_enter_only_offers_actions_that_can_start() {
    use super::harness::{TINY, area_for};
    let area = area_for(TINY);
    let mut app = test_app(); // a blocked config still permits show
    app.spawn_runner = |_, _, _| panic!("an empty, blocked or group row must not spawn");
    app.focus_inspector();
    assert!(footer_text(&app).contains("[Enter] Run show"));
    app.workflow_cursor = 0;
    assert!(footer_text(&app).contains("[Enter] Toggle group"));
    for expected in [true, false] {
        handle_key(&mut app, key(KeyCode::Enter), area).unwrap();
        assert_eq!(
            app.collapsed_groups.contains(&ActionGroup::Utilities),
            expected
        );
    }
    app.workflow_fixture = Some(vec![MonitorAction::Analyze]);
    app.workflow_cursor = 1;
    assert!(!footer_text(&app).contains("[Enter]"));
    handle_key(&mut app, key(KeyCode::Enter), area).unwrap();
    assert!(!app.last_run.as_ref().unwrap().ok);
    app.action_query = "no-such-workflow".into();
    assert!(!footer_text(&app).contains("[Enter]"));
    handle_key(&mut app, key(KeyCode::Enter), area).unwrap();
    assert!(!app.command_running());
}

#[test]
fn tiny_footer_tracks_real_search_help_and_cancel_input() {
    use super::harness::{TINY, area_for, render_text};
    let area = area_for(TINY);
    for pane in [FocusPane::Commands, FocusPane::Runs] {
        let mut app = running_app();
        let (cancel, rx) = mpsc::channel();
        app.active_run.as_mut().unwrap().cancel = cancel;
        app.focus = pane;
        handle_key(&mut app, char_key('/'), area).unwrap();
        let frame = render_text(&mut app, TINY);
        let footer = frame.lines().last().unwrap().trim_end();
        for hint in ["[Enter] Apply", "[Esc] Clear", "[Ctrl+C] Stop"] {
            assert!(footer.contains(hint), "{pane:?}: {footer}");
        }
        assert!(!footer.contains("Help") && !footer.contains("Quit"));
        for ch in ['?', 'q', '4'] {
            assert_eq!(
                handle_key(&mut app, char_key(ch), area).unwrap(),
                KeyOutcome::Continue
            );
        }
        assert_eq!(
            if pane == FocusPane::Runs {
                &app.run_query
            } else {
                &app.action_query
            },
            "?q4"
        );
        handle_key(&mut app, ctrl_key(KeyCode::Char('c')), area).unwrap();
        assert_eq!(rx.try_recv().unwrap(), CancelReason::CtrlC);
        handle_key(&mut app, key(KeyCode::Enter), area).unwrap();
        assert!(!app.search_mode && !app.run_search_mode);
        assert_eq!(
            if pane == FocusPane::Runs {
                &app.run_query
            } else {
                &app.action_query
            },
            "?q4"
        );
        handle_key(&mut app, key(KeyCode::Esc), area).unwrap();
        assert!(app.run_query.is_empty() && app.action_query.is_empty());
        handle_key(&mut app, char_key('?'), area).unwrap();
        handle_key(&mut app, ctrl_key(KeyCode::Char('c')), area).unwrap();
        assert!(!app.show_help);
        assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
    }
    let mut app = running_app();
    let (cancel, rx) = mpsc::channel();
    app.active_run.as_mut().unwrap().cancel = cancel;
    handle_key(&mut app, char_key('?'), area).unwrap();
    let frame = render_text(&mut app, TINY);
    let footer = frame.lines().last().unwrap();
    assert!(
        footer.contains("[Esc] Close") && footer.contains("[Ctrl+C] Stop"),
        "{footer}"
    );
    assert!(!footer.contains("Quit") && !footer.contains("Run "));
    handle_key(&mut app, ctrl_key(KeyCode::Char('c')), area).unwrap();
    assert!(!app.show_help);
    assert_eq!(rx.try_recv().unwrap(), CancelReason::CtrlC);
    handle_key(&mut app, ctrl_key(KeyCode::Char('c')), area).unwrap();
    assert!(matches!(rx.try_recv(), Err(TryRecvError::Empty)));
}

#[test]
fn numbered_focus_keys_move_between_panes() {
    let mut app = test_app();
    let area = Rect::new(0, 0, 120, 30);
    handle_key(&mut app, char_key('3'), area).unwrap();
    assert_eq!(app.focus, FocusPane::Output);
    handle_key(&mut app, char_key('1'), area).unwrap();
    assert_eq!(app.focus, FocusPane::Commands);
    handle_key(&mut app, char_key('2'), area).unwrap();
    assert_eq!(app.focus, FocusPane::Inspector);
    // Inspector-local numbers select tabs without leaving the pane.
    handle_key(&mut app, char_key('4'), area).unwrap();
    assert_eq!(app.focus, FocusPane::Inspector);
    assert_eq!(app.inspector_view, InspectorView::Artifacts);
    // S3: `5` selects the REPORTS tab the same way.
    handle_key(&mut app, char_key('5'), area).unwrap();
    assert_eq!(app.focus, FocusPane::Inspector);
    assert_eq!(app.inspector_view, InspectorView::Reports);
    // `i` cycles through REPORTS back to SUMMARY.
    app.cycle_inspector();
    assert_eq!(app.inspector_view, InspectorView::Summary);
    handle_key(&mut app, char_key('1'), area).unwrap();
    assert_eq!(app.inspector_view, InspectorView::Summary);
}

#[test]
fn typing_while_searching_extends_the_query_without_quitting() {
    let mut app = test_app();
    let area = Rect::new(0, 0, 120, 30);
    handle_key(&mut app, char_key('/'), area).unwrap();
    assert!(app.search_mode);
    handle_key(&mut app, char_key('q'), area).unwrap();
    assert!(app.search_mode);
    assert_eq!(app.action_query, "q");
    assert!(!app.quit_confirm_pending);
    handle_key(&mut app, char_key('?'), area).unwrap();
    assert_eq!(app.action_query, "q?");
    assert!(!app.show_help);
}

#[test]
fn escape_follows_close_then_leave_order() {
    let mut app = test_app();
    let area = Rect::new(0, 0, 120, 30);
    app.show_help = true;
    handle_key(&mut app, key(KeyCode::Esc), area).unwrap();
    assert!(!app.show_help);

    app.begin_action_search();
    app.push_action_query('x');
    handle_key(&mut app, key(KeyCode::Esc), area).unwrap();
    assert!(!app.search_mode);
    assert!(app.action_query.is_empty());

    app.push_output(OutputStream::Stdout, "one\ntwo");
    app.focus_output();
    app.enter_output_line_visual_mode();
    handle_key(&mut app, key(KeyCode::Esc), area).unwrap();
    assert_eq!(app.output_selection_anchor, None);
    assert_eq!(app.focus, FocusPane::Output);
    handle_key(&mut app, key(KeyCode::Esc), area).unwrap();
    assert_eq!(app.focus, FocusPane::Commands);
}

#[test]
fn quit_confirmation_arms_disarms_and_confirms() {
    let mut app = running_app();
    let area = Rect::new(0, 0, 120, 30);
    // First `q` arms and keeps the loop alive.
    assert_eq!(
        handle_key(&mut app, char_key('q'), area).unwrap(),
        KeyOutcome::Continue
    );
    assert!(app.quit_confirm_pending);
    assert!(app.command_running());
    // Any other key disarms.
    handle_key(&mut app, char_key('j'), area).unwrap();
    assert!(!app.quit_confirm_pending);
    assert!(app.command_running());
    // `Esc` also disarms.
    handle_key(&mut app, char_key('q'), area).unwrap();
    assert!(app.quit_confirm_pending);
    handle_key(&mut app, key(KeyCode::Esc), area).unwrap();
    assert!(!app.quit_confirm_pending);
}

#[test]
fn confirmed_quit_stops_the_child_and_drains_before_leaving() {
    let mut app = test_app();
    let (event_tx, event_rx) = mpsc::channel();
    let (cancel_tx, cancel_rx) = mpsc::channel();
    app.active_run = Some(ActiveRun {
        action: MonitorAction::Analyze,
        label: "Analyze all",
        started_at: Instant::now(),
        receiver: event_rx,
        cancel: cancel_tx,
        cancel_requested: false,
    });
    std::thread::spawn(move || {
        let reason = cancel_rx.recv().expect("quit must send a cancel reason");
        assert_eq!(reason, CancelReason::CtrlC);
        event_tx
            .send(RunEvent::Output(
                OutputStream::System,
                "stopping".to_string(),
            ))
            .unwrap();
        event_tx
            .send(RunEvent::Finished {
                ok: false,
                status: "stopped".to_string(),
            })
            .unwrap();
    });

    assert!(!app.request_quit());
    assert!(app.quit_confirm_pending);
    assert!(app.request_quit());
    assert!(app.active_run.is_none());
    let record = app.last_run.as_ref().expect("stop must record the run");
    assert_eq!(record.result, "stopped");
    assert!(
        app.run_output
            .iter()
            .any(|entry| entry.text.contains("stopping"))
    );
}

#[test]
fn stop_screen_cancels_and_drains_without_waiting_for_exit() {
    // FR-06: leaving a screen cancels its child and drains queued events; the
    // run stays active until the runner thread reports back.
    let mut app = test_app();
    let (event_tx, event_rx) = mpsc::channel();
    event_tx
        .send(RunEvent::Output(
            OutputStream::System,
            "queued line".to_string(),
        ))
        .unwrap();
    let (cancel_tx, cancel_rx) = mpsc::channel();
    app.active_run = Some(ActiveRun {
        action: MonitorAction::Analyze,
        label: "Analyze all",
        started_at: Instant::now(),
        receiver: event_rx,
        cancel: cancel_tx,
        cancel_requested: false,
    });

    app.stop_screen();

    assert!(app.active_run.as_ref().unwrap().cancel_requested);
    assert_eq!(cancel_rx.try_recv().unwrap(), CancelReason::CtrlC);
    assert!(
        app.run_output
            .iter()
            .any(|entry| entry.text.contains("queued line"))
    );
    assert!(app.active_run.is_some());

    event_tx
        .send(RunEvent::Finished {
            ok: true,
            status: "done".to_string(),
        })
        .unwrap();
    app.poll_command();
    assert!(app.active_run.is_none());
    assert_eq!(app.last_run.as_ref().unwrap().result, "done");
}

#[test]
fn stop_screen_is_a_noop_without_an_active_run() {
    let mut app = test_app();
    app.stop_screen();
    assert!(app.active_run.is_none());
}

#[test]
fn mouse_capture_defaults_on_and_opts_out_via_env() {
    let prior = std::env::var("PMOKE_MOUSE").ok();
    // SAFETY: no other test in this binary touches `PMOKE_MOUSE`, and the
    // value is restored before returning.
    unsafe {
        std::env::remove_var("PMOKE_MOUSE");
        assert!(mouse_capture_enabled());
        for off in ["off", "0", "no", "false", "disabled", "OFF"] {
            std::env::set_var("PMOKE_MOUSE", off);
            assert!(!mouse_capture_enabled(), "{off}");
        }
        std::env::set_var("PMOKE_MOUSE", "on");
        assert!(mouse_capture_enabled());
        match prior {
            Some(value) => std::env::set_var("PMOKE_MOUSE", value),
            None => std::env::remove_var("PMOKE_MOUSE"),
        }
    }
}

#[test]
fn help_notes_cover_mouse_keymap_prefs_and_reserved_colon() {
    let mut app = test_app();
    app.show_help = true;
    let rendered = rendered_help(120, 120, &mut app);
    assert!(rendered.contains("PMOKE_MOUSE"));
    assert!(rendered.contains("keymap.toml"));
    assert!(rendered.contains("history stays on [ ]"));
}
