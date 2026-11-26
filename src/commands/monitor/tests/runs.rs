//! S2 run browser tests (Issue #276).
//!
//! - FR-01: count/depth budgets truncate instead of walking unboundedly,
//!   symlinks are never followed, and scans are read-only (no artifact
//!   writes from browser paths).
//! - FR-02: list + `/` filter + inspector preview reuse the S1 idioms.
//! - FR-03: `[`/`]` move the browser selection while Runs is focused and
//!   browse command history elsewhere; pinning returns to live output.
//! - FR-04: show / raw-verify / doctor carry panel notes while the re-exec
//!   launch model is unchanged (pinning never spawns).

use super::*;
use ratatui::backend::TestBackend;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

static RUN_TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn temp_root(name: &str) -> PathBuf {
    let id = RUN_TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let root =
        std::env::temp_dir().join(format!("pmoke_tui_s2_{name}_{}_{id}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn write_run_manifest(dir: &Path, status: &str, stage: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(
        dir.join("run.toml"),
        format!(
            "schema_version = 1\nstatus = \"{status}\"\nstage = \"{stage}\"\n\
             pmoke_version = \"test\"\nstarted_at = \"2026-01-01T00:00:00Z\"\n\
             updated_at = \"2026-02-01T00:00:00Z\"\n"
        ),
    )
    .unwrap();
}

fn finish_temp_root(root: &Path) {
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn refresh_keeps_the_startup_artifact_target_after_a_config_error() {
    use super::harness::{WIDE, area_for, assert_continue, ch, drive, key};
    let root = temp_root("workflow_root");
    let path = root.join("cfg/config.toml");
    let run_dir = root.join("runs/shot B");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::create_dir_all(run_dir.join("acquisition")).unwrap();
    write_run_manifest(&run_dir, "complete", "acquire");
    write_run_manifest(&root.join("runs/shot C"), "failed", "analyze");
    let valid = "version = 7\n[[sensors]]\nchannel = 1\nscale = { factor = 1.0 }\nlabel = 'field'\nunit = 'T'\n";
    std::fs::write(&path, valid).unwrap();
    let options = RunOptions {
        run_dir: Some(run_dir.clone()),
        force: true,
    };
    let config_path = path.to_str().unwrap().to_owned();
    let load = options.load(&config_path);
    let mut app = MonitorApp::new(config_path, load, options);
    let area = area_for(WIDE);
    assert!(action_runnable(MonitorAction::RawVerify, &app.load));
    app.refresh_runs();
    assert_continue(&drive(
        &mut app,
        area,
        &[ch('b'), ch('G'), key(KeyCode::Enter), ch('2'), ch('5')],
    ));
    assert!(render_text(120, 30, &mut app).contains("VIEW shot C (read-only)"));
    std::fs::write(&path, "invalid TOML [").unwrap();
    assert_continue(&drive(&mut app, area, &[ch('r')]));
    assert!(app.diagnostics().is_some());
    assert!(
        render_text(120, 30, &mut app).contains("VIEW shot C (read-only)"),
        "the selected report does not depend on parsing config"
    );
    std::fs::write(&path, valid).unwrap();
    assert_continue(&drive(&mut app, area, &[ch('r')]));
    assert_eq!(
        action_readiness(MonitorAction::RawVerify, &app.load),
        Ok(())
    );
    let (config, _) = app.ready_config().unwrap();
    assert_eq!(config.paths().run_dir, run_dir);
    assert!(config.force);
    assert_eq!(Path::new(&app.run_root), root.join("runs"));
    assert!(render_text(120, 30, &mut app).contains("VIEW shot C (read-only)"));
    finish_temp_root(&root);
}

/// Recursive `(relative path, bytes)` snapshot used to prove read-only
/// scans. Independent of `scan_run_dirs` so the test cannot pass trivially.
fn snapshot_tree(root: &Path) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let mut children: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .collect();
        children.sort();
        for child in children {
            if child.is_dir() && !child.is_symlink() {
                stack.push(child);
            } else if child.is_file() {
                let relative = child
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned();
                let bytes = std::fs::read(&child).unwrap();
                out.push((relative, bytes));
            }
        }
    }
    out.sort();
    out
}

