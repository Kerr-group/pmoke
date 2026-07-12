use crate::config::{Config, render_normalized_config};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{self, Seek, Write};
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AnalysisStage {
    Li,
    Phase,
    Kerr,
    ExportNpy,
}

pub fn prepare(cfg: &Config) -> Result<()> {
    let paths = cfg.paths();
    let root = &paths.run_dir;
    ensure_run_directory(root)?;
    let source = cfg
        .source_text
        .as_deref()
        .context("source config text is unavailable")?;
    write_once_or_verify(&paths.source_config(), source.as_bytes())?;
    let resolved = render_normalized_config(cfg).context("failed to render resolved config")?;
    write_once_or_verify(&paths.resolved_config(), resolved.as_bytes())?;
    if !paths.run_manifest().exists() {
        write_run_state(cfg, "initializing", "initializing", None)?;
    }
    sync_directory(root)
}

#[derive(Debug, Serialize, Deserialize)]
struct RunState {
    schema_version: u32,
    status: String,
    stage: String,
    pmoke_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    git_commit: Option<String>,
    started_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    acquired_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    analysis_started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    analyzed_at: Option<String>,
    updated_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    completed_at: Option<String>,
    config_source: String,
    config_resolved: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    acquisition_manifest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    analysis_manifest: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failed_stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_summary: Option<String>,
}

pub fn write_run_state(
    cfg: &Config,
    status: &str,
    stage: &str,
    error: Option<&anyhow::Error>,
) -> Result<()> {
    let paths = cfg.paths();
    ensure_run_directory(&paths.run_dir)?;
    let existing = match fs::read_to_string(paths.run_manifest()) {
        Ok(contents) => Some(
            toml::from_str::<RunState>(&contents)
                .context("failed to parse existing run manifest")?,
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).context("failed to read existing run manifest"),
    };
    let now = jiff::Timestamp::now().to_string();
    let (ex_started, ex_acquired, ex_anal_started, ex_analyzed) = match &existing {
        Some(state) => (
            Some(state.started_at.clone()),
            state.acquired_at.clone(),
            state.analysis_started_at.clone(),
            state.analyzed_at.clone(),
        ),
        None => (None, None, None, None),
    };

    let started_at = ex_started.unwrap_or_else(|| now.clone());
    let acquired_at = if status == "acquired" {
        Some(now.clone())
    } else {
        ex_acquired
    };
    let analysis_started_at = if status == "analyzing" && ex_anal_started.is_none() {
        Some(now.clone())
    } else {
        ex_anal_started
    };
    let analyzed_at = if status == "complete" {
        Some(now.clone())
    } else if status == "analyzing" {
        None
    } else {
        ex_analyzed
    };
    let completed_at = (status == "complete").then(|| now.clone());

    let state = RunState {
        schema_version: 1,
        status: status.to_string(),
        stage: stage.to_string(),
        pmoke_version: env!("CARGO_PKG_VERSION").to_string(),
        git_commit: option_env!("PMOKE_GIT_COMMIT").map(str::to_string),
        started_at,
        acquired_at,
        analysis_started_at,
        analyzed_at,
        updated_at: now,
        completed_at,
        config_source: "config.source.toml".to_string(),
        config_resolved: "config.resolved.toml".to_string(),
        acquisition_manifest: paths
            .acquisition_manifest()
            .exists()
            .then(|| "acquisition/manifest.toml".to_string()),
        analysis_manifest: paths
            .analysis_manifest()
            .exists()
            .then(|| "analysis/manifest.toml".to_string()),
        failed_stage: error.map(|_| stage.to_string()),
        error_summary: error.map(|error| error.to_string()),
    };
    let encoded = toml::to_string_pretty(&state).context("failed to encode run manifest")?;
    write_atomic_file(&paths.run_manifest(), encoded.as_bytes())
}

