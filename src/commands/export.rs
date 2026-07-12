use crate::cli::ExportCommand;
use crate::config::Config;

use crate::ui;
use crate::utils::waveform::export_raw_waveform_csv;
use anyhow::{Context, Result};

mod npy;

pub fn run(cfg: &Config, command: &ExportCommand) -> Result<()> {
    match command {
        ExportCommand::Csv { input, output } => {
            let paths = cfg.paths();
            let manifest = cfg.resolver().acquisition_manifest();
            let default_input = manifest
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."));
            let default_output = paths.waveform_csv();
            let input = input.as_deref().unwrap_or(default_input);
            let output = output.as_deref().unwrap_or(&default_output);
            csv_with_canonical_lock(cfg, input, output, cfg.force)
        }
        ExportCommand::Npy { output } => {
            if let Some(output) = output {
                npy::export(cfg, output)
            } else {
                npy::export_canonical(cfg)
            }
        }
    }
}

pub fn csv_with_canonical_lock(
    cfg: &Config,
    input: &std::path::Path,
    output: &std::path::Path,
    force: bool,
) -> Result<()> {
    let canonical_output = cfg.paths().waveform_csv();
    if paths_equivalent(output, &canonical_output)? {
        let run_dir = &cfg.paths().run_dir;
        crate::commands::run_dir::ensure_run_directory(run_dir)?;
        let _lock = crate::commands::run_dir::RunMutationLock::acquire(run_dir, "export_csv")?;
        csv(input, output, force)
    } else {
        let resolved_output = resolve_for_comparison(output)?;
        if looks_like_canonical_waveform_path(&resolved_output) {
            anyhow::bail!(
                "output resolves to another run's canonical waveform; \
                 select that run with --run-dir or use a custom export path"
            );
        }
        csv(input, output, force)
    }
}

fn looks_like_canonical_waveform_path(output: &std::path::Path) -> bool {
    let has_canonical_suffix = || -> Option<bool> {
        if output.file_name()? != "waveform.csv" {
            return Some(false);
        }
        let waveforms = output.parent()?;
        if waveforms.file_name()? != "waveforms" {
            return Some(false);
        }
        let acquisition = waveforms.parent()?;
        if acquisition.file_name()? != "acquisition" {
            return Some(false);
        }
        Some(true)
    };
    has_canonical_suffix().unwrap_or(false)
}

fn resolve_for_comparison(p: &std::path::Path) -> Result<std::path::PathBuf> {
    let absolute = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .context("failed to get current working directory")?
            .join(p)
    };

    let mut resolved = std::path::PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::Prefix(prefix) => {
                resolved.push(prefix.as_os_str());
            }
            std::path::Component::RootDir => {
                resolved.push(component.as_os_str());
            }
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if resolved.exists() {
                    resolved = resolved.canonicalize().with_context(|| {
                        format!(
                            "failed to resolve existing path ancestor: {}",
                            resolved.display()
                        )
                    })?;
                }
                resolved.pop();
            }
            std::path::Component::Normal(name) => {
                resolved.push(name);
                if resolved.exists() {
                    resolved = resolved.canonicalize().with_context(|| {
                        format!(
                            "failed to resolve existing path ancestor: {}",
                            resolved.display()
                        )
                    })?;
                }
            }
        }
    }

    Ok(clean_path(&resolved))
}

fn paths_equivalent(a: &std::path::Path, b: &std::path::Path) -> Result<bool> {
    let resolved_a = resolve_for_comparison(a)?;
    let resolved_b = resolve_for_comparison(b)?;

    #[cfg(windows)]
    {
        let lower_a = resolved_a.to_string_lossy().to_lowercase();
        let lower_b = resolved_b.to_string_lossy().to_lowercase();
        Ok(lower_a == lower_b)
    }
    #[cfg(not(windows))]
    {
        Ok(resolved_a == resolved_b)
    }
}

fn clean_path(p: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut stack = Vec::new();
    for comp in p.components() {
        match comp {
            Component::CurDir => {}
            Component::ParentDir => {
                if let Some(last) = stack.last() {
                    match last {
                        Component::Prefix(_) | Component::RootDir => {
                            // Do not pop RootDir or Prefix
                        }
                        Component::ParentDir => {
                            stack.push(comp);
                        }
                        Component::Normal(_) => {
                            stack.pop();
                        }
                        _ => {}
                    }
                } else {
                    stack.push(comp);
                }
            }
            Component::Normal(_) | Component::RootDir | Component::Prefix(_) => {
                stack.push(comp);
            }
        }
    }
    stack.iter().collect()
}

