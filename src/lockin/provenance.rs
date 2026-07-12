use crate::config::{ArtifactPaths, ArtifactResolver, Config, LockinLpfKind};

use crate::lockin::lockin_core::{LockinProcessor, legacy_boxcar_enbw_hz};
use crate::lockin::reference::ref_analysis::RefFitParams;
use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;
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
struct OutputFileInfo {
    file: String,
    sha256: String,
}

#[derive(Serialize, PartialEq, Eq)]
struct ColumnSet {
    names: Vec<String>,
}

#[derive(Serialize)]
struct AnalysisArtifact {
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    channel: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    csv: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    npy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    column_set: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rows: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    columns: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    dtype: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    order: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    depends_on: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<String>,
}

#[derive(Serialize)]
struct ReferenceProvenance {
    channel: u8,
    frequency_hz: f64,
    amplitude: f64,
    phase_rad: f64,
}

#[derive(Serialize)]
struct AnalysisMetadata<'a> {
    schema_version: u32,
    pmoke_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    git_commit: Option<&'static str>,
    timestamp: String,
    analyzed_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_acquisition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_waveform: Option<String>,
    source_config: &'static str,
    reference: ReferenceProvenance,
    lockin: &'a LockinProvenance,
    column_sets: BTreeMap<String, ColumnSet>,
    artifacts: Vec<AnalysisArtifact>,
    outputs: Vec<OutputFileInfo>,
}

fn scan_outputs(dir: &Path) -> Result<Vec<OutputFileInfo>> {
    let mut outputs = Vec::new();
    if !dir.exists() {
        return Ok(outputs);
    }

    fn traverse(
        base_dir: &Path,
        current_dir: &Path,
        outputs: &mut Vec<OutputFileInfo>,
    ) -> Result<()> {
        for entry in std::fs::read_dir(current_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                traverse(base_dir, &path, outputs)?;
            } else if path.is_file() {
                let filename = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
                if filename == "manifest.toml" || filename.starts_with('.') {
                    continue;
                }
                let relative = path
                    .strip_prefix(base_dir)
                    .context("failed to strip prefix from output file path")?;
                let relative_str = relative
                    .to_str()
                    .ok_or_else(|| anyhow::anyhow!("non-utf8 path in output files"))?
                    .replace('\\', "/");
                let sha256 = crate::utils::checksum::file_sha256(&path)?;
                outputs.push(OutputFileInfo {
                    file: relative_str,
                    sha256,
                });
            }
        }
        Ok(())
    }

    traverse(dir, dir, &mut outputs)?;
    outputs.sort_by(|a, b| a.file.cmp(&b.file));
    Ok(outputs)
}

fn describe_analysis_artifacts(
    dir: &Path,
) -> Result<(BTreeMap<String, ColumnSet>, Vec<AnalysisArtifact>)> {
    let mut column_sets = BTreeMap::new();
    let mut artifacts = Vec::new();
    for entry in [dir.join("lockin"), dir.join("kerr")] {
        if !entry.exists() {
            continue;
        }
        for file in fs::read_dir(&entry)? {
            let path = file?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("csv") {
                continue;
            }
            let relative = path
                .strip_prefix(dir)
                .context("failed to relativize analysis CSV")?;
            let relative_string = relative.to_string_lossy().replace('\\', "/");
            let (kind, channel) = analysis_artifact_identity(relative)?;
            let (names, rows) = inspect_csv_shape(&path)?;
            let columns = names.len();
            let candidate = ColumnSet { names };
            if let Some(existing) = column_sets.get(&kind) {
                if existing != &candidate {
                    anyhow::bail!("column set differs between {kind} artifacts");
                }
            } else {
                column_sets.insert(kind.clone(), candidate);
            }
            let npy_path = path.with_extension("npy");
            let npy = npy_path.exists().then(|| {
                npy_path
                    .strip_prefix(dir)
                    .unwrap_or(&npy_path)
                    .to_string_lossy()
                    .replace('\\', "/")
            });
            artifacts.push(AnalysisArtifact {
                kind: kind.clone(),
                channel,
                csv: Some(relative_string),
                file: None,
                npy,
                column_set: Some(kind),
                rows: Some(rows),
                columns: Some(columns),
                dtype: Some("<f8"),
                order: Some("C"),
                depends_on: None,
                format: None,
            });
        }
    }
    artifacts.extend(describe_plot_artifacts(dir)?);
    artifacts.sort_by(|left, right| {
        left.csv
            .as_deref()
            .or(left.file.as_deref())
            .cmp(&right.csv.as_deref().or(right.file.as_deref()))
    });
    Ok((column_sets, artifacts))
}