fn write_atomic_file(path: &Path, contents: &[u8]) -> Result<()> {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    let temporary = path.with_file_name(name);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| {
        file.write_all(contents)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        atomic_replace(&temporary, path)?;
        sync_parent(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

pub(crate) fn replace_file_atomically(source: &Path, destination: &Path) -> Result<()> {
    atomic_replace(source, destination).with_context(|| {
        format!(
            "failed to atomically replace {} with {}",
            destination.display(),
            source.display()
        )
    })?;
    sync_parent(destination)
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let result = unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

pub fn publish_staged_directory(staging: &Path, destination: &Path, force: bool) -> Result<()> {
    if !destination.exists() {
        fs::rename(staging, destination).with_context(|| {
            format!(
                "failed to publish {} as {}",
                staging.display(),
                destination.display()
            )
        })?;
        return sync_parent(destination);
    }
    if !force {
        bail!("output directory already exists: {}", destination.display());
    }

    let backup = replacement_backup_path(destination);
    if backup.exists() {
        bail!("replacement backup already exists: {}", backup.display());
    }
    fs::rename(destination, &backup).with_context(|| {
        format!(
            "failed to move existing output {} to {}",
            destination.display(),
            backup.display()
        )
    })?;
    if let Err(error) = fs::rename(staging, destination) {
        return match fs::rename(&backup, destination) {
            Ok(()) => Err(error).with_context(|| {
                format!(
                    "failed to publish staged output {}; previous output restored",
                    staging.display()
                )
            }),
            Err(rollback) => Err(error).with_context(|| {
                format!(
                    "failed to publish {}; additionally failed to restore {}: {rollback}",
                    staging.display(),
                    backup.display()
                )
            }),
        };
    }
    if let Err(error) = fs::remove_dir_all(&backup) {
        crate::ui::warn(format!(
            "analysis was published, but stale backup could not be removed: {error}"
        ));
    }
    sync_parent(destination)
}

pub(crate) fn prepare_analysis_staging(cfg: &Config, stage: AnalysisStage) -> Result<Config> {
    let mut staging_cfg = cfg.clone();
    staging_cfg.staging_active = true;
    let staging = staging_cfg.paths();
    let staging_dir = staging.analysis_dir();
    if staging_dir.exists() {
        fs::remove_dir_all(&staging_dir).with_context(|| {
            format!(
                "failed to remove incomplete analysis staging directory: {}",
                staging_dir.display()
            )
        })?;
    }
    fs::create_dir(&staging_dir).with_context(|| {
        format!(
            "failed to create analysis staging directory: {}",
            staging_dir.display()
        )
    })?;

    if stage == AnalysisStage::ExportNpy {
        let canonical_dir = cfg.paths().analysis_dir();
        if canonical_dir.exists() {
            for entry in fs::read_dir(&canonical_dir)? {
                let entry = entry?;
                let path = entry.path();
                let target = staging_dir.join(entry.file_name());
                let metadata = fs::symlink_metadata(&path)?;
                if metadata.file_type().is_dir() {
                    copy_optional_tree(&path, &target)?;
                } else if metadata.file_type().is_file() {
                    copy_file(&path, &target)?;
                }
            }
        }
        return Ok(staging_cfg);
    }

    if stage == AnalysisStage::Li {
        return Ok(staging_cfg);
    }

    let resolver = cfg.resolver();
    for &channel in cfg.phase_signal_ch() {
        copy_required_file(
            &resolver.lockin_xy_csv(channel),
            &staging.lockin_xy_csv(channel),
            "lock-in XY result",
        )?;
        copy_optional_file(
            &resolver.lockin_xy_npy(channel),
            &staging.lockin_xy_npy(channel),
        )?;
        if stage == AnalysisStage::Kerr {
            copy_required_file(
                &resolver.lockin_rotated_csv(channel),
                &staging.lockin_rotated_csv(channel),
                "phase-rotated lock-in result",
            )?;
            copy_optional_file(
                &resolver.lockin_rotated_npy(channel),
                &staging.lockin_rotated_npy(channel),
            )?;
        }
    }

    let canonical = cfg.paths();
    for source in [
        canonical.reference_plot_dir(),
        canonical.sensor_plot_dir(),
        canonical.lockin_plot_dir(),
    ] {
        if let Some(name) = source.file_name() {
            copy_optional_tree(&source, &staging.plot_dir().join(name))?;
        }
    }
    if stage == AnalysisStage::Kerr {
        copy_optional_tree(&canonical.phase_plot_dir(), &staging.phase_plot_dir())?;
    }
    copy_optional_tree(&canonical.debug_dir(), &staging.debug_dir())?;

    let manifest = resolver.analysis_manifest();
    copy_required_file(
        &manifest,
        &staging.analysis_manifest(),
        "analysis manifest (run pmoke li first)",
    )?;
    Ok(staging_cfg)
}

pub(crate) fn publish_analysis_staging(cfg: &Config, staging_cfg: &Config) -> Result<()> {
    publish_staged_directory(
        &staging_cfg.paths().analysis_dir(),
        &cfg.paths().analysis_dir(),
        true,
    )
}

pub struct AnalysisLock {
    file: fs::File,
    path: PathBuf,
}

impl AnalysisLock {
    pub fn acquire(run_dir: &Path, stage: &str) -> Result<Self> {
        let path = run_dir.join(".analysis.lock");
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("failed to open lock file: {}", path.display()))?;

        use fs2::FileExt;
        if file.try_lock_exclusive().is_err() {
            let content = fs::read_to_string(&path).unwrap_or_default();
            bail!(
                "Another analysis is currently running in this directory (lock file exists: {}).\nLock info:\n{}\n\n\
                The lock may be stale. Verify no pmoke process is running, then remove .analysis.lock.",
                path.display(),
                content
            );
        }

        file.set_len(0)?;
        file.seek(std::io::SeekFrom::Start(0))?;

        let now = jiff::Timestamp::now().to_string();
        writeln!(file, "pid = {}", std::process::id())?;
        writeln!(file, "stage = \"{stage}\"")?;
        writeln!(file, "started_at = \"{now}\"")?;
        file.sync_all()?;

        Ok(Self { file, path })
    }
}

impl Drop for AnalysisLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
        let _ = fs::remove_file(&self.path);
    }
}

fn copy_required_file(source: &Path, destination: &Path, label: &str) -> Result<()> {
    match fs::symlink_metadata(source) {
        Ok(metadata) if metadata.file_type().is_file() => copy_file(source, destination),
        Ok(_) => bail!("{label} is not a regular file: {}", source.display()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            bail!("{label} not found: {}", source.display())
        }
        Err(error) => Err(error).with_context(|| format!("failed to inspect {label}")),
    }
}

fn copy_optional_file(source: &Path, destination: &Path) -> Result<()> {
    match fs::symlink_metadata(source) {
        Ok(metadata) if metadata.file_type().is_file() => copy_file(source, destination),
        Ok(_) => bail!(
            "analysis artifact is not a regular file: {}",
            source.display()
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect analysis artifact: {}", source.display())),
    }
}

fn copy_file(source: &Path, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, destination).with_context(|| {
        format!(
            "failed to copy analysis artifact {} to {}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

fn copy_optional_tree(source: &Path, destination: &Path) -> Result<()> {
    match fs::symlink_metadata(source) {
        Ok(metadata) if metadata.file_type().is_dir() => {}
        Ok(_) => bail!(
            "analysis artifact directory is not a regular directory: {}",
            source.display()
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to inspect {}", source.display()));
        }
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let path = entry.path();
        let target = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_dir() {
            copy_optional_tree(&path, &target)?;
        } else if metadata.file_type().is_file() {
            copy_file(&path, &target)?;
        } else {
            bail!(
                "analysis artifact tree contains a symbolic link: {}",
                path.display()
            );
        }
    }
    Ok(())
}

fn replacement_backup_path(destination: &Path) -> PathBuf {
    let mut name = destination.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".backup.{}", std::process::id()));
    destination.with_file_name(name)
}

fn sync_parent(path: &Path) -> Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    sync_directory(parent)
}

fn ensure_run_directory(root: &Path) -> Result<()> {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_dir() => Ok(()),
        Ok(_) => bail!(
            "run directory is not a regular directory: {}",
            root.display()
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => fs::create_dir_all(root)
            .with_context(|| format!("failed to create run directory: {}", root.display())),
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect run directory: {}", root.display())),
    }
}

fn write_once_or_verify(path: &Path, contents: &[u8]) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.file_type().is_file() {
                bail!(
                    "run config snapshot is not a regular file: {}",
                    path.display()
                );
            }
            let existing = fs::read(path).with_context(|| {
                format!("failed to read run config snapshot: {}", path.display())
            })?;
            if existing != contents {
                bail!(
                    "run config snapshot differs from the current config: {}; choose a new --run-dir",
                    path.display()
                );
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
                .with_context(|| {
                    format!("failed to create run config snapshot: {}", path.display())
                })?;
            let result = (|| {
                file.write_all(contents).with_context(|| {
                    format!("failed to write run config snapshot: {}", path.display())
                })?;
                file.sync_all().with_context(|| {
                    format!("failed to sync run config snapshot: {}", path.display())
                })
            })();
            if result.is_err() {
                drop(file);
                let _ = fs::remove_file(path);
            }
            result
        }
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect run config snapshot: {}", path.display())),
    }
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    fs::File::open(path)
        .with_context(|| format!("failed to open run directory for sync: {}", path.display()))?
        .sync_all()
        .with_context(|| format!("failed to sync run directory: {}", path.display()))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}
