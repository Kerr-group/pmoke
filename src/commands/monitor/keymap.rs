//! S1 two-layer keybinding registry (Issue #275, FR-01/FR-02).
//!
//! All key handling in the monitor event loop goes through [`REGISTRY`]:
//! `global` bindings plus `per-pane` bindings (`Commands` / `Inspector` /
//! `Output`), with two modal layers (`Searching`, `HelpOpen`). `?` help and
//! the per-focus footer render from this same table, so the help text cannot
//! drift from the implementation. See the `KeyBinding{action, keys, contexts,
//! description}` shape required by #275.
//!
//! Dispatch rule (first match in [`REGISTRY`] order):
//! - a binding matches when any of its [`KeySpec`]s matches the key event
//!   (code equal, required modifiers subset of the event modifiers) and any
//!   of its [`KeyContext`]s is active;
//! - while the help overlay is open only `HelpOpen` bindings are active
//!   (everything else is swallowed by the modal);
//! - otherwise `Global` is always active, plus the focused pane context,
//!   plus `Searching` while the workflow filter is being typed.
//!
//! Within one key, pane/modal-specific entries are registered before their
//! global fallback (e.g. `Enter` commits the search, copies in the activity
//! pane, and runs the selected action everywhere else).
//!
//! Deliberately NOT bindings (documented here so the drift test can pin them):
//! - workflow filter text entry: any `Char` while searching that matches no
//!   registry entry is appended to the query (vim insert-mode analogue);
//! - `:` is reserved for a future command palette (FR-08). Adopting it
//!   requires relocating the `[`/`]` run-history keys as a set, so `:` stays
//!   unbound and a test pins that;
//! - S2 run browser (Issue #276): `4`/`b` focus the Runs section, `[/]`
//!   move the run selection while it is focused (history elsewhere), `/`
//!   filters the focused list, and `Enter` pins the run preview. All of
//!   these reuse the registry/filter/footer primitives below.
//!
//! - S3 calibration/analysis inspectors (Issue #277): `5` selects the
//!   REPORTS inspector tab while the inspector is focused (`i` cycles
//!   through it like every other tab). The tab renders the S2-selected
//!   run's per-stage manifest status plus standalone report summaries;
//!   scrolling reuses the existing inspector scroll actions, so no new
//!   `TuiAction` was needed.
//!
//! FR-07 keymap-prefs decision (decision only; loaders are an explicit
//! non-goal, "keymap-format future extensions"):
//! - location: `$XDG_CONFIG_HOME/pmoke/keymap.toml`, falling back to
//!   `~/.config/pmoke/keymap.toml` (TOML like the rest of the repo; resolved
//!   without new dependencies via `$XDG_CONFIG_HOME`/`$HOME`);
//! - format (sketch, not implemented): `[[binding]] action = "<TuiAction>`
//!   `keys = ["q", "Ctrl+C"]`, applied as an overlay consulted before
//!   [`REGISTRY`];
//! - the `pmoke` configuration schema (v6) is never consulted for TUI
//!   keymap prefs.
//!
//! FR-05 mouse opt-out: `PMOKE_MOUSE=off` (also `0`/`no`/`false`/`disabled`)
//! disables mouse capture; the default (unset/anything else) keeps the
//! current capture behavior unchanged.

use super::*;

/// Layer a binding applies to: shared (`Global`) or exclusive to one UI
/// state. Mirrors the k9s shared-vs-exclusive `KeyActions` split (survey P9).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeyContext {
    Global,
    Commands,
    /// S2 run browser section of the workflow panel.
    Runs,
    Inspector,
    Output,
    Searching,
    HelpOpen,
}

/// One physical key chord. `modifiers` is a required subset: an event matches
/// when its code is equal and its modifiers contain at least these (so `Up`
/// also matches `Shift+Up`, exactly like the previous flat `match`, which
/// only inspected `code` except where it explicitly checked modifiers).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct KeySpec {
    pub(super) code: KeyCode,
    pub(super) modifiers: KeyModifiers,
}

