//! S2 read-only run browser (Issue #276, FR-01/FR-02).
//!
//! A run directory is recognized by the presence of `run.toml` (the run
//! manifest written by `commands::run_dir`). The browser scans the *parent*
//! of the current run directory for sibling runs, so the list answers "what
//! else was recorded next to this run" without ever leaving the artifact
//! boundary.
//!
//! Read-only contract: every probe is `metadata` / `read_dir` / bounded file
//! read. The scanner creates no files, writes no manifests, and follows no
//! symlinks (cycle safety). `scan_run_dirs` is the only filesystem entry
//! point; all filtering, preview, and panel text below it is pure.
//!
//! Budgets (FR-01, responsiveness on large trees): at most
//! [`RUN_SCAN_MAX_DIRS`] directories visited, [`RUN_SCAN_MAX_DEPTH`] levels
//! below the scan root, [`RUN_SCAN_MAX_ENTRIES`] run entries collected, and
//! [`RUN_MANIFEST_MAX_BYTES`] bytes read from a single `run.toml`.
//! When a budget trips, the outcome carries a truncation flag instead of
//! silently dropping entries, and the panel title renders `TRUNC` so the
//! operator knows the list is partial. Directories that are themselves runs
//! are treated as leaves (never descended into).
//!
//! Filtering (FR-02) reuses the S1 idiom: case-insensitive substring over
//! the same fields the inspector previews (name, path, status, stage).

use super::*;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};

/// Directories visited per scan before truncation (FR-01 count budget).
pub(super) const RUN_SCAN_MAX_DIRS: usize = 512;
/// Levels below the scan root that are descended into (FR-01 depth budget).
pub(super) const RUN_SCAN_MAX_DEPTH: u32 = 3;
/// Run entries collected per scan before truncation (FR-01 count budget).
pub(super) const RUN_SCAN_MAX_ENTRIES: usize = 200;
/// Bytes read from a single `run.toml` (oversized manifests stay unread).
pub(super) const RUN_MANIFEST_MAX_BYTES: u64 = 65_536;

/// One recorded run directory, probed read-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct RunDirEntry {
    pub(super) name: String,
    pub(super) path: String,
    pub(super) status: String,
    pub(super) stage: String,
    pub(super) updated: String,
    pub(super) has_acquisition: bool,
    pub(super) has_analysis: bool,
}

/// Budgeted scan result. `truncated_dirs` / `truncated_entries` tell the
/// panel the list is partial; they are never silent.
#[derive(Clone, Debug)]
pub(super) struct RunScanOutcome {
    pub(super) root: String,
    pub(super) entries: Vec<RunDirEntry>,
    pub(super) dirs_visited: usize,
    pub(super) truncated_dirs: bool,
    pub(super) truncated_entries: bool,
}

/// Short budget label rendered in the runs panel and inspector preview.
pub(super) fn run_scan_budgets_label() -> String {
    format!(
        "budgets {RUN_SCAN_MAX_DIRS} dirs / {RUN_SCAN_MAX_DEPTH} deep / {RUN_SCAN_MAX_ENTRIES} runs"
    )
}

/// Scan `root` for run directories within the S2 budgets.
///
/// Best-effort and read-only: unreadable directories are skipped, symlinks
/// are never followed, and entries are sorted by name for a stable list.
pub(super) fn scan_run_dirs(root: &Path) -> RunScanOutcome {
    let mut entries = Vec::new();
    let mut truncated_dirs = false;
    let mut truncated_entries = false;
    let mut dirs_visited = 0usize;
    let mut queue = VecDeque::from([(root.to_path_buf(), 0u32)]);

    while let Some((dir, depth)) = queue.pop_front() {
        if dirs_visited >= RUN_SCAN_MAX_DIRS {
            truncated_dirs = true;
            break;
        }
        dirs_visited += 1;
        let Ok(children) = fs::read_dir(&dir) else {
            continue;
        };
        for child in children.flatten() {
            let Ok(file_type) = child.file_type() else {
                continue;
            };
            // Never follow symlinks: a linked tree could cycle back onto an
            // ancestor and defeat the depth budget.
            if file_type.is_symlink() || !file_type.is_dir() {
                continue;
            }
            // The depth budget binds what the scan *finds* as well as what
            // it descends into: deeper directories are neither recorded nor
            // walked, so large trees stay responsive.
            if depth >= RUN_SCAN_MAX_DEPTH {
                continue;
            }
            let path = child.path();
            if path.join("run.toml").is_file() {
                if entries.len() >= RUN_SCAN_MAX_ENTRIES {
                    truncated_entries = true;
                    continue;
                }
                entries.push(probe_run_entry(&path));
                // Run directories are leaves; their acquisition/analysis
                // subtrees are summarized by flags, not walked.
                continue;
            }
            queue.push_back((path, depth + 1));
        }
    }

    entries.sort_by(|left, right| left.name.cmp(&right.name));
    RunScanOutcome {
        root: root.to_string_lossy().into_owned(),
        entries,
        dirs_visited,
        truncated_dirs,
        truncated_entries,
    }
}