fn describe_plot_artifacts(dir: &Path) -> Result<Vec<AnalysisArtifact>> {
    let plot_dir = dir.join("plots");
    if !plot_dir.exists() {
        return Ok(Vec::new());
    }
    let mut paths = Vec::new();
    collect_plot_files(&plot_dir, &mut paths)?;
    let mut artifacts = Vec::with_capacity(paths.len());
    for path in paths {
        crate::plot::validate_plot_file(&path)?;
        let relative = path.strip_prefix(dir)?;
        let file = relative.to_string_lossy().replace('\\', "/");
        let stage = relative
            .components()
            .nth(1)
            .and_then(|component| component.as_os_str().to_str())
            .ok_or_else(|| anyhow::anyhow!("invalid plot artifact path: {}", path.display()))?;
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("");
        let channel = stem
            .strip_prefix("ch")
            .and_then(|value| value.split('_').next())
            .and_then(|value| value.parse::<u8>().ok());
        let kind = match (stage, stem) {
            ("reference", _) => "reference_plot",
            ("sensor", _) => "sensor_plot",
            ("lockin", _) => "lockin_xy_plot",
            ("phase", "omega_t0") => "phase_offset_plot",
            ("phase", _) => "phase_rotated_plot",
            ("kerr", _) => "kerr_plot",
            _ => return Err(anyhow::anyhow!("unknown plot stage: {stage}")),
        };
        let depends_on = match (stage, stem) {
            ("lockin", _) => Some(match channel {
                Some(channel) => vec![format!("lockin/ch{channel}_xy.csv")],
                None => matching_analysis_csvs(dir, "_xy.csv")?,
            }),
            ("phase", "omega_t0") => Some(matching_analysis_csvs(dir, "_xy.csv")?),
            ("phase", _) => Some(match channel {
                Some(channel) => vec![format!("lockin/ch{channel}_rotated.csv")],
                None => matching_analysis_csvs(dir, "_rotated.csv")?,
            }),
            ("kerr", _) => Some(vec!["kerr/kerr.csv".to_string()]),
            _ => None,
        };
        artifacts.push(AnalysisArtifact {
            kind: kind.to_string(),
            channel,
            csv: None,
            file: Some(file),
            npy: None,
            column_set: None,
            rows: None,
            columns: None,
            dtype: None,
            order: None,
            depends_on,
            format: path
                .extension()
                .and_then(|value| value.to_str())
                .map(str::to_ascii_lowercase),
        });
    }
    Ok(artifacts)
}

fn matching_analysis_csvs(dir: &Path, suffix: &str) -> Result<Vec<String>> {
    let lockin = dir.join("lockin");
    if !lockin.exists() {
        return Ok(Vec::new());
    }
    let mut matches = fs::read_dir(lockin)?
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.ends_with(suffix))
        })
        .map(|path| {
            path.strip_prefix(dir)
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .map_err(anyhow::Error::from)
        })
        .collect::<Result<Vec<_>>>()?;
    matches.sort();
    Ok(matches)
}

fn collect_plot_files(directory: &Path, paths: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_dir() {
            collect_plot_files(&path, paths)?;
        } else if metadata.file_type().is_file() {
            paths.push(path);
        } else {
            anyhow::bail!("plot artifact is a symbolic link: {}", path.display());
        }
    }
    paths.sort();
    Ok(())
}

