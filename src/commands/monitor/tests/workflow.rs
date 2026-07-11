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
