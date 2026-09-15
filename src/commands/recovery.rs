//! Durable publication journal and restart recovery (R3, AT-024/AT-027).
//!
//! The two-rename publication (`destination -> backup`, `staging ->
//! destination`) is crash-unsafe between the renames: after a kill the
//! authoritative generation is ambiguous (backup only, destination only, or
//! both). This module adds a small durable journal beside the destination
//! that records the intent *before* the first rename, so a restart can
//! determine exactly which generation is authoritative:
//!
//! - intent written (atomic file) -> first rename -> second rename ->
//!   post-commit sync -> outcome recorded -> journal removed (atomic).
//! - Crash before the first rename: destination is authoritative (new
//!   generation never moved into place; intent alone changes nothing).
//! - Crash with backup present and no complete destination: the second
//!   rename never ran. When the surviving staging still matches the
//!   journaled digest, finish the rename; otherwise the staging is not the
//!   journaled attempt (a fresh prepare wiped the killed run's staging), so
//!   restore the previous generation instead. Either way verified output is
//!   never lost.
//! - Crash with both present: the second rename completed but the journal
//!   removal did not (post-commit sync uncertain); the destination
//!   generation is authoritative and the backup is retained for inspection.
//! - A completion record (committed generation + digest) distinguishes
//!   commit outcomes from uncertain durability; cancel-before-commit is an
//!   explicit outcome that never touches the destination.
//!
//! The journal is a single TOML file `<destination>.publish-journal.toml`
//! next to the destination directory, written atomically. The backup is a
//! single deterministic sibling `<destination>.publish-backup` so a restart
//! can always find it. Recovery never deletes user data: it only completes
//! an interrupted second rename (after digest proof), restores the previous
//! generation, or reports which generation is authoritative.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::fs;
use std::path::{Path, PathBuf};

/// Outcome of a publication attempt, recorded durably.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublishOutcome {
    /// Staging was published; destination holds the new generation.
    Committed,
    /// Publication was cancelled before any rename; destination untouched.
    Cancelled,
    /// Destination was replaced but post-commit durability (sync/journal
    /// removal) did not finish; destination holds the new generation but
    /// durability is uncertain.
    UncertainDurability,
}

/// Durable intent record written before the first rename.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishJournal {
    pub schema_version: u32,
    pub staging_generation: Option<u64>,
    pub staging_digest: String,
    pub previous_generation: Option<u64>,
    pub previous_digest: Option<String>,
    pub outcome: Option<PublishOutcome>,
}

pub const PUBLISH_JOURNAL_SCHEMA_VERSION: u32 = 1;

/// Deterministic backup sibling for a publication destination. The name must
/// not contain the process id: a restart runs under a new pid and still has
/// to find the backup left behind by the killed process.
pub fn deterministic_backup_path(destination: &Path) -> PathBuf {
    let mut name = destination.file_name().unwrap_or_default().to_os_string();
    name.push(".publish-backup");
    destination.with_file_name(name)
}

pub fn journal_path(destination: &Path) -> PathBuf {
    let mut name = destination.file_name().unwrap_or_default().to_os_string();
    name.push(".publish-journal.toml");
    destination.with_file_name(name)
}

pub fn staging_tree_digest(staging: &Path) -> Result<String> {
    if !staging.is_dir() {
        bail!(
            "publication staging is not a directory: {}",
            staging.display()
        );
    }
    let mut entries = Vec::new();
    collect_tree_digest_entries(staging, staging, &mut entries)?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut hasher = sha2::Sha256::new();
    for (relative, kind, bytes) in entries {
        use sha2::Digest;
        hasher.update(kind.as_bytes());
        hasher.update([0]);
        hasher.update(relative.as_bytes());
        hasher.update([0]);
        hasher.update(bytes);
        hasher.update([0]);
    }
    Ok(crate::utils::checksum::finalize_sha256_hex(
        hasher.finalize(),
    ))
}