fn fake_snapshot(label: &'static str) -> RunSnapshot {
    RunSnapshot {
        record: RunRecord {
            action: MonitorAction::Show,
            label,
            elapsed: Duration::ZERO,
            result: "ok".to_string(),
            ok: true,
        },
        output: Vec::new(),
    }
}

fn attach_test_runner(app: &mut MonitorApp) -> mpsc::Sender<RunEvent> {
    let (events, receiver) = mpsc::channel();
    let (cancel, _stop) = mpsc::channel();
    app.active_run = Some(ActiveRun {
        action: MonitorAction::Show,
        label: "live",
        started_at: Instant::now(),
        receiver,
        cancel,
        cancel_requested: false,
    });
    events
}

fn sample_entries() -> Vec<RunDirEntry> {
    vec![
        RunDirEntry {
            name: "run-beta".to_string(),
            path: "/runs/run-beta".into(),
            status: "failed".to_string(),
            stage: "analyze".to_string(),
            updated: "2026-02-01".to_string(),
            has_acquisition: true,
            has_analysis: false,
        },
        RunDirEntry {
            name: "run-alpha".to_string(),
            path: "/runs/run-alpha".into(),
            status: "complete".to_string(),
            stage: "moke".to_string(),
            updated: "2026-01-01".to_string(),
            has_acquisition: true,
            has_analysis: true,
        },
        RunDirEntry {
            name: "run-gamma".to_string(),
            path: "/runs/run-gamma".into(),
            status: "acquired".to_string(),
            stage: "fetch".to_string(),
            updated: "2026-03-01".to_string(),
            has_acquisition: true,
            has_analysis: false,
        },
    ]
}

fn runs_app_with_entries() -> MonitorApp {
    let mut app = test_app();
    app.run_entries = sample_entries();
    app.run_cursor = 0;
    app
}

fn render_text(width: u16, height: u16, app: &mut MonitorApp) -> String {
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
fn scan_lists_run_dirs_sorted_with_manifest_state() {
    let root = temp_root("sorted");
    write_run_manifest(&root.join("run-b"), "failed", "analyze");
    write_run_manifest(&root.join("run-a"), "complete", "moke");
    std::fs::create_dir_all(root.join("plain-dir")).unwrap();
    std::fs::create_dir_all(root.join("run-a").join("acquisition")).unwrap();

    let outcome = scan_run_dirs(&root);

    assert!(!outcome.truncated_dirs);
    assert!(!outcome.truncated_entries);
    assert_eq!(
        outcome
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect::<Vec<_>>(),
        vec!["run-a".to_string(), "run-b".to_string()]
    );
    assert_eq!(outcome.entries[0].status, "complete");
    assert_eq!(outcome.entries[0].stage, "moke");
    assert_eq!(outcome.entries[0].updated, "2026-02-01T00:00:00Z");
    assert!(outcome.entries[0].has_acquisition);
    assert!(!outcome.entries[0].has_analysis);
    assert_eq!(outcome.entries[1].status, "failed");

    finish_temp_root(&root);
}

#[test]
fn scan_entry_budget_truncates_instead_of_growing() {
    let root = temp_root("entry_budget");
    for index in 0..(RUN_SCAN_MAX_ENTRIES + 50) {
        write_run_manifest(&root.join(format!("run-{index:03}")), "complete", "moke");
    }

    let outcome = scan_run_dirs(&root);

    assert_eq!(outcome.entries.len(), RUN_SCAN_MAX_ENTRIES);
    assert!(outcome.truncated_entries);
    assert!(!outcome.truncated_dirs);

    finish_temp_root(&root);
}

#[test]
fn scan_depth_budget_skips_deep_runs() {
    let root = temp_root("depth");
    write_run_manifest(&root.join("run-shallow"), "complete", "moke");
    write_run_manifest(&root.join("mid").join("run-mid"), "complete", "moke");
    write_run_manifest(
        &root.join("mid").join("deep").join("run-deep"),
        "complete",
        "moke",
    );
    write_run_manifest(
        &root
            .join("mid")
            .join("deep")
            .join("deeper")
            .join("run-too-deep"),
        "complete",
        "moke",
    );

    let outcome = scan_run_dirs(&root);
    let names = outcome
        .entries
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<Vec<_>>();

    assert!(names.contains(&"run-shallow".to_string()));
    assert!(names.contains(&"run-mid".to_string()));
    assert!(names.contains(&"run-deep".to_string()));
    assert!(!names.contains(&"run-too-deep".to_string()));

    finish_temp_root(&root);
}

#[test]
fn scan_dir_budget_truncates_wide_trees() {
    let root = temp_root("dir_budget");
    for index in 0..(RUN_SCAN_MAX_DIRS + 8) {
        std::fs::create_dir_all(root.join(format!("plain-{index:03}"))).unwrap();
    }

    let outcome = scan_run_dirs(&root);

    assert!(outcome.truncated_dirs);
    assert!(outcome.dirs_visited <= RUN_SCAN_MAX_DIRS);
    assert!(outcome.entries.is_empty());

    finish_temp_root(&root);
}

#[test]
#[cfg(unix)]
fn scan_skips_symlinks_without_cycles() {
    let root = temp_root("symlink");
    write_run_manifest(&root.join("real-run"), "complete", "moke");
    std::os::unix::fs::symlink(&root, root.join("loop-back")).unwrap();

    let outcome = scan_run_dirs(&root);
    let names = outcome
        .entries
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["real-run".to_string()]);
    assert!(!outcome.truncated_dirs);

    finish_temp_root(&root);
}

