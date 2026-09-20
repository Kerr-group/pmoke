//! S3 read-only calibration/analysis inspectors (Issue #277, FR-01/FR-02).
//!
//! The S2 run browser answers "which runs exist"; this module answers "what
//! happened inside the selected run" without executing anything:
//! - FR-02: per-stage lifecycle status derived from the selected run's
//!   `run.toml` (timestamps, failure detail, analysis attempt state);
//! - FR-01: compact summaries of the standalone JSON reports the analysis
//!   lanes write (`calibrate-report.json`, noise `diagnostics.json`,
//!   `compare-report.json`, `m6-evaluation-report.json`), plus the
//!   `calibration.json` artifact summary and the run's
//!   `analysis/manifest.toml` publication state.
//!
//! Read-only contract: every probe is a bounded file read (`metadata` size
//! check, then `read`). Nothing is created, written, or spawned; symlinked
//! report files are refused (a link could point outside the artifact
//! boundary). Oversized or unparsable files degrade to an explicit
//! `unreadable`/`oversized` marker — the inspector never fails a render.
//!
//! No-kernel-coupling contract (FR-03): all parsing goes through
//! `toml::Value` / `serde_json::Value`. The private `RunState`,
//! `CalibrateReport`, `DiagnosticReport`, `CompareReport`, and
//! `EvaluationReport` shapes are never referenced, so a kernel field rename
//! degrades an S3 row to `unknown` instead of breaking the build. FR-03 is
//! verifiable by diff: this slice touches only `src/commands/monitor/`.
//!
//! Probe roots: the selected run directory first; calibrate additionally
//! falls back to `./calibration/` (the documented CWD-anchored default
//! direct-build output). Noise/compare/evaluate reports live beside their
//! user-chosen requests, so outside a run directory they have no
//! conventional location and are reported as not found.

use super::*;
use std::path::{Path, PathBuf};

/// Bytes read from a single `run.toml` / `manifest.toml` (same family as the
/// S2 [`RUN_MANIFEST_MAX_BYTES`] budget; manifests are small TOML).
pub(super) const STAGE_MANIFEST_MAX_BYTES: u64 = 65_536;
/// Bytes read from a single JSON report (reports carry full resolved
/// requests and can be tens of KiB; parsing stays bounded).
pub(super) const REPORT_MAX_BYTES: u64 = 65_536;
/// Bytes read from a `calibration.json` artifact. Mirrors the cap enforced
/// by `calibrate inspect` (`MAX_CALIBRATION_ARTIFACT_BYTES`): artifacts hold
/// phase-bin arrays and legitimately approach a megabyte.
pub(super) const ARTIFACT_MAX_BYTES: u64 = 1 << 20;

/// Per-stage lifecycle status of one run directory. Every field degrades to
/// `"unknown"` / `"—"` when the manifest is missing, oversized, unparsable,
/// or simply predates the field — never an error.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct StageState {
    pub(super) status: String,
    pub(super) stage: String,
    pub(super) started_at: String,
    pub(super) acquired_at: String,
    pub(super) analysis_started_at: String,
    pub(super) analyzed_at: String,
    pub(super) completed_at: String,
    pub(super) updated_at: String,
    pub(super) failed_stage: String,
    pub(super) error_summary: String,
    pub(super) analysis_command: String,
    pub(super) analysis_status: String,
    pub(super) analysis_generation: String,
    pub(super) analysis_started: String,
    pub(super) analysis_completed: String,
    pub(super) published_through: String,
    pub(super) published_generation: String,
}

