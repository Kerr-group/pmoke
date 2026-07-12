use crate::cli::ExportCommand;
use crate::config::Config;

use crate::ui;
use crate::utils::waveform::export_raw_waveform_csv;
use anyhow::Result;

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
        csv(input, output, force)
    }
}

fn paths_equivalent(a: &std::path::Path, b: &std::path::Path) -> Result<bool> {
    let clean_path = |p: &std::path::Path| -> std::path::PathBuf {
        use std::path::Component;
        let mut stack = Vec::new();
        for comp in p.components() {
            match comp {
                Component::CurDir => {}
                Component::ParentDir => {
                    stack.pop();
                }
                Component::Normal(c) => {
                    stack.push(c);
                }
                Component::RootDir => {
                    stack.push(comp.as_os_str());
                }
                Component::Prefix(prefix) => {
                    stack.push(prefix.as_os_str());
                }
            }
        }
        stack.iter().collect()
    };

    let resolve = |p: &std::path::Path| -> std::path::PathBuf {
        let abs = if p.is_absolute() {
            p.to_path_buf()
        } else {
            std::env::current_dir()
                .unwrap_or_else(|_| std::path::PathBuf::from("."))
                .join(p)
        };
        if let Ok(canon) = abs.canonicalize() {
            return canon;
        }
        clean_path(&abs)
    };

    Ok(resolve(a) == resolve(b))
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

        let error =
            csv_with_canonical_lock(&config, Path::new("missing-raw"), &output, false).unwrap_err();
        assert!(error.to_string().contains("run-mutating operation"));

        drop(lock);
        fs::remove_dir_all(run_dir).unwrap();
    }
}