#[test]
fn scan_is_read_only() {
    let root = temp_root("readonly");
    write_run_manifest(&root.join("run-a"), "complete", "moke");
    write_run_manifest(&root.join("nested").join("run-b"), "failed", "analyze");
    std::fs::create_dir_all(root.join("run-a").join("acquisition")).unwrap();
    let before = snapshot_tree(&root);

    let outcome = scan_run_dirs(&root);
    let after = snapshot_tree(&root);

    assert_eq!(outcome.entries.len(), 2);
    assert_eq!(before, after);

    finish_temp_root(&root);
}

#[test]
fn scan_missing_root_yields_empty_outcome() {
    let root = std::env::temp_dir().join(format!("pmoke_tui_s2_absent_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let outcome = scan_run_dirs(&root);

    assert!(outcome.entries.is_empty());
    assert!(!outcome.truncated_dirs);
    assert!(!outcome.truncated_entries);
}

#[test]
fn filter_matches_name_status_and_stage_case_insensitively() {
    let entries = sample_entries();

    assert_eq!(filter_run_entries(&entries, "").len(), 3);
    assert_eq!(filter_run_entries(&entries, "ALPHA"), vec![1], "name match");
    assert_eq!(
        filter_run_entries(&entries, "fail"),
        vec![0],
        "status match"
    );
    assert_eq!(filter_run_entries(&entries, "MOKE"), vec![1], "stage match");
    assert_eq!(
        filter_run_entries(&entries, "run-"),
        vec![0, 1, 2],
        "path-adjacent match"
    );
    assert!(filter_run_entries(&entries, "no-such-run").is_empty());
}

#[test]
fn run_browser_root_prefers_run_dir_parent_then_falls_back() {
    let app = ready_test_app(2);
    // Fixture config without artifact root resolves the run dir to the
    // config's directory; the fixture uses "config.toml", so the scan root
    // falls back to the startup directory.
    assert_eq!(
        run_browser_root("config.toml", &app.load, None, &app.current_dir),
        PathBuf::from(&app.current_dir)
    );

    let mut app = ready_test_app(2);
    if let ConfigLoad::Ready { config, .. } = &mut app.load {
        config.set_artifact_root(PathBuf::from("/tmp/pmoke-runs/run-001"));
    }
    assert_eq!(
        run_browser_root("config.toml", &app.load, None, &app.current_dir),
        PathBuf::from("/tmp/pmoke-runs")
    );

    let diagnostics = ConfigLoad::Diagnostics(ConfigDiagnostics {
        version: None,
        warnings: Vec::new(),
        diagnostics: Vec::new(),
        normalized: None,
    });
    assert_eq!(
        run_browser_root("some/dir/config.toml", &diagnostics, None, "."),
        PathBuf::from("some/dir")
    );
}

#[test]
fn run_search_filters_the_focused_list_and_esc_clears_it() {
    let mut app = runs_app_with_entries();
    app.focus_runs();
    app.begin_run_search();
    assert!(app.run_search_mode);
    assert!(!app.search_mode);

    app.push_run_query('b');
    app.push_run_query('e');
    assert_eq!(app.filtered_run_indices().len(), 1);

    app.escape();
    assert!(!app.run_search_mode);
    assert!(app.run_query.is_empty());
    assert_eq!(app.filtered_run_indices().len(), 3);
}

#[test]
fn esc_closes_help_before_clearing_the_runs_filter() {
    let mut app = runs_app_with_entries();
    app.focus_runs();
    app.begin_run_search();
    app.push_run_query('a');
    app.show_help = true;

    app.escape();
    assert!(!app.show_help);
    assert_eq!(app.run_query, "a");

    app.escape();
    assert!(app.run_query.is_empty());
}

#[test]
fn refreshing_reports_keeps_the_same_filtered_run_path() {
    use super::harness::{WIDE, area_for, assert_continue, ch, drive, key};
    let root = temp_root("refresh_target");
    let target = root.join("team-b/shared");
    write_run_manifest(&target, "complete", "moke");
    write_run_manifest(&root.join("zz"), "failed", "analyze");
    let mut app = test_app();
    app.config_path = root.join("absent.toml").to_string_lossy().into_owned();
    app.refresh();
    let area = area_for(WIDE);
    assert_continue(&drive(
        &mut app,
        area,
        &[
            ch('b'),
            ch('/'),
            ch('c'),
            key(KeyCode::Enter),
            key(KeyCode::Enter),
            ch('i'),
            ch('5'),
        ],
    ));
    assert_eq!(app.selected_run_entry().unwrap().path, target);
    write_run_manifest(&root.join("aaa"), "complete", "moke");
    write_run_manifest(&root.join("team-a/shared"), "complete", "moke");
    assert_continue(&drive(&mut app, area, &[ch('r')]));
    assert_eq!(app.selected_run_entry().unwrap().path, target);
    assert_eq!(app.run_query, "c");
    assert!(render_text(120, 30, &mut app).contains("VIEW shared (read-only)"));
    std::fs::remove_file(target.join("run.toml")).unwrap();
    assert_continue(&drive(&mut app, area, &[ch('r')]));
    assert!(app.run_cursor < app.filtered_run_indices().len());
    assert_ne!(app.selected_run_entry().unwrap().path, target);
    finish_temp_root(&root);
}

#[test]
#[cfg(unix)]
fn reports_and_refresh_never_use_a_lossy_display_name_as_a_path() {
    use super::harness::{WIDE, area_for, assert_continue, ch, drive};
    use std::os::unix::ffi::OsStrExt;
    let root = temp_root("lossy_alias");
    // Even on filesystems that reject non-UTF-8 names, a stale scan entry
    // must not be opened or re-selected as the real U+FFFD look-alike.
    let alias = root.join("run-�");
    let raw = root.join(std::ffi::OsStr::from_bytes(b"run-\xff"));
    write_run_manifest(&alias, "wrong-report", "other-stage");
    write_run_manifest(&root.join("z-survivor"), "complete", "done");
    let mut app = test_app();
    app.config_path = root.join("absent.toml").to_str().unwrap().to_owned();
    app.run_entries = vec![probe_run_entry(&alias), probe_run_entry(&raw)];
    let area = area_for(WIDE);
    assert_continue(&drive(
        &mut app,
        area,
        &[ch('b'), ch('G'), ch('2'), ch('5')],
    ));
    assert!(
        !reports_table_rows(&app)
            .iter()
            .flatten()
            .any(|s| s == "wrong-report")
    );
    assert_continue(&drive(&mut app, area, &[ch('r')]));
    assert_eq!(app.selected_run_entry().unwrap().name, "z-survivor");
    finish_temp_root(&root);
}

#[cfg(target_os = "linux")]
#[test]
fn refreshing_real_byte_named_siblings_preserves_their_distinct_reports() {
    use super::harness::{WIDE, area_for, assert_continue, ch, drive};
    use std::os::unix::ffi::OsStrExt;
    let root = temp_root("raw_names");
    for (name, stage) in [(b"run-\xfe", "first-stage"), (b"run-\xff", "second-stage")] {
        write_run_manifest(
            &root.join(std::ffi::OsStr::from_bytes(name)),
            "complete",
            stage,
        );
    }
    let mut app = test_app();
    app.config_path = root.join("absent.toml").to_str().unwrap().to_owned();
    let area = area_for(WIDE);
    assert_continue(&drive(
        &mut app,
        area,
        &[ch('r'), ch('b'), ch('G'), ch('2'), ch('5'), ch('r')],
    ));
    assert_eq!(app.run_cursor, 1);
    // One Stage comes from the list snapshot; the other is reread from disk.
    assert_eq!(
        reports_table_rows(&app)
            .iter()
            .filter(|r| r.as_slice() == ["Stage", "second-stage"])
            .count(),
        2
    );
    finish_temp_root(&root);
}

#[test]
fn run_cursor_clamps_when_the_filter_shrinks() {
    let mut app = runs_app_with_entries();
    app.run_cursor = 2;
    app.run_query = "alpha".to_string();
    app.clamp_run_cursor();
    assert_eq!(app.run_cursor, 0);
    assert_eq!(
        app.selected_run_entry().map(|entry| entry.name.as_str()),
        Some("run-alpha")
    );
}

#[test]
fn history_keys_follow_the_runs_focus() {
    let mut app = runs_app_with_entries();
    app.run_history.push_back(fake_snapshot("first"));
    app.run_history.push_back(fake_snapshot("second"));

    // Runs focused: `[`/`]` move the browser selection, history untouched.
    app.focus_runs();
    app.run_cursor = 1;
    app.show_previous_run();
    assert_eq!(app.run_cursor, 0);
    assert_eq!(app.history_view, None);
    app.show_next_run();
    assert_eq!(app.run_cursor, 1);
    assert_eq!(app.history_view, None);

    // Workflow focused: the same keys browse command history as in S1.
    app.focus_commands();
    app.show_previous_run();
    assert_eq!(app.history_view, Some(0));
    app.show_next_run();
    assert_eq!(app.history_view, None);
}

#[test]
fn end_goes_to_the_bottom_of_displayed_history_without_leaving_it() {
    use super::harness::{WIDE, area_for, assert_continue, ch, drive, key, render_text};
    let mut app = test_app();
    for _ in 0..32 {
        app.push_output(OutputStream::Stdout, "archived event");
    }
    let mut archived = fake_snapshot("first");
    archived.output = std::mem::take(&mut app.run_output);
    app.run_history.extend([archived, fake_snapshot("latest")]);
    let area = area_for(WIDE);
    assert_continue(&drive(
        &mut app,
        area,
        &[ch('o'), ch('['), key(KeyCode::PageUp)],
    ));
    assert_eq!(app.history_view, Some(0));
    assert!(app.run_output_scroll > 0);
    let frame = render_text(&mut app, WIDE);
    assert!(frame.lines().last().unwrap().contains("[End] Bottom"));
    assert_continue(&drive(&mut app, area, &[key(KeyCode::End)]));
    assert_eq!(app.run_output_scroll, 0);
    assert_eq!(app.history_view, Some(0));
}

#[test]
fn live_progress_does_not_move_a_history_selection() {
    use super::harness::{WIDE, area_for, assert_continue, ch, drive};
    let mut app = test_app();
    let mut older = fake_snapshot("archive");
    older.output = ["a", "b", "c"]
        .map(|s| LogEntry::new(OutputStream::Stdout, s))
        .into();
    app.run_history.push_back(older);
    let events = attach_test_runner(&mut app);
    let progress = |sequence, fields| UiEvent {
        event_type: "event".into(),
        sequence,
        elapsed_ms: 0,
        level: EventLevel::Info,
        kind: EventKind::Progress,
        stage: None,
        message: "progress".into(),
        fields,
        progress_id: Some("task".into()),
        progress_current: None,
        progress_total: None,
        duration_ms: None,
    };
    events
        .send(RunEvent::Structured(progress(1, Vec::new())))
        .unwrap();
    app.poll_command();
    assert_continue(&drive(
        &mut app,
        area_for(WIDE),
        &[ch('o'), ch('['), ch('g'), ch('J')],
    ));
    assert_eq!(app.selected_output_text().as_deref(), Some("a\nb"));
    events
        .send(RunEvent::Structured(progress(
            2,
            vec![("count".into(), "2".into())],
        )))
        .unwrap();
    app.poll_command();
    assert_eq!(app.selected_output_text().as_deref(), Some("a\nb"));
    assert_eq!(
        app.run_output.len(),
        2,
        "the live progress update must still be applied"
    );
}

#[test]
fn finishing_a_child_keeps_the_viewed_history_record_until_eviction() {
    use super::harness::{WIDE, area_for, assert_continue, ch, drive};
    let root = temp_root("history_finish");
    let area = area_for(WIDE);
    for (count, selected) in [(1, 0), (10, 3), (10, 0)] {
        let mut app = test_app();
        app.config_path = root.join("absent.toml").to_string_lossy().into_owned();
        for i in 0..count {
            let mut snapshot = fake_snapshot("old");
            snapshot
                .output
                .push(LogEntry::new(OutputStream::System, format!("old {i}")));
            app.run_history.push_back(snapshot);
        }
        let events = attach_test_runner(&mut app);
        assert_continue(&drive(&mut app, area, &[ch('o')]));
        assert_continue(&drive(&mut app, area, &vec![ch('['); count - selected]));
        assert_continue(&drive(&mut app, area, &[ch('v')]));
        let text = format!("old {selected}");
        assert_eq!(app.selected_output_text().as_deref(), Some(text.as_str()));
        assert!(app.output_selection_anchor.is_some());
        events
            .send(RunEvent::Finished {
                ok: true,
                status: "live done".into(),
            })
            .unwrap();
        app.poll_command();
        assert!(!app.command_running());
        if count == 10 && selected == 0 {
            assert_eq!(app.history_view, None);
            assert_eq!(app.output_selection_anchor, None);
            assert!(app.selected_output_text().unwrap().contains("live done"));
        } else {
            assert!(app.history_view.is_some());
            assert_eq!(app.selected_output_text().as_deref(), Some(text.as_str()));
            assert!(app.output_selection_anchor.is_some());
            assert_continue(&drive(&mut app, area, &vec![ch(']'); count - selected]));
            assert_eq!(app.history_view, None);
            assert!(app.selected_output_text().unwrap().contains("live done"));
        }
    }
    finish_temp_root(&root);
}

#[test]
fn activity_status_belongs_to_the_displayed_history_not_the_live_child() {
    use super::harness::{WIDE, area_for, assert_continue, ch, drive};
    let mut app = test_app();
    let _events = attach_test_runner(&mut app);
    let mut failed = fake_snapshot("old failure");
    failed.record.ok = false;
    app.run_history
        .extend([fake_snapshot("old success"), failed]);
    for (key, label, color) in [
        ('[', "FAILED", Color::Red),
        ('[', "DONE", Color::Green),
        (']', "FAILED", Color::Red),
        (']', "RUNNING", Color::Yellow),
    ] {
        assert_continue(&drive(&mut app, area_for(WIDE), &[ch(key)]));
        let frame = render_text(120, 30, &mut app);
        let row = frame
            .lines()
            .find(|line| line.contains(" events "))
            .unwrap();
        assert!(row.contains(label), "{row}");
        assert_eq!(displayed_run_status(&app).1, color);
        assert!(app.command_running());
    }
}

#[test]
fn pin_selected_run_returns_to_live_without_spawning() {
    let mut app = runs_app_with_entries();
    app.spawn_runner = |_, _, _| panic!("preview must not start a command");
    let mut older = fake_snapshot("first");
    older.output = (0..32)
        .map(|i| LogEntry::new(OutputStream::Stdout, format!("old {i}")))
        .collect();
    app.push_output(OutputStream::Stdout, "live only");
    app.run_history.push_back(older);
    app.run_history.push_back(fake_snapshot("second"));
    app.show_previous_run();
    app.enter_output_line_visual_mode();
    app.scroll_output_up(4);

    app.focus_runs();
    let frame = super::harness::render_text(&mut app, super::harness::TINY);
    assert!(frame.lines().last().unwrap().contains("[Enter] Preview"));
    let area = super::harness::area_for(super::harness::TINY);
    super::harness::assert_continue(&super::harness::drive(
        &mut app,
        area,
        &[super::harness::key(KeyCode::Enter)],
    ));

    assert_eq!(app.focus, FocusPane::Runs);
    assert_eq!(app.history_view, None);
    assert!(app.active_run.is_none());
    assert_eq!(app.selected_output_text().as_deref(), Some("live only"));
    assert_eq!(app.output_selection_anchor, None);
    assert_eq!(app.run_output_scroll, 0);
}

#[test]
fn readout_panel_notes_cover_only_the_utility_panels() {
    for action in [
        MonitorAction::Show,
        MonitorAction::Doctor,
        MonitorAction::RawVerify,
    ] {
        assert!(
            readout_panel_note(action).is_some(),
            "{action:?} needs a panel note"
        );
    }
    for action in [
        MonitorAction::Analyze,
        MonitorAction::Reference,
        MonitorAction::Moke,
    ] {
        assert_eq!(readout_panel_note(action), None);
    }
}

#[test]
fn preview_rows_cover_the_inspector_fields() {
    let rows = run_preview_rows(&sample_entries()[1]);

    assert!(
        rows.iter()
            .any(|row| row == &vec!["Run".to_string(), "run-alpha".to_string()])
    );
    assert!(
        rows.iter()
            .any(|row| row == &vec!["Status".to_string(), "complete".to_string()])
    );
    assert!(
        rows.iter()
            .any(|row| row == &vec!["Stage".to_string(), "moke".to_string()])
    );
}

#[test]
fn dashboard_renders_runs_browser_alongside_workflow() {
    let mut app = runs_app_with_entries();
    let rendered = render_text(120, 30, &mut app);

    assert!(rendered.contains("RUNS"));
    assert!(rendered.contains("WORKFLOW"));
    assert!(rendered.contains("run-alpha"));
    assert!(!rendered.contains("INSPECTOR · RUN"));
}

#[test]
fn truncated_scan_marks_the_runs_title() {
    let mut app = runs_app_with_entries();
    app.run_truncated = true;
    let rendered = render_text(120, 30, &mut app);

    assert!(rendered.contains("TRUNC"));
}

#[test]
fn inspector_previews_the_selected_run() {
    let mut app = runs_app_with_entries();
    app.focus_runs();
    app.run_cursor = 1;
    let rendered = render_text(120, 30, &mut app);

    assert!(rendered.contains("INSPECTOR · RUN"));
    assert!(rendered.contains("run-alpha"));
    assert!(rendered.contains("complete"));
    assert!(rendered.contains("moke"));
}

#[test]
fn clicking_the_runs_section_selects_that_row() {
    let mut app = runs_app_with_entries();
    let area = Rect::new(0, 0, 60, 12);
    let (runs_area, _) = runs_layout(area);
    let inner = bordered_inner(runs_area);

    // With three entries and the cursor at 0, the third rendered row maps to
    // filtered position 2.
    app.run_cursor = 0;
    select_run_at(&mut app, runs_area, inner.x + 1, inner.y + 2);

    assert_eq!(app.focus, FocusPane::Runs);
    assert_eq!(app.run_cursor, 2);
}
