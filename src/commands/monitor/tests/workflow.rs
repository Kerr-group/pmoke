use super::*;

#[test]
fn workflow_starts_on_safe_config_action() {
    let app = test_app();

    assert_eq!(
        app.selected_workflow_entry(),
        Some(WorkflowEntry::Action(MonitorAction::Show))
    );
}

#[test]
fn workflow_groups_can_be_collapsed_and_expanded() {
    let mut app = test_app();
    app.workflow_cursor = 0;
    let expanded_len = app.workflow_entries().len();

    assert!(app.toggle_selected_group());
    assert!(app.collapsed_groups.contains(&ActionGroup::Utilities));
    assert!(app.workflow_entries().len() < expanded_len);

    assert!(app.toggle_selected_group());
    assert!(!app.collapsed_groups.contains(&ActionGroup::Utilities));
    assert_eq!(app.workflow_entries().len(), expanded_len);
}

#[test]
fn workflow_search_selects_an_action_and_ignores_collapsed_groups() {
    let mut app = test_app();
    app.collapsed_groups.insert(ActionGroup::Analysis);

    app.begin_action_search();
    for ch in "kerr".chars() {
        app.push_action_query(ch);
    }

    assert_eq!(
        app.selected_workflow_entry(),
        Some(WorkflowEntry::Action(MonitorAction::Kerr))
    );
    assert!(
        app.workflow_entries()
            .contains(&WorkflowEntry::Action(MonitorAction::Kerr))
    );
}

#[test]
fn workflow_search_with_no_match_has_no_implicit_fallback_action() {
    let mut app = test_app();
    app.begin_action_search();
    for ch in "definitely-not-an-action".chars() {
        app.push_action_query(ch);
    }

    assert!(app.workflow_entries().is_empty());
    assert_eq!(app.selected_workflow_entry(), None);
}

#[test]
fn readiness_explains_config_and_raw_artifact_blocks() {
    let diagnostics = test_app();
    assert!(
        action_readiness(MonitorAction::Analyze, &diagnostics.load)
            .unwrap_err()
            .contains("configuration has errors")
    );

    let ready = ready_test_app(3);
    assert!(
        action_readiness(MonitorAction::RawVerify, &ready.load)
            .unwrap_err()
            .contains("RAW directory not found")
    );
}

#[test]
fn run_history_is_bounded_and_skips_the_live_duplicate() {
    let mut app = test_app();
    for index in 0..12 {
        app.run_output.clear();
        app.push_output(OutputStream::System, &format!("run {index}"));
        app.last_run = Some(RunRecord {
            action: MonitorAction::Show,
            label: "Config",
            elapsed: Duration::from_secs(index),
            result: format!("result {index}"),
            ok: true,
        });
        app.archive_last_run();
    }

    assert_eq!(app.run_history.len(), 10);
    assert_eq!(app.visible_output()[0].text, "run 11");

    app.show_previous_run();
    assert_eq!(app.history_position(), Some((9, 10)));
    assert_eq!(app.visible_output()[0].text, "run 10");

    app.show_next_run();
    assert_eq!(app.history_position(), None);
    assert_eq!(app.visible_output()[0].text, "run 11");
}

#[test]
fn a_single_completed_run_does_not_create_a_duplicate_history_view() {
    let mut app = test_app();
    app.push_output(OutputStream::System, "only run");
    app.last_run = Some(RunRecord {
        action: MonitorAction::Show,
        label: "Config",
        elapsed: Duration::ZERO,
        result: "done".to_string(),
        ok: true,
    });
    app.archive_last_run();

    app.show_previous_run();

    assert_eq!(app.history_position(), None);
}

#[test]
fn active_run_can_browse_the_immediately_previous_completed_run() {
    let mut app = test_app();
    app.push_output(OutputStream::System, "previous run");
    app.last_run = Some(RunRecord {
        action: MonitorAction::Show,
        label: "Config",
        elapsed: Duration::ZERO,
        result: "done".to_string(),
        ok: true,
    });
    app.archive_last_run();
    app.run_output.clear();
    app.push_output(OutputStream::System, "active run");
    let (_event_tx, event_rx) = mpsc::channel();
    let (cancel_tx, _cancel_rx) = mpsc::channel();
    app.active_run = Some(ActiveRun {
        action: MonitorAction::Analyze,
        label: "Analyze",
        started_at: Instant::now(),
        receiver: event_rx,
        cancel: cancel_tx,
        cancel_requested: false,
    });

    app.show_previous_run();
    assert_eq!(app.visible_output()[0].text, "previous run");

    app.show_next_run();
    assert_eq!(app.history_position(), None);
    assert_eq!(app.visible_output()[0].text, "active run");
}