impl KeySpec {
    const fn plain(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::NONE,
        }
    }

    const fn ctrl(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: KeyModifiers::CONTROL,
        }
    }

    pub(super) fn label(self) -> String {
        let mut out = String::new();
        if self.modifiers.contains(KeyModifiers::CONTROL) {
            out.push_str("Ctrl+");
        }
        if self.modifiers.contains(KeyModifiers::ALT) {
            out.push_str("Alt+");
        }
        if self.modifiers.contains(KeyModifiers::SHIFT) {
            out.push_str("Shift+");
        }
        out.push_str(&key_code_label(self.code));
        out
    }
}

fn key_code_label(code: KeyCode) -> String {
    match code {
        KeyCode::Char(ch) => ch.to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::BackTab => "Shift-Tab".to_string(),
        KeyCode::Backspace => "Backspace".to_string(),
        KeyCode::Delete => "Delete".to_string(),
        KeyCode::Insert => "Insert".to_string(),
        KeyCode::Up => "Up".to_string(),
        KeyCode::Down => "Down".to_string(),
        KeyCode::Left => "Left".to_string(),
        KeyCode::Right => "Right".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PageUp".to_string(),
        KeyCode::PageDown => "PageDown".to_string(),
        _ => "?".to_string(),
    }
}

/// Every key-triggered behavior. Pure dispatch target: [`resolve_key`] maps a
/// key event to one of these with no side effects, and the event loop applies
/// it separately, so tests can assert the mapping without spawning children.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TuiAction {
    ToggleHelp,
    CloseHelp,
    Interrupt,
    Escape,
    BeginSearch,
    SearchBackspace,
    SearchCommit,
    PageUpScroll,
    PageDownScroll,
    HalfUpScroll,
    HalfDownScroll,
    FollowOutput,
    RequestQuit,
    Refresh,
    HistoryPrev,
    HistoryNext,
    FocusWorkflow,
    FocusActivity,
    CycleInspectorTabs,
    FocusMessages,
    FocusFiles,
    FocusStatus,
    FocusPane(FocusPane),
    InspectorTab(InspectorView),
    /// S2 run browser (FR-02/FR-03). Pane-local to `Runs`; handled by the
    /// same apply path as the workflow list (no S1 redesign).
    FocusRuns,
    RunsPrev,
    RunsNext,
    RunsFirst,
    RunsLast,
    /// Preview the selected run in the inspector without spawning anything.
    PinRun,
    CopySelection,
    EnterVisualMode,
    RunSelected,
    OutputUpExtend,
    OutputDownExtend,
    OutputPrev,
    OutputNext,
    OutputFirst,
    OutputLast,
    InspectorUp,
    InspectorDown,
    CommandsPrev,
    CommandsNext,
    CommandsFirst,
    CommandsLast,
    FocusPrev,
    FocusNext,
}

/// One registry row: what to do, which keys trigger it, where it applies,
/// and the help description rendered verbatim in the `?` overlay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct KeyBinding {
    pub(super) action: TuiAction,
    pub(super) keys: &'static [KeySpec],
    pub(super) contexts: &'static [KeyContext],
    pub(super) description: &'static str,
}

impl KeyBinding {
    pub(super) fn label(&self) -> String {
        self.keys
            .iter()
            .map(|spec| spec.label())
            .collect::<Vec<_>>()
            .join("/")
    }
}