/// Read the lifecycle state of `run_dir/run.toml`. Pure read path shared by
/// the inspector render and the unit tests.
pub(super) fn read_stage_state(run_dir: &Path) -> StageState {
    let value = read_toml_value(&run_dir.join("run.toml"), STAGE_MANIFEST_MAX_BYTES);
    let field = |key: &str| toml_field(&value, key);
    let analysis = value.as_ref().and_then(|value| value.get("analysis"));
    let last_attempt = analysis.and_then(|analysis| analysis.get("last_attempt"));
    let attempt_text = |key: &str| {
        last_attempt
            .and_then(|last| last.get(key))
            .and_then(toml::Value::as_str)
            .unwrap_or("—")
            .to_string()
    };
    let attempt_generation = last_attempt
        .and_then(|last| last.get("generation"))
        .and_then(toml::Value::as_integer)
        .map(|num| num.to_string())
        .unwrap_or_else(|| "—".to_string());
    let published_through = analysis
        .and_then(|analysis| analysis.get("published_through"))
        .and_then(toml::Value::as_str)
        .unwrap_or("—")
        .to_string();
    let published_generation = analysis
        .and_then(|analysis| analysis.get("published_generation"))
        .and_then(toml::Value::as_integer)
        .map(|num| num.to_string())
        .unwrap_or_else(|| "—".to_string());
    StageState {
        status: field("status"),
        stage: field("stage"),
        started_at: toml_stamp(&value, "started_at"),
        acquired_at: toml_stamp(&value, "acquired_at"),
        analysis_started_at: toml_stamp(&value, "analysis_started_at"),
        analyzed_at: toml_stamp(&value, "analyzed_at"),
        completed_at: toml_stamp(&value, "completed_at"),
        updated_at: toml_stamp(&value, "updated_at"),
        failed_stage: toml_opt(&value, "failed_stage"),
        error_summary: toml_opt(&value, "error_summary"),
        analysis_command: attempt_text("command"),
        analysis_status: attempt_text("status"),
        analysis_generation: attempt_generation,
        analysis_started: attempt_text("started_at"),
        analysis_completed: attempt_text("completed_at"),
        published_through,
        published_generation,
    }
}

/// Lifecycle rows for the shared `two_col_table` helper (FR-02). Lifecycle
/// milestones render `"—"` until reached; failure and analysis-attempt rows
/// only appear when the manifest carries them, so a fresh run stays compact.
pub(super) fn stage_rows(state: &StageState) -> Vec<Vec<String>> {
    let mut rows = vec![
        vec!["Status".to_string(), state.status.clone()],
        vec!["Stage".to_string(), state.stage.clone()],
        vec!["Started".to_string(), state.started_at.clone()],
        vec!["Acquired".to_string(), state.acquired_at.clone()],
        vec![
            "Analysis started".to_string(),
            state.analysis_started_at.clone(),
        ],
        vec!["Analyzed".to_string(), state.analyzed_at.clone()],
        vec!["Completed".to_string(), state.completed_at.clone()],
        vec!["Updated".to_string(), state.updated_at.clone()],
    ];
    if state.failed_stage != "—" || state.error_summary != "—" {
        rows.push(vec!["Failed stage".to_string(), state.failed_stage.clone()]);
        rows.push(vec![
            "Error".to_string(),
            truncate_cell(&state.error_summary, 120),
        ]);
    }
    if state.analysis_command != "—" || state.analysis_status != "—" {
        rows.push(vec![
            "Attempt".to_string(),
            format!(
                "{} {} (gen {})",
                state.analysis_command, state.analysis_status, state.analysis_generation
            ),
        ]);
        rows.push(vec![
            "Attempt window".to_string(),
            format!("{} → {}", state.analysis_started, state.analysis_completed),
        ]);
    }
    if state.published_through != "—" {
        rows.push(vec![
            "Published".to_string(),
            format!(
                "through {} (gen {})",
                state.published_through, state.published_generation
            ),
        ]);
    }
    rows
}

/// The four standalone report lanes surfaced read-only (FR-01).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ReportKind {
    Calibrate,
    Noise,
    Compare,
    Evaluate,
}

