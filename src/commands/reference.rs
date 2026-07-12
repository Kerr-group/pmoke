use crate::config::Config;
use crate::config::ValidationTarget;
use crate::lockin::reference::run;
use anyhow::{Context, Result};
use std::{fs, io};

pub fn reference(cfg: &Config) -> Result<()> {
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock =
        crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "reference")?;
    crate::config::validate_for_target(cfg, ValidationTarget::Reference)?;
    crate::commands::run_dir::prepare(cfg)?;

    let staging_cfg = crate::commands::run_dir::prepare_analysis_staging(
        cfg,
        crate::commands::run_dir::AnalysisStage::Reference,
    )?;
    run(&staging_cfg)?;
    refresh_manifest_if_present(&staging_cfg, "reference")?;
    crate::commands::run_dir::publish_analysis_staging(cfg, &staging_cfg)
}

pub(crate) fn refresh_manifest_if_present(cfg: &Config, stage: &str) -> Result<()> {
    let manifest = cfg.paths().analysis_manifest();
    match fs::symlink_metadata(&manifest) {
        Ok(metadata) if metadata.file_type().is_file() => {
            crate::lockin::provenance::refresh_analysis_manifest_outputs(cfg, stage)
        }
        Ok(_) => anyhow::bail!(
            "analysis manifest must be a regular file: {}",
            manifest.display()
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| {
            format!(
                "failed to inspect analysis manifest: {}",
                manifest.display()
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::refresh_manifest_if_present;

    #[test]
    fn missing_analysis_manifest_is_valid_for_diagnostic_stages() {
        let directory = std::env::temp_dir().join(format!(
            "pmoke-reference-without-manifest-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir(&directory).unwrap();
        let mut cfg = crate::test_support::test_config(vec![1], vec![2]);
        cfg.set_artifact_root(directory.clone());

        refresh_manifest_if_present(&cfg, "reference").unwrap();
        refresh_manifest_if_present(&cfg, "sensor").unwrap();

        std::fs::remove_dir_all(directory).unwrap();
    }
}
