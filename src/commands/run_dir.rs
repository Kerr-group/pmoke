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
    Reference,
    Sensor,
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

    if matches!(
        stage,
        AnalysisStage::Reference | AnalysisStage::Sensor | AnalysisStage::ExportNpy
    ) {
        copy_optional_tree(&cfg.paths().analysis_dir(), &staging_dir)?;
        match stage {
            AnalysisStage::Reference => {
                remove_optional_tree(&staging.reference_plot_dir())?;
            }
            AnalysisStage::Sensor => {
                remove_optional_tree(&staging.sensor_plot_dir())?;
            }
            _ => {}
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

fn remove_optional_tree(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_dir() => fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove stale artifact tree: {}", path.display())),
        Ok(_) => bail!(
            "artifact tree is not a regular directory: {}",
            path.display()
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error)
            .with_context(|| format!("failed to inspect artifact tree: {}", path.display())),
    }
}

pub(crate) fn publish_analysis_staging(cfg: &Config, staging_cfg: &Config) -> Result<()> {
    publish_staged_directory(
        &staging_cfg.paths().analysis_dir(),
        &cfg.paths().analysis_dir(),
        true,
    )
}

pub struct RunMutationLock {
    file: fs::File,
}

impl RunMutationLock {
    pub fn acquire(run_dir: &Path, stage: &str) -> Result<Self> {
        let path = run_dir.join(".run.lock");
        let mut file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .with_context(|| format!("failed to open lock file: {}", path.display()))?;

        use fs2::FileExt;
        match file.try_lock_exclusive() {
            Ok(()) => {}
            Err(error) => {
                let is_lock_collision = {
                    #[cfg(windows)]
                    {
                        error.raw_os_error() == Some(32) || error.raw_os_error() == Some(33)
                    }
                    #[cfg(not(windows))]
                    {
                        false
                    }
                } || error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::PermissionDenied;

                if is_lock_collision {
                    let content = fs::read_to_string(&path).unwrap_or_default();
                    bail!(
                        "another run-mutating operation is already running in this directory (lock file: {}).\nLock info:\n{}",
                        path.display(),
                        content
                    );
                } else {
                    return Err(error).with_context(|| {
                        format!("failed to acquire run mutation lock: {}", path.display())
                    });
                }
            }
        }

        file.set_len(0)?;
        file.seek(std::io::SeekFrom::Start(0))?;

        let now = jiff::Timestamp::now().to_string();
        writeln!(file, "pid = {}", std::process::id())?;
        writeln!(file, "stage = \"{stage}\"")?;
        writeln!(file, "started_at = \"{now}\"")?;
        file.sync_all()?;

        Ok(Self { file })
    }
}

impl Drop for RunMutationLock {
    fn drop(&mut self) {
        let _ = fs2::FileExt::unlock(&self.file);
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

pub(crate) fn ensure_run_directory(root: &Path) -> Result<()> {
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

    #[test]
    fn diagnostic_plot_stages_replace_only_their_owned_plot_trees() {
        let directory = temporary_directory();
        let mut cfg = crate::test_support::test_config(vec![1], vec![2]);
        cfg.set_artifact_root(directory.clone());
        let paths = cfg.paths();
        for file in [
            paths.reference_plot_dir().join("old.png"),
            paths.sensor_plot_dir().join("old.png"),
            paths.lockin_plot_dir().join("old.png"),
        ] {
            fs::create_dir_all(file.parent().unwrap()).unwrap();
            fs::write(file, b"old").unwrap();
        }

        let reference = prepare_analysis_staging(&cfg, AnalysisStage::Reference).unwrap();
        assert!(!reference.paths().reference_plot_dir().exists());
        assert!(
            reference
                .paths()
                .sensor_plot_dir()
                .join("old.png")
                .is_file()
        );
        assert!(
            reference
                .paths()
                .lockin_plot_dir()
                .join("old.png")
                .is_file()
        );
        fs::remove_dir_all(reference.paths().analysis_dir()).unwrap();

        let sensor = prepare_analysis_staging(&cfg, AnalysisStage::Sensor).unwrap();
        assert!(
            sensor
                .paths()
                .reference_plot_dir()
                .join("old.png")
                .is_file()
        );
        assert!(!sensor.paths().sensor_plot_dir().exists());
        assert!(sensor.paths().lockin_plot_dir().join("old.png").is_file());

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn test_analysis_lock_exclusive_advisory() {
        let directory = temporary_directory();
        let run_dir = &directory;
        fs::create_dir_all(run_dir).unwrap();

        // 1. Process A acquires lock
        let lock_a = RunMutationLock::acquire(run_dir, "stage_a").unwrap();

        // 2. Process B try to acquire lock (should fail)
        let lock_b_err = RunMutationLock::acquire(run_dir, "stage_b");
        assert!(lock_b_err.is_err());

        // 3. Drop A, then B acquires lock successfully
        std::mem::drop(lock_a);
        let lock_b = RunMutationLock::acquire(run_dir, "stage_b");
        assert!(lock_b.is_ok());

        // 4. lock file still exists
        let lock_path = run_dir.join(".run.lock");
        assert!(lock_path.exists());

        // 5. Drop B
        std::mem::drop(lock_b);

        // 6. Stress test with multiple threads trying to acquire lock
        use std::sync::{Arc, Mutex};
        use std::thread;

        let run_dir_arc = Arc::new(run_dir.clone());
        let active_count = Arc::new(Mutex::new(0));
        let max_concurrency = Arc::new(Mutex::new(0));
        let mut threads = Vec::new();

        for i in 0..10 {
            let run_dir_c = Arc::clone(&run_dir_arc);
            let active_count_c = Arc::clone(&active_count);
            let max_concurrency_c = Arc::clone(&max_concurrency);
            threads.push(thread::spawn(move || {
                for _ in 0..20 {
                    if let Ok(_lock) =
                        RunMutationLock::acquire(&run_dir_c, &format!("thread_{}", i))
                    {
                        {
                            let mut active = active_count_c.lock().unwrap();
                            *active += 1;
                            let mut max_c = max_concurrency_c.lock().unwrap();
                            if *active > *max_c {
                                *max_c = *active;
                            }
                            assert!(
                                *active <= 1,
                                "Exclusion violated! Concurrency count: {}",
                                *active
                            );
                        }
                        thread::sleep(std::time::Duration::from_millis(2));
                        {
                            let mut active = active_count_c.lock().unwrap();
                            *active -= 1;
                        }
                    }
                    thread::sleep(std::time::Duration::from_millis(1));
                }
            }));
        }

        for t in threads {
            t.join().unwrap();
        }

        let max_observed = *max_concurrency.lock().unwrap();
        assert!(max_observed <= 1);

        // 7. Verify lock file can be reacquired without manual deletion
        let lock_final = RunMutationLock::acquire(run_dir, "final");
        assert!(lock_final.is_ok());

        fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn test_diagnostic_stages_manifest_behavior_and_no_side_effects() {
        unsafe {
            std::env::set_var("MPLBACKEND", "agg");
        }
        let directory = temporary_directory();
        let run_dir = &directory;
        fs::create_dir_all(run_dir).unwrap();

        // 1. Setup acquisition waveforms
        let waveforms_dir = run_dir.join("acquisition/waveforms");
        fs::create_dir_all(&waveforms_dir).unwrap();
        let csv_path = waveforms_dir.join("waveform.csv");
        let mut csv_content = String::from("t,ch1,ch2\n");
        let dt = 1e-9;
        let f = 10.0e6;
        for i in 0..200 {
            let t = i as f64 * dt;
            let ch1 = (2.0 * std::f64::consts::PI * f * t).sin();
            let ch2 = (2.0 * std::f64::consts::PI * f * t + 0.5).sin();
            csv_content.push_str(&format!("{:.12},{:.12},{:.12}\n", t, ch1, ch2));
        }
        fs::write(&csv_path, csv_content).unwrap();

        // 2. Setup initial analysis directory with manifest
        let analysis_dir = run_dir.join("analysis");
        let plots_dir = analysis_dir.join("plots");
        let ref_plots_dir = plots_dir.join("reference");
        let sensor_plots_dir = plots_dir.join("sensor");
        fs::create_dir_all(&ref_plots_dir).unwrap();
        fs::create_dir_all(&sensor_plots_dir).unwrap();

        // Write dummy plots (must have valid PNG signature)
        let ref_plot_file = ref_plots_dir.join("reference_fit.png");
        let sensor_plot_file = sensor_plots_dir.join("sensor_fit.png");
        let dummy_png = [137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 0];
        fs::write(&ref_plot_file, dummy_png).unwrap();
        fs::write(&sensor_plot_file, dummy_png).unwrap();

        let initial_manifest = r#"schema_version = 1
source_acquisition = "../acquisition/manifest.toml"
analyzed_at = "2026-07-13T00:00:00Z"
exported_at = "2026-07-13T00:05:00Z"

[stages.li]
completed_at = "2026-07-13T00:00:00Z"
pmoke_version = "0.1.0"

[stages.phase]
completed_at = "2026-07-13T00:01:00Z"
pmoke_version = "0.1.0"

[stages.kerr]
completed_at = "2026-07-13T00:02:00Z"
pmoke_version = "0.1.0"

[stages.sensor]
completed_at = "2026-07-13T00:03:00Z"
pmoke_version = "0.1.0"

[[artifacts]]
kind = "reference_plot"
file = "plots/reference/reference_fit.png"
sha256 = "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"

[[artifacts]]
kind = "sensor_plot"
file = "plots/sensor/sensor_fit.png"
sha256 = "2f0cde9b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088fa52e"
"#;
        let manifest_path = analysis_dir.join("manifest.toml");
        fs::write(&manifest_path, initial_manifest).unwrap();

        // 3. Create test Config
        let mut cfg = crate::test_support::test_config(vec![2], vec![1]);
        cfg.set_artifact_root(run_dir.clone());
        cfg.reference.fft_window.start = 0.0;
        cfg.reference.fft_window.end = 200e-9;
        cfg.reference.stride_samples = 2;
        cfg.reference.window_samples = 10;
        let window_bg = crate::config::Window {
            start: 0.0,
            end: 50.0e-9,
        };
        cfg.pulse.bg_window_before = window_bg;
        cfg.pulse.bg_window_after = window_bg;
        cfg.plot.enabled = false;
        cfg.instruments = Some(crate::config::Instruments {
            function_generator: None,
            oscilloscope: crate::config::Oscilloscope {
                connection: crate::config::Connection::Tcpip {
                    ip: "127.0.0.1".to_string(),
                    port: 80,
                },
                model: "dummy".to_string(),
            },
        });

        // Save reference plot hash before sensor execution
        let ref_plot_hash_before = crate::utils::checksum::file_sha256(&ref_plot_file).unwrap();

        // 4. Run standalone sensor
        crate::commands::sensor::sensor(&cfg).unwrap();

        // A. Verify sensor command did NOT overwrite/delete/modify the reference plot!
        let ref_plot_hash_after_sensor =
            crate::utils::checksum::file_sha256(&ref_plot_file).unwrap();
        assert_eq!(
            ref_plot_hash_before, ref_plot_hash_after_sensor,
            "standalone sensor run modified reference plot!"
        );
        // The sensor plot was deleted in staging and not regenerated since plot.enabled = false
        assert!(!sensor_plot_file.exists());

        // B. Verify manifest state after sensor execution
        let manifest_content = fs::read_to_string(&manifest_path).unwrap();
        let manifest: toml::Value = toml::from_str(&manifest_content).unwrap();

        assert_eq!(
            manifest.get("analyzed_at").unwrap().as_str().unwrap(),
            "2026-07-13T00:00:00Z"
        );
        assert_eq!(
            manifest.get("exported_at").unwrap().as_str().unwrap(),
            "2026-07-13T00:05:00Z"
        );

        let stages = manifest.get("stages").unwrap().as_table().unwrap();
        assert!(stages.contains_key("li"));
        assert!(stages.contains_key("phase"));
        assert!(stages.contains_key("kerr"));
        assert!(stages.contains_key("sensor"));
        // stages.sensor should be updated (not 2026-07-13T00:03:00Z anymore)
        let sensor_stage = stages.get("sensor").unwrap().as_table().unwrap();
        assert_ne!(
            sensor_stage.get("completed_at").unwrap().as_str().unwrap(),
            "2026-07-13T00:03:00Z"
        );

        // C. Run standalone reference
        // First recreate sensor plot in analysis/ to verify reference doesn't touch it
        fs::create_dir_all(sensor_plot_file.parent().unwrap()).unwrap();
        fs::write(&sensor_plot_file, dummy_png).unwrap();
        let sensor_plot_hash_before_ref =
            crate::utils::checksum::file_sha256(&sensor_plot_file).unwrap();

        crate::commands::reference::reference(&cfg).unwrap();

        // D. Verify reference command did NOT overwrite/delete/modify sensor plot
        let sensor_plot_hash_after_ref =
            crate::utils::checksum::file_sha256(&sensor_plot_file).unwrap();
        assert_eq!(
            sensor_plot_hash_before_ref, sensor_plot_hash_after_ref,
            "standalone reference run modified sensor plot!"
        );
        // The reference plot was deleted in staging and not regenerated since plot.enabled = false
        assert!(!ref_plot_file.exists());

        // F. Verify manifest updates after reference execution
        let manifest_content_2 = fs::read_to_string(&manifest_path).unwrap();
        let manifest_2: toml::Value = toml::from_str(&manifest_content_2).unwrap();

        assert_eq!(
            manifest_2.get("analyzed_at").unwrap().as_str().unwrap(),
            "2026-07-13T00:00:00Z"
        );
        assert!(manifest_2.get("plots_updated_at").is_some());

        let stages_2 = manifest_2.get("stages").unwrap().as_table().unwrap();
        assert!(stages_2.contains_key("li"));
        assert!(stages_2.contains_key("phase"));
        assert!(stages_2.contains_key("kerr"));
        assert!(stages_2.contains_key("reference"));
        assert!(stages_2.contains_key("sensor"));

        fs::remove_dir_all(run_dir).unwrap();
    }
}
