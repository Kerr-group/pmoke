use crate::config::Config;
use crate::config::ValidationTarget;
use crate::lockin::reference::run;
use anyhow::Result;

pub fn reference(cfg: &Config) -> Result<()> {
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock =
        crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "reference")?;
    crate::config::validate_for_target(cfg, ValidationTarget::Reference)?;
    crate::commands::run_dir::prepare_analysis_run(cfg)?;
    let result = reference_inner(cfg);
    match &result {
        Ok(()) => {
            crate::commands::run_dir::write_run_state(cfg, "published", "reference_complete", None)?
        }
        Err(error) => {
            crate::commands::run_dir::write_run_state(cfg, "failed", "reference", Some(error))?
        }
    }
    result
}

fn reference_inner(cfg: &Config) -> Result<()> {
    let staging_cfg = crate::commands::run_dir::prepare_analysis_staging(
        cfg,
        crate::commands::run_dir::AnalysisStage::Reference,
    )?;
    crate::commands::run_dir::ensure_analysis_config_snapshots(&staging_cfg)?;
    crate::commands::run_dir::write_diagnostic_config_snapshots(&staging_cfg, "reference")?;
    run(&staging_cfg)?;
    refresh_manifest_if_present(&staging_cfg, "reference")?;
    crate::commands::run_dir::publish_analysis_staging(cfg, &staging_cfg)
}

pub(crate) fn refresh_manifest_if_present(cfg: &Config, stage: &str) -> Result<()> {
    crate::lockin::provenance::refresh_analysis_manifest_outputs(cfg, stage)
}

#[cfg(test)]
mod tests {
    use super::refresh_manifest_if_present;

    #[test]
    fn missing_analysis_manifest_is_created_for_diagnostic_stages() {
        let directory = std::env::temp_dir().join(format!(
            "pmoke-reference-without-manifest-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir(&directory).unwrap();
        let mut cfg = crate::test_support::test_config(vec![1], vec![2]);
        cfg.set_artifact_root(directory.clone());
        crate::commands::run_dir::write_analysis_config_snapshots(&cfg).unwrap();
        crate::commands::run_dir::write_diagnostic_config_snapshots(&cfg, "reference").unwrap();

        refresh_manifest_if_present(&cfg, "reference").unwrap();
        let manifest = std::fs::read_to_string(cfg.paths().analysis_manifest()).unwrap();
        let manifest: toml::Value = toml::from_str(&manifest).unwrap();
        assert_eq!(manifest["schema_version"].as_integer(), Some(2));
        assert!(manifest["diagnostics"]["reference"].is_table());
        assert!(manifest["diagnostics"]["reference"]["config_sha256"].is_str());

        std::fs::remove_dir_all(directory).unwrap();
    }
}
