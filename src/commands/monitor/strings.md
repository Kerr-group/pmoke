# Monitor TUI string inventory + CJK cell budgets (S4, Issue #278, Phase 2)

Scope: S1 keybinding registry (Issue #275) + S2 run browser (Issue #276) +
S3 calibration/analysis inspectors (Issue #277). Phase 1 covered S1/S2;
Phase 2 adds the S3 REPORTS tab below.

## Source-of-truth rule

- Key bindings, their labels, and their help descriptions live in exactly one
  place: `REGISTRY` in `keymap.rs`. The `?` overlay (`help_sections`) and the
  per-focus footer (`footer_spans`) render from that table, so help/footer
  text cannot drift from the implementation.
- This file therefore does NOT duplicate the binding table. Binding behavior
  is pinned by `tests/keymap.rs` (determinism: modal > search > pane >
  global, first match wins) and by the help-overlay snapshot in
  `tests/snapshots.rs`.
- TUI display strings stay English-only (Issue #278, FR-04). Only the website
  `monitor.mdx` pages are paired EN/JA.

## Curated static strings (not registry-derived)

| String | Location | Notes |
| --- | --- | --- |
| ` pmoke `, `●`, status (`RUNNABLE`/`BLOCKED`), `│`, run label, `MM:SS` | `header_spans` (`view.rs`) | `elapsed()` ticks whole minutes/seconds only |
| `◆ cwd …  │  config …` / compact `◆ …` | `context_bar_spans` (`view.rs`) | path budgets use `cell_width`, never char count |
| ` WORKFLOW {:02}/{} `, ` WORKFLOW /{query} `, ` WORKFLOW /{query} · NO MATCHES ` | `render_workflow_list` (`view.rs`) | S1 filter; `/` begins, `Enter` commits, `Esc` clears |
| ` RUNS {:02}/{total} `, ` RUNS /{query} {:02}/{total} `, ` RUNS /{query} · NO MATCHES `, ` RUNS 0/0 `, `no run dirs under scan root`, `no runs match the filter`, optional `TRUNC` | `render_runs_panel`, `runs_panel_title` (`view.rs`) | S2 browser; budgets `RUN_SCAN_MAX_*` in `runs.rs` |
| `▌ ` selection marker | `render_runs_panel` (`view.rs`) | single-cell block, width 1 |
| Inspector tab labels `SUMMARY`, `CONFIG`, `DIAGNOSTICS`, `ARTIFACTS`, `REPORTS` | `InspectorView::label` (`mod.rs`) | `i` cycles; inspector-local `1`-`5` select (`5` = S3 REPORTS) |
| ` Help ` overlay, section titles `GLOBAL WORKFLOW RUNS INSPECTOR ACTIVITY SEARCH` | `help_sections` (`keymap.rs`), `panels.rs` | fully registry-generated rows |
| Footer verbs per focus (`run move filter focus …`, `filter move preview focus …`, `tabs tab scroll focus …`, `select visual copy follow …`, shared `history help quit [focus]`, quit-confirm ` q again to stop + quit ` / `Esc stays`) | `footer_spans` (`keymap.rs`) | key labels via `primary_label`; verbs curated |
| `HISTORY {i}/{n}`, `PAUSED · {n} NEW · G follow`, `PAUSED · G follow`, `STOPPING · …` | `activity_title` (`view.rs`) | never wall-clock, only counters/state |
| `capture on (click/drag/wheel); PMOKE_MOUSE=off disables`, `capture off (PMOKE_MOUSE=off)` | `panels.rs` | FR-05 mouse opt-out |
| `PMOKE_MOTION=full\|reduced\|off` tick rates | `mod.rs` (`MotionMode`, `tui_frame_tick`) | snapshots run idle; motion never affects frames |

## S3 REPORTS tab curated strings (Phase 2)

The REPORTS tab (`InspectorView::Reports` in `mod.rs`, rendered by
`render_reports` in `view.rs` from `reports_table_rows` in `inspect.rs`)
shows the S2-selected run: a run header, a `Stages` section from the run
manifest (FR-02), and a `Reports` section of standalone lane summaries
(FR-01). `5` selects the tab while the inspector is focused; scrolling
reuses the inspector `j`/`k` actions into `reports_scroll` (reset by
`refresh`, clamped by `reports_scroll_max`, title shows the scroll range).

| String | Location | Notes |
| --- | --- | --- |
| ` REPORTS `, empty `No recorded run selected.`, `root  …`, `scan  …`, `hint  press … to focus the runs browser` | `render_reports` (`view.rs`) | no run selected; scan line quotes `run_scan_budgets_label` |
| `Report: {name}` + `Value` header, `REPORTS {start}-{end}/{total}` title | `render_reports` (`view.rs`) | single scroll window over the combined row list |
| `Run`, `Status`, `Stage`, `Updated`, `Path` | `reports_table_rows` (`inspect.rs`) | header rows from the S2 `RunDirEntry` |
| `Stages` / `per-stage status from run manifest` | `reports_table_rows` (`inspect.rs`) | section label row |
| `Status`, `Stage`, `Started`, `Acquired`, `Analyzed`, `Completed`, `Updated`, conditional `Failed stage`, analysis attempt rows, published rows | `stage_rows` (`inspect.rs`) | from `run.toml`; missing fields degrade to `unknown` / `—`, never an error |
| `Reports` / `calibrate / noise / compare / evaluate` | `reports_table_rows` (`inspect.rs`) | section label row |
| lane labels `calibrate`, `noise`, `compare`, `evaluate`; files `calibrate-report.json`, `diagnostics.json`, `compare-report.json`, `m6-evaluation-report.json` | `ReportKind` (`inspect.rs`) | calibrate additionally probes `./calibration/` |
| `not found ({file} in run dir)`, `unreadable ({oversized\|unreadable\|invalid json})` | `report_table_rows` (`inspect.rs`) | symlinks refused (report as missing, next candidate tried) |
| `calibration artifact` / `not found (calibration.json in run dir or ./calibration/)`, `model=… ch=… f=…` summary | `calibration_artifact_row` (`inspect.rs`) | artifact budget mirrors `calibrate inspect` |
| `analysis results` / `no analysis manifest in run dir`, `published through {lane} (gen {n})` | `analysis_manifest_row` (`inspect.rs`) | from `<run>/analysis/manifest.toml` |

Probe budgets (`inspect.rs`): `STAGE_MANIFEST_MAX_BYTES` 65_536,
`REPORT_MAX_BYTES` 65_536, `ARTIFACT_MAX_BYTES` 1 MiB. Contracts: every
probe is a bounded read-only file read (nothing created, written, or
spawned); all parsing goes through `toml::Value` / `serde_json::Value`, so
a kernel field rename degrades a row to `unknown` instead of breaking the
build. Pinned by `tests/inspect.rs` (row content, degradation, read-only
tree snapshot) and by the `reports_empty` / `reports_populated` canonical
frames in `tests/snapshots.rs` (chrome + scroll range at `WIDE`).

## CJK cell budgets

All truncation/padding go through `formatting.rs` (`fit_text`, `fit_path`,
`pad_display_width`) on `unicode_width` cell widths: ASCII/latin = 1 cell,
CJK = 2 cells. Rules:

1. Never truncate by char count where the result is rendered; use `fit_text`
   (adds `…`, 1 cell) or `fit_path` (keeps the tail).
2. Context-bar path budgets split the available width between cwd and config
   (`config` capped at a third); at narrow widths the bar degrades to the
   `◆` icon form instead of overflowing.
3. Run names are fitted to 20 cells, statuses to 10 cells; the `▌` marker is
   outside the text budget.

Enforced by `tests/snapshots.rs`:

- every rendered line of every canonical frame is at most the frame width in
  cells (checked by `assert_fits_width` inside each snapshot test);
- a CJK probe (`fit_text` over mixed-width input) stays within budget and
  `pad_display_width` reaches exact width (`cjk_probe_stays_within_budget`).

## Layout minima and snapshot size classes

| Class | Size | What it pins |
| --- | --- | --- |
| wide | 120x30 | full dashboard: header, context bar, workflow + runs, inspector, activity, footer |
| compact | 72x24 | core regions kept (`view.rs` test); context details collapse below 60 cells |
| tiny | 40x14 | inspector hidden, workflow + activity preserved |

Minima: `WORKFLOW_MIN_WIDTH` 24, `ACTIVITY_MIN_WIDTH` 48 (`layout.rs`);
`CONTEXT_DETAILS_MIN_WIDTH` 60 (`mod.rs`); `ELAPSED_MIN_OUTPUT_WIDTH` 52,
`ELAPSED_PREFIX_WIDTH` 9, `OUTPUT_PREFIX_WIDTH` 12. Change a minimum only
together with its snapshot and this table.

## Maintenance checklist (per TUI change)

1. Change the string at its source location above, not in a test or doc.
2. Re-run snapshots with review (`INSTA_UPDATE=review` or `always` followed
   by reading the diff); never blind-accept.
3. Update the website `monitor.mdx` EN/JA pair when user-visible behavior
   changes (doc-pair gate: both locales, same sections).
4. Keep the search-recall gate green (`pnpm build` runs it): JA wording feeds
   the hybrid index, so a new JA term that collides with another page's
   concept (e.g. `設定` vs validation/installation queries) can push those
   queries out of the top 5. Prefer `コンフィグ`/established mixed terms on
   the monitor page, tune `SEARCH_HINTS` repetitions in
   `website/app/api/semantic-search/route.ts` (counts matter; see the S4
   handoff), and keep `tests/fixtures/search-relevance-v1.json` monitor
   queries hitting. Ship only at Recall@5 >= 0.95 per locale. Phase 2
   lesson: literal `*.toml` filenames on the JA page fired the `config`
   concept and stole a validation query; the JA page paraphrases them
   (`run manifest`, `analysis/manifest`) while EN keeps exact filenames.
5. Keep this inventory's tables and minima in sync.