/// Probe one run directory. Every access is a read; failures degrade to
/// `"unknown"` / `false` rather than failing the scan.
fn probe_run_entry(path: &Path) -> RunDirEntry {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned());
    let (status, stage, updated) = read_run_manifest_state(&path.join("run.toml"));
    RunDirEntry {
        name,
        path: path.to_string_lossy().into_owned(),
        status,
        stage,
        updated,
        has_acquisition: path.join("acquisition").is_dir()
            || path.join("acquisition.incomplete").is_dir(),
        has_analysis: path.join("analysis").is_dir() || path.join("analysis.incomplete").is_dir(),
    }
}

/// Read `(status, stage, updated_at)` from a run manifest without depending
/// on the private `RunState` shape. Oversized or unparsable manifests yield
/// unknowns; the entry is still listed.
fn read_run_manifest_state(manifest: &Path) -> (String, String, String) {
    let Ok(meta) = fs::metadata(manifest) else {
        return (
            "unknown".to_string(),
            "unknown".to_string(),
            "unknown".to_string(),
        );
    };
    if meta.len() > RUN_MANIFEST_MAX_BYTES {
        return (
            "unknown".to_string(),
            "unknown".to_string(),
            "unknown".to_string(),
        );
    }
    let Ok(text) = fs::read_to_string(manifest) else {
        return (
            "unknown".to_string(),
            "unknown".to_string(),
            "unknown".to_string(),
        );
    };
    let value: toml::Value = match toml::from_str(&text) {
        Ok(value) => value,
        Err(_) => {
            return (
                "unknown".to_string(),
                "unknown".to_string(),
                "unknown".to_string(),
            );
        }
    };
    let field = |key: &str| {
        value
            .get(key)
            .and_then(|field| field.as_str())
            .unwrap_or("unknown")
            .to_string()
    };
    (field("status"), field("stage"), field("updated_at"))
}

/// Resolve the browser root: parent of the current run directory, falling
/// back to the config file's parent and then the startup directory. All
/// three are read-only scan roots; none is ever created.
pub(super) fn run_browser_root(config_path: &str, load: &ConfigLoad, current_dir: &str) -> PathBuf {
    if let ConfigLoad::Ready { config, .. } = load {
        let run_dir = config.paths().run_dir;
        if let Some(parent) = run_dir
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            return parent.to_path_buf();
        }
        return PathBuf::from(current_dir);
    }
    let path = Path::new(config_path);
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        return parent.to_path_buf();
    }
    PathBuf::from(current_dir)
}

/// S1-idiom filter over the previewed fields (FR-02). Returns indices into
/// `entries` so the cursor stays stable while typing.
pub(super) fn filter_run_entries(entries: &[RunDirEntry], query: &str) -> Vec<usize> {
    let query = query.trim().to_ascii_lowercase();
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            query.is_empty()
                || entry.name.to_ascii_lowercase().contains(&query)
                || entry.path.to_ascii_lowercase().contains(&query)
                || entry.status.to_ascii_lowercase().contains(&query)
                || entry.stage.to_ascii_lowercase().contains(&query)
        })
        .map(|(index, _)| index)
        .collect()
}

/// Inspector rows for the selected run (FR-02 preview). Rendered with the
/// shared `two_col_table` helper next to the workflow panels.
#[cfg(test)]
pub(super) fn run_preview_rows(entry: &RunDirEntry) -> Vec<Vec<String>> {
    vec![
        vec!["Run".to_string(), entry.name.clone()],
        vec!["Status".to_string(), entry.status.clone()],
        vec!["Stage".to_string(), entry.stage.clone()],
        vec!["Updated".to_string(), entry.updated.clone()],
        vec!["Path".to_string(), entry.path.clone()],
        vec![
            "Acquisition".to_string(),
            if entry.has_acquisition {
                "present".to_string()
            } else {
                "missing".to_string()
            },
        ],
        vec![
            "Analysis".to_string(),
            if entry.has_analysis {
                "present".to_string()
            } else {
                "missing".to_string()
            },
        ],
    ]
}

/// FR-04: the read-only Utilities actions surfaced as first-class panels.
/// `None` for every other action, so only show / raw-verify / doctor gain
/// the panel note. Launching still goes through the existing re-exec
/// runner (`spawn_command_runner`); the browser never executes or writes.
pub(super) fn readout_panel_note(action: MonitorAction) -> Option<&'static str> {
    match action {
        MonitorAction::Show => Some(
            "Read-only config panel. Enter re-runs `show` through the re-exec runner; nothing is written.",
        ),
        MonitorAction::RawVerify => Some(
            "Read-only RAW panel. Enter re-runs `raw verify` through the re-exec runner; nothing is written.",
        ),
        MonitorAction::Doctor => Some(
            "Read-only doctor panel. Enter re-runs `doctor` through the re-exec runner; nothing is written.",
        ),
        _ => None,
    }
}