fn analysis_artifact_identity(path: &Path) -> Result<(String, Option<u8>)> {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            anyhow::anyhow!("analysis artifact has no UTF-8 stem: {}", path.display())
        })?;
    if stem == "kerr" {
        return Ok(("kerr".to_string(), None));
    }
    let channel = stem
        .strip_prefix("ch")
        .and_then(|value| value.split('_').next())
        .and_then(|value| value.parse::<u8>().ok())
        .ok_or_else(|| anyhow::anyhow!("invalid analysis artifact name: {}", path.display()))?;
    let kind = if stem.ends_with("_xy") {
        "lockin_xy"
    } else if stem.ends_with("_rotated") {
        "lockin_rotated"
    } else {
        return Err(anyhow::anyhow!(
            "unknown analysis artifact name: {}",
            path.display()
        ));
    };
    Ok((kind.to_string(), Some(channel)))
}

fn inspect_csv_shape(path: &Path) -> Result<(Vec<String>, usize)> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(path)
        .with_context(|| format!("failed to inspect analysis CSV: {}", path.display()))?;
    let names = reader
        .headers()?
        .iter()
        .map(str::to_string)
        .collect::<Vec<_>>();
    let mut rows = 0usize;
    for record in reader.records() {
        record.with_context(|| format!("failed to inspect analysis CSV: {}", path.display()))?;
        rows = rows
            .checked_add(1)
            .ok_or_else(|| anyhow::anyhow!("analysis CSV row count overflows"))?;
    }
    Ok((names, rows))
}

pub fn write_analysis_metadata(
    output_paths: &ArtifactPaths,
    source_resolver: &ArtifactResolver,
    reference: &RefFitParams,
    lockin: &LockinProvenance,
    cfg_roles_reference_ch: u8,
) -> Result<()> {
    let path = output_paths.analysis_manifest();
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("analysis manifest has no parent directory"))?;
    if !parent.exists() {
        fs::create_dir_all(parent).context("failed to create directory for analysis manifest")?;
    }

    let outputs = scan_outputs(parent)?;
    let (column_sets, artifacts) = describe_analysis_artifacts(parent)?;
    let (source_acquisition, source_waveform) =
        analysis_sources(&output_paths.run_dir, source_resolver)?;

    let now = jiff::Timestamp::now().to_string();
    let metadata = AnalysisMetadata {
        schema_version: 1,
        pmoke_version: env!("CARGO_PKG_VERSION"),
        git_commit: option_env!("PMOKE_GIT_COMMIT"),
        timestamp: now.clone(),
        analyzed_at: now,
        source_acquisition,
        source_waveform,
        source_config: "../config.resolved.toml",
        reference: ReferenceProvenance {
            channel: cfg_roles_reference_ch,
            frequency_hz: reference.f_ref,
            amplitude: reference.a_ref,
            phase_rad: reference.omega_tref,
        },
        lockin,
        column_sets,
        artifacts,
        outputs,
    };
    let encoded =
        toml::to_string_pretty(&metadata).context("failed to encode analysis metadata")?;
    write_atomic(&path, encoded.as_bytes())
}

pub(crate) fn analysis_sources(
    run_dir: &Path,
    resolver: &ArtifactResolver,
) -> Result<(Option<String>, Option<String>)> {
    let manifest = resolver.acquisition_manifest();
    if manifest.is_file() {
        return Ok((Some(relative_analysis_source(run_dir, &manifest)?), None));
    }
    let waveform = resolver.waveform_csv();
    if waveform.is_file() {
        return Ok((None, Some(relative_analysis_source(run_dir, &waveform)?)));
    }
    Ok((None, None))
}

fn relative_analysis_source(run_dir: &Path, source: &Path) -> Result<String> {
    let relative = source.strip_prefix(run_dir).unwrap_or(source);
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            std::path::Component::Normal(part) => parts.push(
                part.to_str()
                    .ok_or_else(|| anyhow::anyhow!("analysis source path is not UTF-8"))?,
            ),
            std::path::Component::CurDir => {}
            _ => anyhow::bail!(
                "analysis source is outside the run directory: {}",
                source.display()
            ),
        }
    }
    if parts.is_empty() {
        anyhow::bail!("analysis source path is empty: {}", source.display());
    }
    Ok(format!("../{}", parts.join("/")))
}