impl ReportKind {
    pub(super) const ALL: [Self; 4] = [Self::Calibrate, Self::Noise, Self::Compare, Self::Evaluate];

    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Calibrate => "calibrate",
            Self::Noise => "noise",
            Self::Compare => "compare",
            Self::Evaluate => "evaluate",
        }
    }

    pub(super) fn file_name(self) -> &'static str {
        match self {
            Self::Calibrate => "calibrate-report.json",
            Self::Noise => "diagnostics.json",
            Self::Compare => "compare-report.json",
            Self::Evaluate => "m6-evaluation-report.json",
        }
    }

    /// Ordered probe candidates: the selected run first, then — for
    /// calibrate only — the documented CWD-anchored default output dir.
    pub(super) fn candidates(self, run_dir: &Path) -> Vec<PathBuf> {
        let mut out = vec![run_dir.join(self.file_name())];
        if matches!(self, Self::Calibrate) {
            let fallback = Path::new("calibration").join(self.file_name());
            if fallback != out[0] {
                out.push(fallback);
            }
        }
        out
    }

    /// Compact key/value summary extracted from a parsed report. Unknown
    /// shapes (e.g. a staging `diagnostics.json` from another lane) degrade
    /// to `unknown` fields rather than failing the probe.
    pub(super) fn summarize(self, value: &serde_json::Value) -> Vec<(String, String)> {
        match self {
            Self::Calibrate => vec![
                ("model".to_string(), json_str(value, "model_id")),
                (
                    "adequate".to_string(),
                    json_bool(value, "adequate").unwrap_or_else(|| "unknown".to_string()),
                ),
                (
                    "blocks".to_string(),
                    json_uint(value, "training_blocks").unwrap_or_else(|| "unknown".to_string()),
                ),
                (
                    "modes".to_string(),
                    json_str_list(value, "modes").unwrap_or_else(|| "unknown".to_string()),
                ),
                (
                    "tolerance".to_string(),
                    value
                        .get("tolerance_basis")
                        .and_then(|basis| basis.get("final_tol"))
                        .and_then(serde_json::Value::as_f64)
                        .map(|tol| format!("{tol:.6e}"))
                        .unwrap_or_else(|| "unknown".to_string()),
                ),
                (
                    "warnings".to_string(),
                    value
                        .get("warnings")
                        .and_then(serde_json::Value::as_array)
                        .map(|warnings| warnings.len().to_string())
                        .unwrap_or_else(|| "unknown".to_string()),
                ),
            ],
            Self::Noise => vec![
                ("operation".to_string(), json_str(value, "operation")),
                ("finding".to_string(), json_str(value, "diagnostic_finding")),
                (
                    "reason".to_string(),
                    truncate_cell(&json_str(value, "finding_reason"), 100),
                ),
                (
                    "measurements".to_string(),
                    json_bool(value, "measurements_available")
                        .unwrap_or_else(|| "unknown".to_string()),
                ),
            ],
            Self::Compare => {
                let legs = value
                    .get("legs")
                    .and_then(serde_json::Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                let complete = legs
                    .iter()
                    .filter(|leg| {
                        leg.get("status").and_then(serde_json::Value::as_str) == Some("complete")
                    })
                    .count();
                vec![
                    ("gate".to_string(), json_str(value, "gate_version")),
                    (
                        "legs".to_string(),
                        format!("{complete}/{} complete", legs.len()),
                    ),
                    (
                        "grid".to_string(),
                        json_bool(value, "shared_grid_equal")
                            .unwrap_or_else(|| "unknown".to_string()),
                    ),
                    (
                        "reference".to_string(),
                        json_bool(value, "shared_reference_equal")
                            .unwrap_or_else(|| "unknown".to_string()),
                    ),
                    (
                        "complete".to_string(),
                        json_bool(value, "computation_complete")
                            .unwrap_or_else(|| "unknown".to_string()),
                    ),
                ]
            }
            Self::Evaluate => vec![
                ("method".to_string(), json_str(value, "method_label")),
                (
                    "channel".to_string(),
                    json_uint(value, "channel").unwrap_or_else(|| "unknown".to_string()),
                ),
                ("benefit".to_string(), json_str(value, "benefit_gate")),
                ("verdict".to_string(), json_str(value, "scientific_verdict")),
                (
                    "sd_ratio".to_string(),
                    value
                        .get("sd_ratio")
                        .and_then(serde_json::Value::as_f64)
                        .map(|ratio| format!("{ratio:.4}"))
                        .unwrap_or_else(|| "—".to_string()),
                ),
                (
                    "complete".to_string(),
                    json_bool(value, "computation_complete")
                        .unwrap_or_else(|| "unknown".to_string()),
                ),
            ],
        }
    }
}