/// The single source of truth for key handling, `?` help, and footer labels.
/// Order matters: within one key, modal/pane-specific entries precede their
/// global fallback (first match wins in [`resolve_key`]).
pub(super) const REGISTRY: &[KeyBinding] = &[
    KeyBinding {
        action: TuiAction::Interrupt,
        keys: &[KeySpec::ctrl(KeyCode::Char('c'))],
        contexts: &[KeyContext::Global, KeyContext::HelpOpen],
        description: "interrupt command or cancel activity selection",
    },
    KeyBinding {
        action: TuiAction::ToggleHelp,
        keys: &[KeySpec::plain(KeyCode::Char('?'))],
        contexts: &[KeyContext::Global, KeyContext::HelpOpen],
        description: "toggle this help",
    },
    KeyBinding {
        action: TuiAction::Escape,
        keys: &[KeySpec::plain(KeyCode::Esc)],
        contexts: &[
            KeyContext::Global,
            KeyContext::Searching,
            KeyContext::HelpOpen,
        ],
        description: "close dialog, clear search/selection, then leave panel",
    },
    KeyBinding {
        action: TuiAction::CloseHelp,
        keys: &[KeySpec::plain(KeyCode::Char('q'))],
        contexts: &[KeyContext::HelpOpen],
        description: "close this help",
    },
    KeyBinding {
        action: TuiAction::BeginSearch,
        keys: &[KeySpec::plain(KeyCode::Char('/'))],
        contexts: &[KeyContext::Global],
        description: "search focused list (workflow / runs)",
    },
    KeyBinding {
        action: TuiAction::SearchBackspace,
        keys: &[KeySpec::plain(KeyCode::Backspace)],
        contexts: &[KeyContext::Searching],
        description: "erase one search character",
    },
    KeyBinding {
        action: TuiAction::SearchCommit,
        keys: &[KeySpec::plain(KeyCode::Enter)],
        contexts: &[KeyContext::Searching],
        description: "keep the search filter and leave typing mode",
    },
    KeyBinding {
        action: TuiAction::PageUpScroll,
        keys: &[KeySpec::plain(KeyCode::PageUp)],
        contexts: &[KeyContext::Output, KeyContext::Inspector],
        description: "scroll activity/inspector up one page",
    },
    KeyBinding {
        action: TuiAction::PageDownScroll,
        keys: &[KeySpec::plain(KeyCode::PageDown)],
        contexts: &[KeyContext::Output, KeyContext::Inspector],
        description: "scroll activity/inspector down one page",
    },
    KeyBinding {
        action: TuiAction::HalfUpScroll,
        keys: &[KeySpec::ctrl(KeyCode::Char('u'))],
        contexts: &[KeyContext::Output, KeyContext::Inspector],
        description: "scroll activity/inspector up half a page",
    },
    KeyBinding {
        action: TuiAction::HalfDownScroll,
        keys: &[KeySpec::ctrl(KeyCode::Char('d'))],
        contexts: &[KeyContext::Output, KeyContext::Inspector],
        description: "scroll activity/inspector down half a page",
    },
    KeyBinding {
        action: TuiAction::FollowOutput,
        keys: &[KeySpec::plain(KeyCode::End)],
        contexts: &[KeyContext::Global],
        description: "follow live activity",
    },
    KeyBinding {
        action: TuiAction::RequestQuit,
        keys: &[KeySpec::plain(KeyCode::Char('q'))],
        contexts: &[KeyContext::Global],
        description: "quit (press again to stop a running command and quit)",
    },
    KeyBinding {
        action: TuiAction::Refresh,
        keys: &[KeySpec::plain(KeyCode::Char('r'))],
        contexts: &[KeyContext::Global],
        description: "refresh config and files",
    },
    KeyBinding {
        action: TuiAction::HistoryPrev,
        keys: &[KeySpec::plain(KeyCode::Char('['))],
        contexts: &[KeyContext::Global],
        description: "older runs (selection in runs)",
    },
    KeyBinding {
        action: TuiAction::HistoryNext,
        keys: &[KeySpec::plain(KeyCode::Char(']'))],
        contexts: &[KeyContext::Global],
        description: "live activity (selection in runs)",
    },
    KeyBinding {
        action: TuiAction::FocusWorkflow,
        keys: &[KeySpec::plain(KeyCode::Char('a'))],
        contexts: &[KeyContext::Global],
        description: "focus workflow panel",
    },
    KeyBinding {
        action: TuiAction::FocusActivity,
        keys: &[KeySpec::plain(KeyCode::Char('o'))],
        contexts: &[KeyContext::Global],
        description: "focus activity panel and select events",
    },
    KeyBinding {
        action: TuiAction::CycleInspectorTabs,
        keys: &[KeySpec::plain(KeyCode::Char('i'))],
        contexts: &[KeyContext::Global],
        description: "cycle inspector tabs",
    },
    KeyBinding {
        action: TuiAction::FocusMessages,
        keys: &[KeySpec::plain(KeyCode::Char('m'))],
        contexts: &[KeyContext::Global],
        description: "focus inspector messages tab",
    },
    KeyBinding {
        action: TuiAction::FocusFiles,
        keys: &[KeySpec::plain(KeyCode::Char('f'))],
        contexts: &[KeyContext::Global],
        description: "focus inspector files tab",
    },
    KeyBinding {
        action: TuiAction::FocusStatus,
        keys: &[KeySpec::plain(KeyCode::Char('s'))],
        contexts: &[KeyContext::Global],
        description: "focus inspector panel",
    },
    KeyBinding {
        action: TuiAction::InspectorTab(InspectorView::Summary),
        keys: &[KeySpec::plain(KeyCode::Char('1'))],
        contexts: &[KeyContext::Inspector],
        description: "inspector summary tab",
    },
    KeyBinding {
        action: TuiAction::InspectorTab(InspectorView::Config),
        keys: &[KeySpec::plain(KeyCode::Char('2'))],
        contexts: &[KeyContext::Inspector],
        description: "inspector config tab",
    },
    KeyBinding {
        action: TuiAction::InspectorTab(InspectorView::Diagnostics),
        keys: &[KeySpec::plain(KeyCode::Char('3'))],
        contexts: &[KeyContext::Inspector],
        description: "inspector messages tab",
    },
    KeyBinding {
        action: TuiAction::InspectorTab(InspectorView::Artifacts),
        keys: &[KeySpec::plain(KeyCode::Char('4'))],
        contexts: &[KeyContext::Inspector],
        description: "inspector files tab",
    },
    KeyBinding {
        action: TuiAction::InspectorTab(InspectorView::Reports),
        keys: &[KeySpec::plain(KeyCode::Char('5'))],
        contexts: &[KeyContext::Inspector],
        description: "inspector reports tab",
    },
    KeyBinding {
        action: TuiAction::FocusPane(FocusPane::Commands),
        keys: &[KeySpec::plain(KeyCode::Char('1'))],
        contexts: &[KeyContext::Global],
        description: "focus workflow panel",
    },
    KeyBinding {
        action: TuiAction::FocusPane(FocusPane::Inspector),
        keys: &[KeySpec::plain(KeyCode::Char('2'))],
        contexts: &[KeyContext::Global],
        description: "focus inspector panel",
    },
    KeyBinding {
        action: TuiAction::FocusPane(FocusPane::Output),
        keys: &[KeySpec::plain(KeyCode::Char('3'))],
        contexts: &[KeyContext::Global],
        description: "focus activity panel",
    },
    KeyBinding {
        action: TuiAction::FocusRuns,
        keys: &[
            KeySpec::plain(KeyCode::Char('4')),
            KeySpec::plain(KeyCode::Char('b')),
        ],
        contexts: &[KeyContext::Global],
        description: "focus runs browser panel",
    },
    KeyBinding {
        action: TuiAction::CopySelection,
        keys: &[KeySpec::plain(KeyCode::Char('y'))],
        contexts: &[KeyContext::Global],
        description: "copy selected activity events",
    },
    KeyBinding {
        action: TuiAction::EnterVisualMode,
        keys: &[
            KeySpec::plain(KeyCode::Char('v')),
            KeySpec::plain(KeyCode::Char('V')),
        ],
        contexts: &[KeyContext::Output],
        description: "visual-line select activity",
    },
    KeyBinding {
        action: TuiAction::CopySelection,
        keys: &[KeySpec::plain(KeyCode::Enter)],
        contexts: &[KeyContext::Output],
        description: "copy selected activity events",
    },
    KeyBinding {
        action: TuiAction::PinRun,
        keys: &[KeySpec::plain(KeyCode::Enter)],
        contexts: &[KeyContext::Runs],
        description: "preview selected run in inspector (read-only)",
    },
    KeyBinding {
        action: TuiAction::RunSelected,
        keys: &[KeySpec::plain(KeyCode::Enter)],
        contexts: &[KeyContext::Global],
        description: "run selected command",
    },
    KeyBinding {
        action: TuiAction::OutputUpExtend,
        keys: &[KeySpec::plain(KeyCode::Char('K'))],
        contexts: &[KeyContext::Output],
        description: "extend activity selection upward",
    },
    KeyBinding {
        action: TuiAction::OutputDownExtend,
        keys: &[KeySpec::plain(KeyCode::Char('J'))],
        contexts: &[KeyContext::Output],
        description: "extend activity selection downward",
    },
    KeyBinding {
        action: TuiAction::OutputPrev,
        keys: &[
            KeySpec::plain(KeyCode::Up),
            KeySpec::plain(KeyCode::Char('k')),
        ],
        contexts: &[KeyContext::Output],
        description: "move activity selection up",
    },
    KeyBinding {
        action: TuiAction::OutputNext,
        keys: &[
            KeySpec::plain(KeyCode::Down),
            KeySpec::plain(KeyCode::Char('j')),
        ],
        contexts: &[KeyContext::Output],
        description: "move activity selection down",
    },
    KeyBinding {
        action: TuiAction::OutputFirst,
        keys: &[KeySpec::plain(KeyCode::Char('g'))],
        contexts: &[KeyContext::Output],
        description: "move activity selection to the first event",
    },
    KeyBinding {
        action: TuiAction::OutputLast,
        keys: &[KeySpec::plain(KeyCode::Char('G'))],
        contexts: &[KeyContext::Output],
        description: "move activity selection to the last event",
    },
    // S2 run browser navigation (FR-02/FR-03). Pane-specific rows precede
    // the global workflow fallbacks so first-match resolves to the runs
    // list while it is focused (see the determinism test).
    KeyBinding {
        action: TuiAction::RunsPrev,
        keys: &[
            KeySpec::plain(KeyCode::Up),
            KeySpec::plain(KeyCode::Char('k')),
        ],
        contexts: &[KeyContext::Runs],
        description: "move run selection up",
    },
    KeyBinding {
        action: TuiAction::RunsNext,
        keys: &[
            KeySpec::plain(KeyCode::Down),
            KeySpec::plain(KeyCode::Char('j')),
        ],
        contexts: &[KeyContext::Runs],
        description: "move run selection down",
    },
    KeyBinding {
        action: TuiAction::RunsFirst,
        keys: &[KeySpec::plain(KeyCode::Char('g'))],
        contexts: &[KeyContext::Runs],
        description: "move run selection to the first run",
    },
    KeyBinding {
        action: TuiAction::RunsLast,
        keys: &[KeySpec::plain(KeyCode::Char('G'))],
        contexts: &[KeyContext::Runs],
        description: "move run selection to the last run",
    },
    KeyBinding {
        action: TuiAction::InspectorUp,
        keys: &[
            KeySpec::plain(KeyCode::Up),
            KeySpec::plain(KeyCode::Char('k')),
        ],
        contexts: &[KeyContext::Inspector],
        description: "scroll inspector up",
    },
    KeyBinding {
        action: TuiAction::InspectorDown,
        keys: &[
            KeySpec::plain(KeyCode::Down),
            KeySpec::plain(KeyCode::Char('j')),
        ],
        contexts: &[KeyContext::Inspector],
        description: "scroll inspector down",
    },
    KeyBinding {
        action: TuiAction::CommandsPrev,
        keys: &[
            KeySpec::plain(KeyCode::Up),
            KeySpec::plain(KeyCode::Char('k')),
        ],
        contexts: &[KeyContext::Global],
        description: "move command selection up",
    },
    KeyBinding {
        action: TuiAction::CommandsNext,
        keys: &[
            KeySpec::plain(KeyCode::Down),
            KeySpec::plain(KeyCode::Char('j')),
        ],
        contexts: &[KeyContext::Global],
        description: "move command selection down",
    },
    KeyBinding {
        action: TuiAction::CommandsFirst,
        keys: &[KeySpec::plain(KeyCode::Char('g'))],
        contexts: &[KeyContext::Global],
        description: "move command selection to the first row",
    },
    KeyBinding {
        action: TuiAction::CommandsLast,
        keys: &[KeySpec::plain(KeyCode::Char('G'))],
        contexts: &[KeyContext::Global],
        description: "move command selection to the last row",
    },
    KeyBinding {
        action: TuiAction::FocusPrev,
        keys: &[
            KeySpec::plain(KeyCode::Left),
            KeySpec::plain(KeyCode::BackTab),
            KeySpec::plain(KeyCode::Char('h')),
        ],
        contexts: &[KeyContext::Global],
        description: "focus previous panel",
    },
    KeyBinding {
        action: TuiAction::FocusNext,
        keys: &[
            KeySpec::plain(KeyCode::Tab),
            KeySpec::plain(KeyCode::Right),
            KeySpec::plain(KeyCode::Char('l')),
        ],
        contexts: &[KeyContext::Global],
        description: "focus next panel",
    },
];