static TEMP_FILE_COUNTER: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn unique_temporary_path(path: &Path) -> Result<PathBuf> {
    let pid = std::process::id();
    let counter = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{pid}.{nanos}.{counter}.replace"));
    let temp_path = path.with_file_name(name);
    if temp_path.exists() {
        bail!(
            "unique temporary path already exists: {}",
            temp_path.display()
        );
    }
    Ok(temp_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ArtifactPaths;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_directory() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("pmoke-run-dir-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn snapshot_can_be_reused_only_with_identical_contents() {
        let directory = temporary_directory();
        fs::create_dir(&directory).unwrap();
        let paths = ArtifactPaths::new(&directory);
        let path = paths.source_config();

        write_once_or_verify(&path, b"version = 4\n").unwrap();
        write_once_or_verify(&path, b"version = 4\n").unwrap();
        let error = write_once_or_verify(&path, b"version = 3\n").unwrap_err();

        assert!(error.to_string().contains("choose a new --run-dir"));
        assert_eq!(fs::read(&path).unwrap(), b"version = 4\n");
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn prepare_writes_source_and_resolved_snapshots() {
        let directory = temporary_directory();
        let mut cfg = crate::test_support::test_config(vec![1], vec![2]);
        cfg.version = 3;
        cfg.source_text = Some("version = 3\n".to_string());
        cfg.set_artifact_root(directory.clone());
        let paths = cfg.paths();

        prepare(&cfg).unwrap();
        prepare(&cfg).unwrap();

        assert_eq!(
            fs::read_to_string(paths.source_config()).unwrap(),
            "version = 3\n"
        );
        let resolved = fs::read_to_string(paths.resolved_config()).unwrap();
        assert!(resolved.starts_with("version = 3\n"));
        assert!(resolved.contains("[plot]"));
        let run: toml::Value =
            toml::from_str(&fs::read_to_string(paths.run_manifest()).unwrap()).unwrap();
        assert_eq!(run["status"].as_str(), Some("initializing"));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn force_publish_replaces_complete_directory_without_deleting_it_first() {
        let directory = temporary_directory();
        fs::create_dir(&directory).unwrap();
        let staging = directory.join("analysis.incomplete");
        let destination = directory.join("analysis");
        fs::create_dir(&staging).unwrap();
        fs::write(staging.join("new.txt"), b"new").unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("old.txt"), b"old").unwrap();

        publish_staged_directory(&staging, &destination, true).unwrap();

        assert_eq!(fs::read(destination.join("new.txt")).unwrap(), b"new");
        assert!(!destination.join("old.txt").exists());
        assert!(!staging.exists());
        assert!(!replacement_backup_path(&destination).exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn run_state_tracks_stage_failure_and_canonical_manifests() {
        let directory = temporary_directory();
        let mut cfg = crate::test_support::test_config(vec![1], vec![2]);
        cfg.set_artifact_root(directory.clone());
        prepare(&cfg).unwrap();
        fs::create_dir_all(cfg.paths().acquisition_dir()).unwrap();
        fs::write(cfg.paths().acquisition_manifest(), b"schema_version = 1\n").unwrap();
        let error = anyhow::anyhow!("network disconnected");

        write_run_state(&cfg, "failed", "fetch", Some(&error)).unwrap();

        let run: toml::Value =
            toml::from_str(&fs::read_to_string(cfg.paths().run_manifest()).unwrap()).unwrap();
        assert_eq!(run["status"].as_str(), Some("failed"));
        assert_eq!(run["failed_stage"].as_str(), Some("fetch"));
        assert_eq!(
            run["acquisition_manifest"].as_str(),
            Some("acquisition/manifest.toml")
        );
        assert_eq!(run["error_summary"].as_str(), Some("network disconnected"));
        fs::remove_dir_all(directory).unwrap();
    }
}