/// Full REPORTS-tab row list for the selected run: run header, STAGES
/// section (FR-02, from the run manifest), REPORTS section (FR-01,
/// standalone lane reports). Rendered as one `two_col_table` with a single
/// scroll window (`reports_scroll`). Empty when no run is selected; the
/// caller renders the empty state instead.
pub(super) fn reports_table_rows(app: &MonitorApp) -> Vec<Vec<String>> {
    let Some(entry) = app.selected_run_entry() else {
        return Vec::new();
    };
    let run_dir = Path::new(&entry.path);
    let mut rows = vec![
        vec!["Run".to_string(), entry.name.clone()],
        vec!["Status".to_string(), entry.status.clone()],
        vec!["Stage".to_string(), entry.stage.clone()],
        vec!["Updated".to_string(), entry.updated.clone()],
        vec!["Path".to_string(), entry.path.clone()],
        vec![
            "Stages".to_string(),
            "per-stage status from run manifest".to_string(),
        ],
    ];
    rows.extend(stage_rows(&read_stage_state(run_dir)));
    rows.push(vec![
        "Reports".to_string(),
        "calibrate / noise / compare / evaluate".to_string(),
    ]);
    rows.extend(report_table_rows(run_dir));
    rows
}

/// One line per report lane for the REPORTS table: `present` with a compact
/// summary, or an explicit `not found` / `unreadable` marker. The selected
/// run directory is probed first; calibrate additionally probes the
/// CWD-anchored `./calibration/` default. Every access is a bounded read.
pub(super) fn report_table_rows(run_dir: &Path) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    for kind in ReportKind::ALL {
        let mut probed: Option<(PathBuf, String)> = None;
        for candidate in kind.candidates(run_dir) {
            match read_json_value(&candidate, REPORT_MAX_BYTES) {
                JsonRead::Missing => continue,
                JsonRead::Unreadable(reason) => {
                    probed = Some((candidate, format!("unreadable ({reason})")));
                    break;
                }
                JsonRead::Value(value) => {
                    let summary = kind
                        .summarize(&value)
                        .into_iter()
                        .map(|(key, val)| format!("{key}={val}"))
                        .collect::<Vec<_>>()
                        .join(" ");
                    probed = Some((candidate, truncate_cell(&summary, 160)));
                    break;
                }
            }
        }
        match probed {
            Some((path, summary)) => rows.push(vec![
                kind.label().to_string(),
                format!("{} ({})", summary, short_path(&path)),
            ]),
            None => rows.push(vec![
                kind.label().to_string(),
                format!("not found ({} in run dir)", kind.file_name()),
            ]),
        }
    }
    rows.push(calibration_artifact_row(run_dir));
    rows.push(analysis_manifest_row(run_dir));
    rows
}