fn pane_context(focus: FocusPane) -> KeyContext {
    match focus {
        FocusPane::Commands => KeyContext::Commands,
        FocusPane::Runs => KeyContext::Runs,
        FocusPane::Inspector => KeyContext::Inspector,
        FocusPane::Output => KeyContext::Output,
    }
}

fn binding_matches(binding: &KeyBinding, key: &KeyEvent) -> bool {
    binding
        .keys
        .iter()
        .any(|spec| spec.code == key.code && key.modifiers.contains(spec.modifiers))
}

fn context_active(
    binding: &KeyBinding,
    focus: FocusPane,
    search_mode: bool,
    show_help: bool,
) -> bool {
    if show_help {
        return binding.contexts.contains(&KeyContext::HelpOpen);
    }
    binding.contexts.contains(&KeyContext::Global)
        || binding.contexts.contains(&pane_context(focus))
        || (search_mode && binding.contexts.contains(&KeyContext::Searching))
}

/// Pure key-to-action mapping with no side effects. Returns `None` when the
/// key is unbound in the current UI state (the caller swallows it, or appends
/// it to the workflow filter when typing).
pub(super) fn resolve_key(
    key: &KeyEvent,
    focus: FocusPane,
    search_mode: bool,
    show_help: bool,
) -> Option<TuiAction> {
    REGISTRY
        .iter()
        .find(|binding| {
            binding_matches(binding, key) && context_active(binding, focus, search_mode, show_help)
        })
        .map(|binding| binding.action)
}

