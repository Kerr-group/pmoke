use crate::config::{Config, render_normalized_config};
use anyhow::{Context, Result, bail};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

const SOURCE_CONFIG_NAME: &str = "config.source.toml";
const RESOLVED_CONFIG_NAME: &str = "config.resolved.toml";

pub fn prepare(cfg: &Config) -> Result<()> {
    let root = cfg
        .artifact_root
        .as_deref()
        .context("run directory is not configured")?;
    ensure_run_directory(root)?;
    let source = cfg
        .source_text
        .as_deref()
        .context("source config text is unavailable")?;
    write_once_or_verify(&root.join(SOURCE_CONFIG_NAME), source.as_bytes())?;
    let resolved = render_normalized_config(cfg).context("failed to render resolved config")?;
    write_once_or_verify(&root.join(RESOLVED_CONFIG_NAME), resolved.as_bytes())?;
    sync_directory(root)
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

#[cfg(test)]
mod tests {
    use super::*;
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
        let path = directory.join(SOURCE_CONFIG_NAME);

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

        prepare(&cfg).unwrap();
        prepare(&cfg).unwrap();

        assert_eq!(
            fs::read_to_string(directory.join(SOURCE_CONFIG_NAME)).unwrap(),
            "version = 3\n"
        );
        let resolved = fs::read_to_string(directory.join(RESOLVED_CONFIG_NAME)).unwrap();
        assert!(resolved.starts_with("version = 3\n"));
        assert!(resolved.contains("[plot]"));
        fs::remove_dir_all(directory).unwrap();
    }
}