/// `calibration.json` artifact summary (the `calibrate inspect` fields that
/// fit one row). Probed in the run first, then `./calibration/`.
fn calibration_artifact_row(run_dir: &Path) -> Vec<String> {
    let candidates = [
        run_dir.join("calibration.json"),
        Path::new("calibration").join("calibration.json"),
    ];
    for candidate in candidates {
        match read_json_value(&candidate, ARTIFACT_MAX_BYTES) {
            JsonRead::Missing => continue,
            JsonRead::Unreadable(reason) => {
                return vec![
                    "calibration artifact".to_string(),
                    format!("unreadable ({reason})"),
                ];
            }
            JsonRead::Value(value) => {
                let channel = value
                    .get("binding")
                    .and_then(|binding| binding.get("channel"))
                    .and_then(serde_json::Value::as_u64)
                    .map(|ch| ch.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                let freq = value
                    .get("binding")
                    .and_then(|binding| binding.get("reference_frequency_hz"))
                    .and_then(serde_json::Value::as_f64)
                    .map(|freq| format!("{freq:.6e} Hz"))
                    .unwrap_or_else(|| "unknown".to_string());
                return vec![
                    "calibration artifact".to_string(),
                    format!(
                        "model={} ch={} f={} ({})",
                        truncate_cell(&json_str(&value, "model_id"), 40),
                        channel,
                        freq,
                        short_path(&candidate),
                    ),
                ];
            }
        }
    }
    vec![
        "calibration artifact".to_string(),
        "not found (calibration.json in run dir or ./calibration/)".to_string(),
    ]
}

/// `<run>/analysis/manifest.toml` publication state: which lane the analysis
/// most recently published through and at which generation.
fn analysis_manifest_row(run_dir: &Path) -> Vec<String> {
    let manifest = run_dir.join("analysis").join("manifest.toml");
    let value = read_toml_value(&manifest, STAGE_MANIFEST_MAX_BYTES);
    match value {
        None => vec![
            "analysis results".to_string(),
            "no analysis manifest in run dir".to_string(),
        ],
        Some(value) => {
            let through = value
                .get("published_through")
                .and_then(toml::Value::as_str)
                .unwrap_or("unknown");
            let generation = value
                .get("published_generation")
                .and_then(toml::Value::as_integer)
                .map(|num| num.to_string())
                .unwrap_or_else(|| "—".to_string());
            vec![
                "analysis results".to_string(),
                format!("published through {through} (gen {generation})"),
            ]
        }
    }
}

enum JsonRead {
    Missing,
    Unreadable(&'static str),
    Value(serde_json::Value),
}

/// Bounded JSON read. Missing files (including refused symlinks) report
/// `Missing` so later candidates are tried; oversized/unparsable files
/// report `Unreadable` with the reason inline.
fn read_json_value(path: &Path, max_bytes: u64) -> JsonRead {
    if is_refused_link(path) {
        return JsonRead::Missing;
    }
    let Ok(meta) = fs::metadata(path) else {
        return JsonRead::Missing;
    };
    if !meta.is_file() || meta.len() > max_bytes {
        return JsonRead::Unreadable(if meta.is_file() {
            "oversized"
        } else {
            "not a file"
        });
    }
    let Ok(bytes) = fs::read(path) else {
        return JsonRead::Unreadable("unreadable");
    };
    match serde_json::from_slice(&bytes) {
        Ok(value) => JsonRead::Value(value),
        Err(_) => JsonRead::Unreadable("invalid json"),
    }
}

/// Bounded TOML read without depending on the kernel's manifest structs.
/// Anything unexpected (missing, symlink, oversized, unreadable,
/// unparsable) yields `None` and every caller degrades to unknowns.
fn read_toml_value(path: &Path, max_bytes: u64) -> Option<toml::Value> {
    if is_refused_link(path) {
        return None;
    }
    let meta = fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > max_bytes {
        return None;
    }
    let text = fs::read_to_string(path).ok()?;
    toml::from_str(&text).ok()
}

/// Symlinked report files are never followed: a link could resolve outside
/// the artifact boundary the scan root implies.
fn is_refused_link(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|meta| meta.file_type().is_symlink())
        .unwrap_or(false)
}

fn toml_field(value: &Option<toml::Value>, key: &str) -> String {
    value
        .as_ref()
        .and_then(|value| value.get(key))
        .and_then(toml::Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

/// Optional timestamp: `"—"` until the lifecycle reaches the milestone.
fn toml_stamp(value: &Option<toml::Value>, key: &str) -> String {
    value
        .as_ref()
        .and_then(|value| value.get(key))
        .and_then(toml::Value::as_str)
        .unwrap_or("—")
        .to_string()
}

/// Optional free text: `"—"` when absent (failure detail, error summaries).
fn toml_opt(value: &Option<toml::Value>, key: &str) -> String {
    value
        .as_ref()
        .and_then(|value| value.get(key))
        .and_then(toml::Value::as_str)
        .unwrap_or("—")
        .to_string()
}

fn json_str(value: &serde_json::Value, key: &str) -> String {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_string()
}

fn json_bool(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_bool)
        .map(|flag| flag.to_string())
}

fn json_uint(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_u64)
        .map(|num| num.to_string())
}

fn json_str_list(value: &serde_json::Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join(",")
        })
}

fn truncate_cell(text: &str, max_chars: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max_chars {
        return text.to_string();
    }
    chars[..max_chars.max(1) - 1].iter().collect::<String>() + "…"
}

/// Display path relative to the current directory when inside it, so report
/// rows name a short stable location instead of an absolute path.
fn short_path(path: &Path) -> String {
    let text = path.to_string_lossy();
    if let Ok(current) = std::env::current_dir()
        && let Ok(relative) = path.strip_prefix(&current)
        && !relative.as_os_str().is_empty()
    {
        return relative.to_string_lossy().into_owned();
    }
    // Never emit Windows `\` separators into a manifest-adjacent display
    // path (forward slashes stay stable across platforms).
    text.replace('\\', "/")
}