/// Primary (first-listed) key label for an action, used by the footer so key
/// names cannot drift from the registry.
pub(super) fn primary_label(action: TuiAction) -> Option<String> {
    REGISTRY
        .iter()
        .find(|binding| binding.action == action)
        .map(KeyBinding::label)
}

/// Registry rows grouped for the `?` overlay: global, per-pane (in pane
/// order), then search. The help dialog is fully generated from this.
/// Grouping is curated by topic (every binding appears exactly once); the
/// rows themselves — key labels and descriptions — come from the registry.
pub(super) fn help_sections() -> Vec<(&'static str, Vec<&'static KeyBinding>)> {
    const ORDER: [&str; 6] = [
        "GLOBAL",
        "WORKFLOW",
        "RUNS",
        "INSPECTOR",
        "ACTIVITY",
        "SEARCH",
    ];
    let mut sections: Vec<(&'static str, Vec<&'static KeyBinding>)> =
        ORDER.into_iter().map(|title| (title, Vec::new())).collect();
    for binding in REGISTRY {
        let section = help_section(binding.action);
        sections
            .iter_mut()
            .find(|(title, _)| *title == section)
            .expect("help section exists")
            .1
            .push(binding);
    }
    sections.retain(|(_, rows)| !rows.is_empty());
    sections
}

