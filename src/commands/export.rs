use crate::cli::ExportCommand;
use crate::config::Config;
use crate::constants::{FETCHED_FNAME, RAW_WAVEFORM_DIR};
use crate::ui;
use crate::utils::waveform::export_raw_waveform_csv;
use anyhow::Result;

pub fn run(cfg: &Config, command: &ExportCommand) -> Result<()> {
    match command {
        ExportCommand::Csv { input, output } => {
            let default_input = cfg.artifact_path(RAW_WAVEFORM_DIR);
            let default_output = cfg.artifact_path(FETCHED_FNAME);
            let input = input.as_deref().unwrap_or(&default_input);
            let output = output.as_deref().unwrap_or(&default_output);
            csv(input, output)
        }
    }
}

pub fn csv(input: &std::path::Path, output: &std::path::Path) -> Result<()> {
    let report = export_raw_waveform_csv(input, output)?;
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