pub fn refresh_analysis_manifest_outputs(cfg: &Config) -> Result<()> {
    let path = cfg.paths().analysis_manifest();
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("analysis manifest has no parent directory"))?;
    let contents =
        fs::read_to_string(&path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut manifest: toml::Value =
        toml::from_str(&contents).with_context(|| format!("failed to parse {}", path.display()))?;
    let outputs = scan_outputs(parent)?;
    let (column_sets, artifacts) = describe_analysis_artifacts(parent)?;
    let table = manifest
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("analysis manifest root must be a table"))?;
    table.insert(
        "outputs".to_string(),
        toml::Value::try_from(outputs).context("failed to encode analysis output entries")?,
    );
    table.insert(
        "column_sets".to_string(),
        toml::Value::try_from(column_sets).context("failed to encode analysis column sets")?,
    );
    table.insert(
        "artifacts".to_string(),
        toml::Value::try_from(artifacts).context("failed to encode analysis artifacts")?,
    );
    table.insert(
        "analyzed_at".to_string(),
        toml::Value::String(jiff::Timestamp::now().to_string()),
    );
    let encoded =
        toml::to_string_pretty(&manifest).context("failed to encode updated analysis manifest")?;
    write_atomic(&path, encoded.as_bytes())
}

fn write_atomic(path: &Path, contents: &[u8]) -> Result<()> {
    let temporary = crate::commands::run_dir::unique_temporary_path(path)?;
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
        crate::commands::run_dir::replace_file_atomically(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LockinLpfKind;
    use std::f64::consts::PI;
    use std::time::{SystemTime, UNIX_EPOCH};

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
        let temp = crate::commands::run_dir::unique_temporary_path(output).unwrap();
        let temp_str = temp.to_string_lossy().replace('\\', "/");
        assert!(temp_str.starts_with("run/analysis_metadata.toml."));
        assert!(temp_str.ends_with(".replace"));
        assert!(temp_str.contains(&std::process::id().to_string()));
    }

    #[test]
    fn analysis_sources_record_the_artifact_that_was_actually_read() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "pmoke-analysis-sources-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(directory.join("raw_waveform")).unwrap();
        fs::write(
            directory.join("raw_waveform/metadata.toml"),
            b"version = 1\n",
        )
        .unwrap();
        let mut cfg = crate::test_support::test_config(vec![1], vec![2]);
        cfg.set_artifact_root(directory.clone());

        let sources = analysis_sources(&cfg.paths().run_dir, &cfg.resolver()).unwrap();
        assert_eq!(
            sources,
            (Some("../raw_waveform/metadata.toml".to_string()), None)
        );

        fs::remove_dir_all(directory.join("raw_waveform")).unwrap();
        fs::write(directory.join("raw.csv"), b"ch1,ch2\n").unwrap();
        let sources = analysis_sources(&cfg.paths().run_dir, &cfg.resolver()).unwrap();
        assert_eq!(sources, (None, Some("../raw.csv".to_string())));

        fs::create_dir_all(directory.join("acquisition")).unwrap();
        fs::write(
            directory.join("acquisition/manifest.toml"),
            b"schema_version = 1\n",
        )
        .unwrap();
        let sources = analysis_sources(&cfg.paths().run_dir, &cfg.resolver()).unwrap();
        assert_eq!(
            sources,
            (Some("../acquisition/manifest.toml".to_string()), None)
        );

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn manifest_describes_only_valid_canonical_plot_files() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "pmoke-plot-manifest-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(directory.join("kerr")).unwrap();
        fs::create_dir_all(directory.join("plots/kerr")).unwrap();
        fs::write(
            directory.join("kerr/kerr.csv"),
            b"time (s),Kerr angle (rad)\n0,0\n",
        )
        .unwrap();
        fs::write(directory.join("plots/kerr/kerr.png"), b"\x89PNG\r\n\x1a\n").unwrap();

        let (_, artifacts) = describe_analysis_artifacts(&directory).unwrap();
        let plot = artifacts
            .iter()
            .find(|artifact| artifact.kind == "kerr_plot")
            .unwrap();
        assert_eq!(plot.file.as_deref(), Some("plots/kerr/kerr.png"));
        assert_eq!(
            plot.depends_on.as_deref(),
            Some(&["kerr/kerr.csv".to_string()][..])
        );
        assert_eq!(plot.format.as_deref(), Some("png"));

        fs::remove_dir_all(directory).unwrap();
    }
}
