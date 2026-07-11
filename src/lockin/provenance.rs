use crate::config::{Config, LockinLpfKind};
use crate::constants::ANALYSIS_METADATA_FNAME;
use crate::lockin::lockin_core::{LockinProcessor, legacy_boxcar_enbw_hz};
use anyhow::{Context, Result};
use serde::Serialize;
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct LockinProvenance {
    kind: LockinLpfKind,
    stride_samples: usize,
    input_sample_rate_hz: f64,
    output_sample_rate_hz: f64,
    reference_frequency_hz: f64,
    effective_window_seconds: f64,
    estimated_enbw_hz: f64,
    edge_policy: &'static str,
    base_index_start: usize,
    base_index_end: usize,
    output_index_start: usize,
    output_index_end: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    cutoff_hz: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    filter_settling_samples: Option<usize>,
}

impl LockinProvenance {
    pub fn from_processor(processor: &LockinProcessor<'_>) -> Self {
        let params = processor.params();
        let filter = processor.filter_design();
        let (base_index_start, base_index_end) = processor.base_index_range();
        let (output_index_start, output_index_end) = processor.output_index_range();
        Self {
            kind: params.lpf_kind,
            stride_samples: params.stride,
            input_sample_rate_hz: params.sample_rate,
            output_sample_rate_hz: params.output_rate,
            reference_frequency_hz: params.f_ref,
            effective_window_seconds: 2.0 * params.t_half,
            estimated_enbw_hz: filter.map_or_else(
                || legacy_boxcar_enbw_hz(params),
                |design| design.estimated_enbw_hz,
            ),
            edge_policy: "trim",
            base_index_start,
            base_index_end,
            output_index_start,
            output_index_end,
            cutoff_hz: filter.map(|design| design.cutoff_hz),
            filter_settling_samples: filter.map(|design| design.settling_samples),
        }
    }
}

#[derive(Serialize)]
struct AnalysisMetadata<'a> {
    schema_version: u32,
    pmoke_version: &'static str,
    created_at: String,
    lockin: &'a LockinProvenance,
}

pub fn write_analysis_metadata(cfg: &Config, lockin: &LockinProvenance) -> Result<()> {
    let path = cfg.artifact_path(ANALYSIS_METADATA_FNAME);
    let metadata = AnalysisMetadata {
        schema_version: 1,
        pmoke_version: env!("CARGO_PKG_VERSION"),
        created_at: jiff::Timestamp::now().to_string(),
        lockin,
    };
    let encoded =
        toml::to_string_pretty(&metadata).context("failed to encode analysis metadata")?;
    write_atomic(&path, encoded.as_bytes())
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let temporary = temporary_path(path);
    let result = (|| {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .with_context(|| format!("failed to create {}", temporary.display()))?;
        let mut writer = BufWriter::new(file);
        writer
            .write_all(contents)
            .with_context(|| format!("failed to write {}", temporary.display()))?;
        writer
            .flush()
            .with_context(|| format!("failed to flush {}", temporary.display()))?;
        writer
            .get_ref()
            .sync_all()
            .with_context(|| format!("failed to sync {}", temporary.display()))?;
        drop(writer);
        replace_file(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.tmp", std::process::id()));
    path.with_file_name(name)
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    fs::rename(source, destination).with_context(|| {
        format!(
            "failed to replace {} with {}",
            destination.display(),
            source.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LockinLpfKind;
    use std::f64::consts::PI;

    #[test]
    fn records_resolved_legacy_filter_values() {
        let mut cfg = crate::test_support::test_config(vec![1], vec![2]);
        cfg.lockin.lpf_kind = LockinLpfKind::BoxcarLegacy;
        cfg.lockin.stride_samples = 10;
        cfg.lockin.lpf_half_window_cycles = 1.0;
        cfg.lockin.lpf_cutoff_hz = None;
        let dt = 1.0e-5;
        let f_ref = 1_000.0;
        let time = (0..4_000)
            .map(|index| index as f64 * dt)
            .collect::<Vec<_>>();
        let signal = time
            .iter()
            .map(|value| (2.0 * PI * f_ref * value).sin())
            .collect::<Vec<_>>();
        let processor = LockinProcessor::new(&time, &signal, f_ref, 0.0, &cfg.lockin).unwrap();

        let provenance = LockinProvenance::from_processor(&processor);

        assert_eq!(provenance.kind, LockinLpfKind::BoxcarLegacy);
        assert_eq!(provenance.stride_samples, 10);
        assert!((provenance.input_sample_rate_hz - 100_000.0).abs() < 1.0e-8);
        assert!((provenance.output_sample_rate_hz - 10_000.0).abs() < 1.0e-8);
        assert!((provenance.reference_frequency_hz - f_ref).abs() < f64::EPSILON);
        assert!((provenance.effective_window_seconds - 0.002).abs() < f64::EPSILON);
        assert!(provenance.estimated_enbw_hz.is_finite());
        assert!(provenance.estimated_enbw_hz > 0.0);
        assert_eq!(provenance.edge_policy, "trim");
        assert_eq!(provenance.cutoff_hz, None);
        assert_eq!(provenance.filter_settling_samples, None);
    }

    #[test]
    fn temporary_metadata_is_a_process_specific_sibling() {
        let output = Path::new("run/analysis_metadata.toml");
        assert_eq!(
            temporary_path(output),
            PathBuf::from(format!(
                "run/analysis_metadata.toml.{}.tmp",
                std::process::id()
            ))
        );
    }
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<()> {
    if destination.exists() {
        fs::remove_file(destination)
            .with_context(|| format!("failed to remove {}", destination.display()))?;
    }
    fs::rename(source, destination).with_context(|| {
        format!(
            "failed to replace {} with {}",
            destination.display(),
            source.display()
        )
    })
}