fn help_section(action: TuiAction) -> &'static str {
    match action {
        TuiAction::SearchBackspace | TuiAction::SearchCommit => "SEARCH",
        TuiAction::FocusRuns
        | TuiAction::RunsPrev
        | TuiAction::RunsNext
        | TuiAction::RunsFirst
        | TuiAction::RunsLast
        | TuiAction::PinRun => "RUNS",
        TuiAction::InspectorUp | TuiAction::InspectorDown | TuiAction::InspectorTab(_) => {
            "INSPECTOR"
        }
        TuiAction::PageUpScroll
        | TuiAction::PageDownScroll
        | TuiAction::HalfUpScroll
        | TuiAction::HalfDownScroll
        | TuiAction::OutputUpExtend
        | TuiAction::OutputDownExtend
        | TuiAction::OutputPrev
        | TuiAction::OutputNext
        | TuiAction::OutputFirst
        | TuiAction::OutputLast
        | TuiAction::EnterVisualMode
        | TuiAction::CopySelection => "ACTIVITY",
        TuiAction::CommandsPrev
        | TuiAction::CommandsNext
        | TuiAction::CommandsFirst
        | TuiAction::CommandsLast
        | TuiAction::RunSelected
        | TuiAction::BeginSearch => "WORKFLOW",
        _ => "GLOBAL",
    }
}