fn collect_tree_digest_entries(
    root: &Path,
    path: &Path,
    entries: &mut Vec<(String, &'static str, Vec<u8>)>,
) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect publication path: {}", path.display()))?;
    if metadata.file_type().is_symlink() {
        bail!(
            "publication staging contains a symbolic link: {}",
            path.display()
        );
    }
    if metadata.is_file() {
        let relative = path
            .strip_prefix(root)
            .context("publication tree path escaped its root")?
            .to_string_lossy()
            .replace('\\', "/");
        entries.push((
            relative,
            "file",
            fs::read(path).with_context(|| {
                format!("failed to read publication artifact: {}", path.display())
            })?,
        ));
        return Ok(());
    }
    if !metadata.is_dir() {
        bail!(
            "publication staging contains unsupported entry: {}",
            path.display()
        );
    }
    let relative = path
        .strip_prefix(root)
        .context("publication tree path escaped its root")?
        .to_string_lossy()
        .replace('\\', "/");
    if !relative.is_empty() {
        entries.push((relative, "directory", Vec::new()));
    }
    for entry in fs::read_dir(path)? {
        collect_tree_digest_entries(root, &entry?.path(), entries)?;
    }
    Ok(())
}

/// Best-effort generation read from a staged analysis manifest.
pub fn staging_generation(staging: &Path) -> Result<Option<u64>> {
    let manifest = staging.join("manifest.toml");
    match fs::read_to_string(&manifest) {
        Ok(contents) => Ok(toml::from_str::<toml::Value>(&contents)?
            .get("generation")
            .and_then(toml::Value::as_integer)
            .map(u64::try_from)
            .transpose()?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", manifest.display())),
    }
}

/// Best-effort generation read from a previously published tree.
pub fn published_generation(destination: &Path) -> Result<Option<u64>> {
    staging_generation(destination)
}

/// Records publication intent atomically before the first rename. The
/// staging digest must identify the exact bytes being published; empty
/// digests are rejected.
pub fn write_publish_intent(
    destination: &Path,
    staging_generation: Option<u64>,
    staging_digest: &str,
    previous_generation: Option<u64>,
    previous_digest: Option<&str>,
) -> Result<()> {
    if staging_digest.is_empty() {
        bail!("publish intent requires a non-empty staging digest");
    }
    let journal = PublishJournal {
        schema_version: PUBLISH_JOURNAL_SCHEMA_VERSION,
        staging_generation,
        staging_digest: staging_digest.to_string(),
        previous_generation,
        previous_digest: previous_digest.map(str::to_string),
        outcome: None,
    };
    let encoded = toml::to_string_pretty(&journal).context("failed to encode publish journal")?;
    write_atomic_sibling(&journal_path(destination), encoded.as_bytes())
}

/// Records the post-rename outcome into the journal (before journal
/// removal). `Cancelled` must only be recorded when no rename ran.
pub fn record_publish_outcome(destination: &Path, outcome: PublishOutcome) -> Result<()> {
    let path = journal_path(destination);
    let contents = fs::read_to_string(&path).with_context(|| {
        format!(
            "publish journal missing for outcome record: {}",
            path.display()
        )
    })?;
    let mut journal: PublishJournal =
        toml::from_str(&contents).context("failed to parse publish journal")?;
    journal.outcome = Some(outcome);
    let encoded = toml::to_string_pretty(&journal).context("failed to encode publish journal")?;
    write_atomic_sibling(&path, encoded.as_bytes())
}

/// Removes the journal after a fully durable commit.
pub fn clear_publish_journal(destination: &Path) -> Result<()> {
    let path = journal_path(destination);
    match fs::remove_file(&path) {
        Ok(()) => super::run_dir::sync_parent(&path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to remove publish journal: {}", path.display())),
    }
}

/// What restart recovery found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryDecision {
    /// No journal: nothing interrupted; destination (if any) is authoritative.
    Clean,
    /// Intent exists but the first rename never ran: destination keeps its
    /// generation; staging (if still present) is the unpublished attempt.
    IntentOnly { staging_generation: Option<u64> },
    /// First rename ran and the journaled staging tree was still present.
    /// Recovery completed the second rename so the destination is now
    /// authoritative.
    CompletedSecondRename {
        committed_generation: Option<u64>,
        previous_generation: Option<u64>,
    },
    /// Recovery could not prove that the surviving staging tree was the
    /// journaled attempt, so it restored the previously verified generation.
    RestoredPreviousGeneration {
        restored_generation: Option<u64>,
        attempted_generation: Option<u64>,
    },
    /// Both destination and backup present with a journal: the second rename
    /// completed but post-commit finalization did not. Destination is
    /// authoritative; the backup is left for inspection, never auto-removed.
    DestinationAuthoritativeUncertain {
        committed_generation: Option<u64>,
        previous_generation: Option<u64>,
    },
}

/// Recovers an interrupted publication. `backup` is the same path the
/// publisher moved the previous destination to, and `staging` the still-
/// present staged directory. Recovery never deletes user data: it only
/// renames a digest-matching staged tree into place or restores the verified
/// backup generation.
pub fn recover_interrupted_publish(
    destination: &Path,
    backup: &Path,
    staging: &Path,
) -> Result<RecoveryDecision> {
    let journal_file = journal_path(destination);
    let contents = match fs::read_to_string(&journal_file) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(RecoveryDecision::Clean);
        }
        Err(error) => {
            return Err(error).with_context(|| {
                format!("failed to read publish journal: {}", journal_file.display())
            });
        }
    };
    let journal: PublishJournal =
        toml::from_str(&contents).context("failed to parse publish journal")?;
    if journal.schema_version != PUBLISH_JOURNAL_SCHEMA_VERSION {
        bail!(
            "unsupported publish journal schema_version: {}",
            journal.schema_version
        );
    }
    if journal.staging_digest.is_empty() {
        bail!("publish journal carries an empty staging digest");
    }
    let destination_present = destination.is_dir();
    let backup_present = backup.is_dir();
    let staging_present = staging.is_dir();
    if journal.outcome == Some(PublishOutcome::Cancelled) {
        // Cancel-before-commit never renames: the destination generation is
        // authoritative regardless of leftover staging content.
        return Ok(RecoveryDecision::IntentOnly {
            staging_generation: journal.staging_generation,
        });
    }

    let destination_matches = destination_present
        && tree_matches_digest(destination, &journal.staging_digest, "destination")?;
    let staging_matches =
        staging_present && tree_matches_digest(staging, &journal.staging_digest, "staging")?;

    match (destination_present, backup_present, staging_present) {
        (true, false, _) if destination_matches => {
            // The second rename completed but backup cleanup/journal removal
            // did not. Do not overwrite the visible committed generation.
            Ok(RecoveryDecision::DestinationAuthoritativeUncertain {
                committed_generation: journal.staging_generation,
                previous_generation: journal.previous_generation,
            })
        }
        (true, false, _) => Ok(RecoveryDecision::IntentOnly {
            staging_generation: journal.staging_generation,
        }),
        (false, false, true) if staging_matches => {
            // There was no previous destination. Complete the only possible
            // journaled commit after proving the staged bytes.
            fs::rename(staging, destination).with_context(|| {
                format!(
                    "failed to complete interrupted publish {} -> {}",
                    staging.display(),
                    destination.display()
                )
            })?;
            super::run_dir::sync_parent(destination)?;
            Ok(RecoveryDecision::CompletedSecondRename {
                committed_generation: journal.staging_generation,
                previous_generation: journal.previous_generation,
            })
        }
        (false, true, true) if staging_matches => {
            // Kill after destination -> backup and before staging ->
            // destination. Complete the second rename only after digest proof.
            fs::rename(staging, destination).with_context(|| {
                format!(
                    "failed to complete interrupted publish {} -> {}",
                    staging.display(),
                    destination.display()
                )
            })?;
            super::run_dir::sync_parent(destination)?;
            Ok(RecoveryDecision::CompletedSecondRename {
                committed_generation: journal.staging_generation,
                previous_generation: journal.previous_generation,
            })
        }
        (false, true, _) => {
            // The staged tree is absent or belongs to another attempt. Restore
            // the old generation; never silently publish unrelated bytes.
            verify_optional_digest(backup, journal.previous_digest.as_deref(), "backup")?;
            fs::rename(backup, destination).with_context(|| {
                format!(
                    "failed to restore previous output {} -> {}",
                    backup.display(),
                    destination.display()
                )
            })?;
            super::run_dir::sync_parent(destination)?;
            Ok(RecoveryDecision::RestoredPreviousGeneration {
                restored_generation: journal.previous_generation,
                attempted_generation: journal.staging_generation,
            })
        }
        (false, false, true) => {
            bail!(
                "publish journal exists but staged tree {} does not match its recorded digest",
                staging.display()
            );
        }
        (false, false, false) => {
            bail!(
                "publish journal exists but neither destination {} nor backup {} survives",
                destination.display(),
                backup.display()
            );
        }
        (true, true, _) if destination_matches => {
            // Both trees survive: the destination is visible and matches the
            // journaled new generation. Keep the backup for inspection.
            Ok(RecoveryDecision::DestinationAuthoritativeUncertain {
                committed_generation: journal.staging_generation,
                previous_generation: journal.previous_generation,
            })
        }
        (true, true, _) => {
            bail!(
                "publish journal is ambiguous: destination {} does not match its staged digest",
                destination.display()
            );
        }
    }
}

