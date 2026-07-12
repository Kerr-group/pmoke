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
            csv_with_canonical_lock(input, output, cfg.force)
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
    input: &std::path::Path,
    output: &std::path::Path,
    force: bool,
) -> Result<()> {
    let Some(run_dir) = canonical_waveform_run_dir(output) else {
        return csv(input, output, force);
    };
    crate::commands::run_dir::ensure_run_directory(&run_dir)?;
    let _lock = crate::commands::run_dir::RunMutationLock::acquire(&run_dir, "export_csv")?;
    csv(input, output, force)
}

fn canonical_waveform_run_dir(output: &std::path::Path) -> Option<std::path::PathBuf> {
    if output.file_name()? != "waveform.csv" {
        return None;
    }
    let waveforms = output.parent()?;
    if waveforms.file_name()? != "waveforms" {
        return None;
    }
    let acquisition = waveforms.parent()?;
    if acquisition.file_name()? != "acquisition" {
        return None;
    }
    acquisition.parent().map(|parent| {
        if parent.as_os_str().is_empty() {
            std::path::PathBuf::from(".")
        } else {
            parent.to_path_buf()
        }
    })
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
    use super::{canonical_waveform_run_dir, csv_with_canonical_lock};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn canonical_csv_path_resolves_its_run_directory() {
        assert_eq!(
            canonical_waveform_run_dir(Path::new(
                "shot with space/acquisition/waveforms/waveform.csv"
            )),
            Some(PathBuf::from("shot with space"))
        );
        assert_eq!(
            canonical_waveform_run_dir(Path::new("exports/waveform.csv")),
            None
        );
        assert_eq!(
            canonical_waveform_run_dir(Path::new("acquisition/waveforms/waveform.csv")),
            Some(PathBuf::from("."))
        );
        assert_eq!(
            canonical_waveform_run_dir(Path::new("shot/acquisition/waveforms/custom.csv")),
            None
        );
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

        let error = csv_with_canonical_lock(Path::new("missing-raw"), &output, false).unwrap_err();
        assert!(error.to_string().contains("run-mutating operation"));

        drop(lock);
        fs::remove_dir_all(run_dir).unwrap();
    }
}