/// Footer hint line for the focused pane. Key labels come from
/// [`primary_label`] (registry-derived); only the short verbs and the
/// grouping per focus are curated here.
pub(super) fn footer_spans(app: &MonitorApp) -> Vec<Span<'static>> {
    fn key(text: String) -> Span<'static> {
        Span::styled(text, Style::default().fg(Color::Cyan))
    }
    fn verb(text: &str) -> Span<'static> {
        Span::raw(format!("{text}  "))
    }

    let mut spans = Vec::new();
    if app.quit_confirm_pending {
        spans.push(Span::styled(
            " q again to stop + quit ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(verb("Esc stays"));
        return spans;
    }

    let run = primary_label(TuiAction::RunSelected).unwrap_or_else(|| "Enter".to_string());
    let focus_next = primary_label(TuiAction::FocusNext).unwrap_or_else(|| "Tab".to_string());
    let help = primary_label(TuiAction::ToggleHelp).unwrap_or_else(|| "?".to_string());
    let quit = primary_label(TuiAction::RequestQuit).unwrap_or_else(|| "q".to_string());
    let history_prev = primary_label(TuiAction::HistoryPrev).unwrap_or_else(|| "[".to_string());
    let history_next = primary_label(TuiAction::HistoryNext).unwrap_or_else(|| "]".to_string());

    match app.focus {
        FocusPane::Commands => {
            spans.push(key(run));
            spans.push(verb("run  "));
            spans.push(key("j/k".to_string()));
            spans.push(verb("move  "));
            spans.push(key(
                primary_label(TuiAction::BeginSearch).unwrap_or_else(|| "/".to_string())
            ));
            spans.push(verb("filter  "));
            spans.push(key("1-4".to_string()));
            spans.push(verb("focus  "));
        }
        FocusPane::Runs => {
            spans.push(key(
                primary_label(TuiAction::BeginSearch).unwrap_or_else(|| "/".to_string())
            ));
            spans.push(verb("filter  "));
            spans.push(key("j/k".to_string()));
            spans.push(verb("move  "));
            spans.push(key(
                primary_label(TuiAction::PinRun).unwrap_or_else(|| "Enter".to_string())
            ));
            spans.push(verb("preview  "));
            spans.push(key(focus_next));
            spans.push(verb("focus  "));
        }
        FocusPane::Inspector => {
            spans.push(key(
                primary_label(TuiAction::CycleInspectorTabs).unwrap_or_else(|| "i".to_string())
            ));
            spans.push(verb("tabs  "));
            spans.push(key("1-5".to_string()));
            spans.push(verb("tab  "));
            spans.push(key("j/k".to_string()));
            spans.push(verb("scroll  "));
            spans.push(key(focus_next));
            spans.push(verb("focus  "));
        }
        FocusPane::Output => {
            spans.push(key("j/k".to_string()));
            spans.push(verb("select  "));
            spans.push(key(
                primary_label(TuiAction::EnterVisualMode).unwrap_or_else(|| "v".to_string())
            ));
            spans.push(verb("visual  "));
            spans.push(key(
                primary_label(TuiAction::CopySelection).unwrap_or_else(|| "y".to_string())
            ));
            spans.push(verb("copy  "));
            spans.push(key(
                primary_label(TuiAction::FollowOutput).unwrap_or_else(|| "End".to_string())
            ));
            spans.push(verb("follow  "));
        }
    }
    spans.push(key(format!("{history_prev} {history_next}")));
    spans.push(verb("history  "));
    spans.push(key(help));
    spans.push(verb("help  "));
    spans.push(key(quit));
    spans.push(verb("quit  "));
    let focus = match app.focus {
        FocusPane::Commands => "workflow",
        FocusPane::Runs => "runs",
        FocusPane::Inspector => "inspector",
        FocusPane::Output => "activity",
    };
    spans.push(Span::styled(
        format!("[{focus}]"),
        Style::default().fg(Color::DarkGray),
    ));
    spans
}