fn tree_matches_digest(path: &Path, expected: &str, role: &str) -> Result<bool> {
    let actual = staging_tree_digest(path)
        .with_context(|| format!("failed to fingerprint {role} publication tree"))?;
    Ok(actual == expected)
}

fn verify_optional_digest(path: &Path, expected: Option<&str>, role: &str) -> Result<()> {
    if let Some(expected) = expected {
        let actual = staging_tree_digest(path)
            .with_context(|| format!("failed to fingerprint {role} publication tree"))?;
        if actual != expected {
            bail!("{role} publication tree digest mismatch: expected {expected}, got {actual}");
        }
    }
    Ok(())
}

fn write_atomic_sibling(path: &Path, contents: &[u8]) -> Result<()> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    let temporary = path.with_file_name(name);
    let result = (|| {
        fs::write(&temporary, contents)?;
        #[cfg(not(windows))]
        {
            fs::rename(&temporary, path)?;
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{
                MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
            };
            let source: Vec<u16> = temporary
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let target: Vec<u16> = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let result = unsafe {
                MoveFileExW(
                    source.as_ptr(),
                    target.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            };
            if result == 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        super::run_dir::sync_parent(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| format!("failed to write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::{
        PublishOutcome, RecoveryDecision, clear_publish_journal, journal_path,
        record_publish_outcome, recover_interrupted_publish, staging_tree_digest,
        write_publish_intent,
    };

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("pmoke_recovery_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn clean_tree_reports_clean() {
        let dir = scratch("clean");
        let destination = dir.join("analysis");
        std::fs::create_dir(&destination).unwrap();
        let decision = recover_interrupted_publish(
            &destination,
            &dir.join("analysis.backup"),
            &dir.join("analysis.staging"),
        )
        .unwrap();
        assert_eq!(decision, RecoveryDecision::Clean);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn crash_before_first_rename_keeps_destination_authoritative() {
        // Intent only: destination holds the old generation, staging holds
        // the unpublished new one. Recovery changes nothing.
        let dir = scratch("intent");
        let destination = dir.join("analysis");
        let staging = dir.join("analysis.staging");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("old.txt"), b"old").unwrap();
        std::fs::create_dir(&staging).unwrap();
        std::fs::write(staging.join("new.txt"), b"new").unwrap();
        write_publish_intent(
            &destination,
            Some(7),
            "digest-new",
            Some(6),
            Some("digest-old"),
        )
        .unwrap();
        let decision =
            recover_interrupted_publish(&destination, &dir.join("analysis.backup"), &staging)
                .unwrap();
        assert_eq!(
            decision,
            RecoveryDecision::IntentOnly {
                staging_generation: Some(7)
            }
        );
        assert_eq!(std::fs::read(destination.join("old.txt")).unwrap(), b"old");
        assert!(staging.join("new.txt").is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn crash_between_renames_completes_second_rename() {
        // Kill after destination -> backup, before staging -> destination:
        // recovery finishes the publish; the new generation is authoritative.
        let dir = scratch("between");
        let destination = dir.join("analysis");
        let backup = dir.join("analysis.backup");
        let staging = dir.join("analysis.staging");
        std::fs::create_dir(&backup).unwrap();
        std::fs::write(backup.join("old.txt"), b"old").unwrap();
        std::fs::create_dir(&staging).unwrap();
        std::fs::write(staging.join("new.txt"), b"new").unwrap();
        let staging_digest = staging_tree_digest(&staging).unwrap();
        let previous_digest = staging_tree_digest(&backup).unwrap();
        write_publish_intent(
            &destination,
            Some(7),
            &staging_digest,
            Some(6),
            Some(&previous_digest),
        )
        .unwrap();
        let decision = recover_interrupted_publish(&destination, &backup, &staging).unwrap();
        assert_eq!(
            decision,
            RecoveryDecision::CompletedSecondRename {
                committed_generation: Some(7),
                previous_generation: Some(6),
            }
        );
        assert_eq!(std::fs::read(destination.join("new.txt")).unwrap(), b"new");
        assert!(!staging.exists());
        assert!(backup.join("old.txt").is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn crash_with_both_present_keeps_destination_and_backup() {
        // Second rename done, journal removal pending: destination is
        // authoritative; the backup is retained for inspection, never
        // auto-removed by recovery.
        let dir = scratch("both");
        let destination = dir.join("analysis");
        let backup = dir.join("analysis.backup");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("new.txt"), b"new").unwrap();
        std::fs::create_dir(&backup).unwrap();
        std::fs::write(backup.join("old.txt"), b"old").unwrap();
        let staging_digest = staging_tree_digest(&destination).unwrap();
        let previous_digest = staging_tree_digest(&backup).unwrap();
        write_publish_intent(
            &destination,
            Some(7),
            &staging_digest,
            Some(6),
            Some(&previous_digest),
        )
        .unwrap();
        record_publish_outcome(&destination, PublishOutcome::UncertainDurability).unwrap();
        let decision =
            recover_interrupted_publish(&destination, &backup, &dir.join("analysis.staging"))
                .unwrap();
        assert_eq!(
            decision,
            RecoveryDecision::DestinationAuthoritativeUncertain {
                committed_generation: Some(7),
                previous_generation: Some(6),
            }
        );
        assert_eq!(std::fs::read(destination.join("new.txt")).unwrap(), b"new");
        assert!(backup.join("old.txt").is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cancel_before_commit_touches_nothing() {
        // Explicit cancel outcome: destination generation authoritative even
        // with leftover staging content; recovery deletes nothing.
        let dir = scratch("cancel");
        let destination = dir.join("analysis");
        let staging = dir.join("analysis.staging");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("old.txt"), b"old").unwrap();
        std::fs::create_dir(&staging).unwrap();
        write_publish_intent(
            &destination,
            Some(7),
            "digest-new",
            Some(6),
            Some("digest-old"),
        )
        .unwrap();
        record_publish_outcome(&destination, PublishOutcome::Cancelled).unwrap();
        let decision =
            recover_interrupted_publish(&destination, &dir.join("analysis.backup"), &staging)
                .unwrap();
        assert_eq!(
            decision,
            RecoveryDecision::IntentOnly {
                staging_generation: Some(7)
            }
        );
        assert_eq!(std::fs::read(destination.join("old.txt")).unwrap(), b"old");
        clear_publish_journal(&destination).unwrap();
        assert!(!journal_path(&destination).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn empty_digest_intent_is_rejected() {
        // An intent without identity cannot prove authoritativeness.
        let dir = scratch("digest");
        let destination = dir.join("analysis");
        std::fs::create_dir(&destination).unwrap();
        assert!(
            write_publish_intent(&destination, Some(7), "", Some(6), Some("digest-old")).is_err()
        );
        assert!(!journal_path(&destination).exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