pub fn csv(input: &std::path::Path, output: &std::path::Path, force: bool) -> Result<()> {
    if output.exists() && !force {
        anyhow::bail!(
            "output file already exists: {} (use --force to overwrite)",
            output.display()
        );
    }
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = crate::commands::run_dir::unique_temporary_path(output)?;
    let report = match export_raw_waveform_csv(input, &temporary) {
        Ok(rep) => rep,
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            return Err(error);
        }
    };
    if output.exists() {
        validate_replaceable_file(output)?;
    }
    if let Err(error) = crate::commands::run_dir::replace_file_atomically(&temporary, output) {
        let _ = std::fs::remove_file(&temporary);
        return Err(error);
    }
    ui::settings_table(
        "CSV export",
        vec![
            ("input".to_string(), input.display().to_string()),
            ("output".to_string(), output.display().to_string()),
            ("channels".to_string(), report.channel_count.to_string()),
            ("samples".to_string(), report.sample_count.to_string()),
        ],
    );
    ui::success("RAW waveform CSV export completed");
    Ok(())
}

fn validate_replaceable_file(path: &std::path::Path) -> Result<()> {
    let metadata = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.file_type().is_file(),
        "output to replace is not a regular file: {}",
        path.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{csv_with_canonical_lock, paths_equivalent};
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn test_paths_equivalent_handles_symlinks_and_normalization() {
        let dir = std::env::temp_dir().join(format!(
            "pmoke-paths-equiv-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let file_a = dir.join("a.txt");
        let file_b = dir.join("b.txt");
        let file_nonexistent = dir.join("nonexistent.txt");

        // Same physical file paths
        assert!(paths_equivalent(&file_a, &file_a).unwrap());
        // Lexically normalized equivalent
        assert!(paths_equivalent(&dir.join("./a.txt"), &file_a).unwrap());
        // Different
        assert!(!paths_equivalent(&file_a, &file_b).unwrap());
        // Non-existent paths that normalize lexically equivalent
        assert!(
            paths_equivalent(&dir.join("subdir/../nonexistent.txt"), &file_nonexistent).unwrap()
        );

        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn canonical_csv_export_uses_the_run_mutation_lock() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let run_dir = std::env::temp_dir().join(format!(
            "pmoke-canonical-export-lock-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&run_dir).unwrap();
        let lock = crate::commands::run_dir::RunMutationLock::acquire(&run_dir, "test").unwrap();
        let output = run_dir.join("acquisition/waveforms/waveform.csv");

        let mut config = crate::test_support::test_config(vec![1], vec![2]);
        config.set_artifact_root(run_dir.clone());

        let canonical = config.paths().waveform_csv();
        let error =
            csv_with_canonical_lock(&config, Path::new("missing-raw"), &output, false).unwrap_err();
        assert!(
            error.to_string().contains("run-mutating operation"),
            "Error did not contain 'run-mutating operation'.\n\
             Error: {}\n\
             output: {:?}\n\
             canonical: {:?}\n\
             resolved output: {:?}\n\
             resolved canonical: {:?}",
            error,
            output,
            canonical,
            super::resolve_for_comparison(&output),
            super::resolve_for_comparison(&canonical)
        );

        drop(lock);
        fs::remove_dir_all(run_dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn canonical_csv_alias_through_symlink_uses_lock() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let run_dir = std::env::temp_dir().join(format!("pmoke-symlink-lock-{}", nonce));
        let canonical_dir = run_dir.join("acquisition/waveforms");
        fs::create_dir_all(&canonical_dir).unwrap();

        // Create a symlink pointing to the waveforms directory
        let symlink_dir = run_dir.join("alias_waveforms");
        std::os::unix::fs::symlink(&canonical_dir, &symlink_dir).unwrap();

        // Lock is acquired on the run directory
        let lock = crate::commands::run_dir::RunMutationLock::acquire(&run_dir, "test").unwrap();

        // waveform.csv does not exist yet
        let output = symlink_dir.join("waveform.csv");

        let mut config = crate::test_support::test_config(vec![1], vec![2]);
        config.set_artifact_root(run_dir.clone());

        let error =
            csv_with_canonical_lock(&config, Path::new("missing-raw"), &output, false).unwrap_err();
        assert!(error.to_string().contains("run-mutating operation"));

        drop(lock);
        fs::remove_dir_all(run_dir).unwrap();
    }

    #[test]
    fn canonical_csv_rejects_other_run_canonical_output() {
        let run_dir_a = std::env::temp_dir().join("pmoke-other-run-a");
        let run_dir_b = std::env::temp_dir().join("pmoke-other-run-b");
        fs::create_dir_all(&run_dir_a).unwrap();
        fs::create_dir_all(&run_dir_b).unwrap();

        let output_b = run_dir_b.join("acquisition/waveforms/waveform.csv");

        let mut config = crate::test_support::test_config(vec![1], vec![2]);
        config.set_artifact_root(run_dir_a.clone());

        let error = csv_with_canonical_lock(&config, Path::new("missing-raw"), &output_b, false)
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("output resolves to another run's canonical waveform")
        );

        fs::remove_dir_all(run_dir_a).unwrap();
        fs::remove_dir_all(run_dir_b).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_other_run_canonical_csv_through_symlink() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let run_dir_a = std::env::temp_dir().join(format!("pmoke-other-symlink-a-{}", nonce));
        let run_dir_b = std::env::temp_dir().join(format!("pmoke-other-symlink-b-{}", nonce));

        // Create run_dir_b's waveforms directory
        let canonical_dir_b = run_dir_b.join("acquisition/waveforms");
        fs::create_dir_all(&canonical_dir_b).unwrap();

        // Create a symlink in run_dir_a pointing to run_dir_b's waveforms directory
        let alias_b = run_dir_a.join("alias_b");
        fs::create_dir_all(&run_dir_a).unwrap();
        std::os::unix::fs::symlink(&canonical_dir_b, &alias_b).unwrap();

        // Config is for shot A
        let mut config = crate::test_support::test_config(vec![1], vec![2]);
        config.set_artifact_root(run_dir_a.clone());

        // Target path is alias_b/waveform.csv which points to shot B's canonical path
        let output = alias_b.join("waveform.csv");

        let error =
            csv_with_canonical_lock(&config, Path::new("missing-raw"), &output, false).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("output resolves to another run's canonical waveform"),
            "Expected 'output resolves to another run's canonical waveform', got: {}",
            error
        );

        fs::remove_dir_all(run_dir_a).unwrap();
        fs::remove_dir_all(run_dir_b).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_other_run_canonical_csv_through_symlink_parent_traversal() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let run_dir_a =
            std::env::temp_dir().join(format!("pmoke-other-symlink-parent-a-{}", nonce));
        let run_dir_b =
            std::env::temp_dir().join(format!("pmoke-other-symlink-parent-b-{}", nonce));

        // Create shot_B/acquisition/waveforms/subdir
        let subdir_b = run_dir_b.join("acquisition/waveforms/subdir");
        fs::create_dir_all(&subdir_b).unwrap();

        // Create shot_A/alias pointing to subdir_b
        let alias = run_dir_a.join("alias");
        fs::create_dir_all(&run_dir_a).unwrap();
        std::os::unix::fs::symlink(&subdir_b, &alias).unwrap();

        // Config is for shot A
        let mut config = crate::test_support::test_config(vec![1], vec![2]);
        config.set_artifact_root(run_dir_a.clone());

        // Target path is alias/../waveform.csv (resolves to shot_B/acquisition/waveforms/waveform.csv)
        let output = alias.join("../waveform.csv");

        let error =
            csv_with_canonical_lock(&config, Path::new("missing-raw"), &output, false).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("output resolves to another run's canonical waveform"),
            "Expected 'output resolves to another run's canonical waveform', got: {}",
            error
        );

        fs::remove_dir_all(run_dir_a).unwrap();
        fs::remove_dir_all(run_dir_b).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn current_run_lock_acquired_through_symlink_parent_traversal() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let run_dir_a = std::env::temp_dir().join(format!("pmoke-current-symlink-a-{}", nonce));

        // Create shot_A/acquisition/waveforms/subdir
        let subdir_a = run_dir_a.join("acquisition/waveforms/subdir");
        fs::create_dir_all(&subdir_a).unwrap();

        // Create alias pointing to subdir_a
        let alias = run_dir_a.join("alias");
        std::os::unix::fs::symlink(&subdir_a, &alias).unwrap();

        // Lock is acquired on run_dir_a
        let lock = crate::commands::run_dir::RunMutationLock::acquire(&run_dir_a, "test").unwrap();

        // Config is for shot A
        let mut config = crate::test_support::test_config(vec![1], vec![2]);
        config.set_artifact_root(run_dir_a.clone());

        // Target path is alias/../waveform.csv (resolves to shot_A/acquisition/waveforms/waveform.csv)
        let output = alias.join("../waveform.csv");

        let error =
            csv_with_canonical_lock(&config, Path::new("missing-raw"), &output, false).unwrap_err();
        assert!(
            error.to_string().contains("run-mutating operation"),
            "Expected 'run-mutating operation', got: {}",
            error
        );

        drop(lock);
        fs::remove_dir_all(run_dir_a).unwrap();
    }
}
