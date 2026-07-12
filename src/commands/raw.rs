use crate::cli::RawCommand;
use crate::config::Config;

use crate::ui;
use crate::utils::waveform::verify_raw_waveform_dir;
use anyhow::Result;
use std::path::Path;

pub fn run(cfg: &Config, command: &RawCommand) -> Result<()> {
    match command {
        RawCommand::Verify { input } => {
            let manifest = cfg.resolver().acquisition_manifest();
            let default_path = manifest.parent().unwrap_or_else(|| Path::new("."));
            verify(input.as_deref().unwrap_or(default_path))
        }
    }
}

pub fn verify(path: &Path) -> Result<()> {
    let result = verify_raw_waveform_dir(path)?;
    ui::settings_table(
        "RAW verification",
        vec![
            ("path".to_string(), path.display().to_string()),
            (
                "metadata version".to_string(),
                result.metadata_version.to_string(),
            ),
            ("channels".to_string(), result.channel_count.to_string()),
            ("samples".to_string(), result.sample_count.to_string()),
            ("bytes".to_string(), result.total_bytes.to_string()),
            (
                "checksums".to_string(),
                if result.checksums_verified {
                    "verified"
                } else {
                    "unavailable (legacy metadata)"
                }
                .to_string(),
            ),
            (
                "config snapshots".to_string(),
                if result.config_snapshot_verified {
                    "verified"
                } else {
                    "unavailable (legacy metadata)"
                }
                .to_string(),
            ),
        ],
    );
    ui::success("RAW waveform verification completed");
    Ok(())
}
