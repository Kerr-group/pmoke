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
            csv(input, output, cfg.force)
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

pub fn csv(input: &std::path::Path, output: &std::path::Path, force: bool) -> Result<()> {
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let temporary = forced_export_path(output);
    let (destination, replace) = if force && output.exists() {
        validate_replaceable_file(output)?;
        (temporary.as_path(), true)
    } else {
        (output, false)
    };
    let report = export_raw_waveform_csv(input, destination)?;
    if replace
        && let Err(error) = crate::commands::run_dir::replace_file_atomically(destination, output)
    {
        let _ = std::fs::remove_file(destination);
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

fn forced_export_path(output: &std::path::Path) -> std::path::PathBuf {
    let mut name = output.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.replace", std::process::id()));
    output.with_file_name(name)
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
