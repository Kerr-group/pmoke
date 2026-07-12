use crate::config::{ArtifactPaths, ArtifactResolver, Config, LockinLpfKind};

use crate::lockin::lockin_core::{LockinProcessor, legacy_boxcar_enbw_hz};
use crate::lockin::reference::ref_analysis::RefFitParams;
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};

pub(crate) const ANALYSIS_MANIFEST_SCHEMA_VERSION: u32 = 3;

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
struct StageProvenance {
    completed_at: String,
    pmoke_version: String,
    config_sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    git_commit: Option<String>,
}

#[derive(Serialize)]
struct AnalysisMetadata<'a> {
    schema_version: u32,
    generation: u64,
    pmoke_version: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    git_commit: Option<&'static str>,
    timestamp: String,
    analyzed_at: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    stages: BTreeMap<String, StageProvenance>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_acquisition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_acquisition_sha256: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_waveform: Option<String>,
    config_source: &'static str,
    config_resolved: &'static str,
    config_source_sha256: String,
    config_resolved_sha256: String,
    config_sha256: String,
    published_through: &'static str,
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
                if matches!(
                    filename,
                    "manifest.toml" | "config.source.toml" | "config.resolved.toml"
                ) || filename.starts_with('.')
                {
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

fn validate_npy_file(path: &Path, expected_rows: usize, expected_cols: usize) -> Result<()> {
    let mut file = std::fs::File::open(path)
        .with_context(|| format!("failed to open NPY file: {}", path.display()))?;
    let mut magic = [0u8; 6];
    file.read_exact(&mut magic)
        .with_context(|| format!("failed to read NPY magic: {}", path.display()))?;
    if &magic != b"\x93NUMPY" {
        bail!("invalid NPY magic bytes: {}", path.display());
    }
    let mut version = [0u8; 2];
    file.read_exact(&mut version)
        .with_context(|| format!("failed to read NPY version: {}", path.display()))?;
    if version[0] != 1 && version[0] != 2 {
        bail!("unsupported NPY version: {}.{}", version[0], version[1]);
    }

    let header_len = if version[0] == 1 {
        let mut header_len_bytes = [0u8; 2];
        file.read_exact(&mut header_len_bytes)
            .with_context(|| format!("failed to read NPY header len: {}", path.display()))?;
        u16::from_le_bytes(header_len_bytes) as usize
    } else {
        let mut header_len_bytes = [0u8; 4];
        file.read_exact(&mut header_len_bytes)
            .with_context(|| format!("failed to read NPY header len: {}", path.display()))?;
        u32::from_le_bytes(header_len_bytes) as usize
    };

    let mut header_bytes = vec![0u8; header_len];
    file.read_exact(&mut header_bytes)
        .with_context(|| format!("failed to read NPY header dict: {}", path.display()))?;
    let header = String::from_utf8(header_bytes)
        .with_context(|| format!("NPY header is not valid UTF-8: {}", path.display()))?;

    if !header.contains("'descr': '<f8'") && !header.contains("\"descr\": \"<f8\"") {
        bail!("NPY descr is not '<f8': {}", path.display());
    }
    if header.contains("'fortran_order': True") || header.contains("\"fortran_order\": true") {
        bail!(
            "NPY is in Fortran order, expected C order: {}",
            path.display()
        );
    }

    let shape_pos = header
        .find("'shape': (")
        .or_else(|| header.find("\"shape\": ("))
        .ok_or_else(|| anyhow::anyhow!("NPY header missing shape: {}", path.display()))?;
    let start_pos = shape_pos + 10;
    let end_pos = header[start_pos..]
        .find(')')
        .ok_or_else(|| anyhow::anyhow!("invalid NPY shape format: {}", path.display()))?
        + start_pos;
    let shape_str = &header[start_pos..end_pos];
    let parts = shape_str
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    if parts.len() != 2 {
        bail!("expected 2D NPY shape, found: ({})", shape_str);
    }
    let parsed_rows: usize = parts[0]
        .parse()
        .with_context(|| format!("failed to parse NPY shape rows: {}", parts[0]))?;
    let parsed_cols: usize = parts[1]
        .parse()
        .with_context(|| format!("failed to parse NPY shape columns: {}", parts[1]))?;

    if parsed_rows != expected_rows {
        bail!(
            "NPY row count mismatch: found {parsed_rows}, expected {expected_rows} in {}",
            path.display()
        );
    }
    if parsed_cols != expected_cols {
        bail!(
            "NPY column count mismatch: found {parsed_cols}, expected {expected_cols} in {}",
            path.display()
        );
    }

    let expected_payload = expected_rows * expected_cols * 8;
    let file_metadata = path
        .metadata()
        .with_context(|| format!("failed to get metadata for NPY: {}", path.display()))?;
    let prefix_len = if version[0] == 1 { 10 } else { 12 };
    let expected_file_size = prefix_len + header_len + expected_payload;
    if file_metadata.len() as usize != expected_file_size {
        bail!(
            "NPY file size mismatch in {}: found {}, expected {expected_file_size}",
            path.display(),
            file_metadata.len()
        );
    }

    Ok(())
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
            let npy = if npy_path.exists() {
                validate_npy_file(&npy_path, rows, columns)?;
                Some(
                    npy_path
                        .strip_prefix(dir)
                        .unwrap_or(&npy_path)
                        .to_string_lossy()
                        .replace('\\', "/"),
                )
            } else {
                None
            };
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
        if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|name| name.starts_with('.'))
        {
            continue;
        }
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_dir() {
            collect_plot_files(&path, paths)?;
        } else if metadata.file_type().is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                let lower = ext.to_ascii_lowercase();
                if matches!(lower.as_str(), "png" | "pdf" | "svg") {
                    paths.push(path);
                }
            }
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

pub fn stage_config_fingerprint(cfg: &Config, stage: &str) -> Result<String> {
    let channels = analysis_channels(cfg);
    let encoded = match stage {
        "li" => serde_json::to_vec(&(
            &cfg.roles,
            &channels,
            &cfg.pulse,
            &cfg.reference,
            &cfg.lockin,
        )),
        "phase" => serde_json::to_vec(&(
            &cfg.roles,
            &channels,
            &cfg.pulse,
            &cfg.reference,
            &cfg.lockin,
            &cfg.phase,
        )),
        "kerr" => serde_json::to_vec(&(
            &cfg.roles,
            &channels,
            &cfg.pulse,
            &cfg.reference,
            &cfg.lockin,
            &cfg.phase,
            &cfg.kerr,
        )),
        _ => bail!("unknown analysis stage fingerprint: {stage}"),
    }
    .context("failed to serialize analysis stage config")?;
    Ok(crate::utils::checksum::sha256_hex(&encoded))
}

fn analysis_channels(cfg: &Config) -> Vec<&crate::config::Channel> {
    let mut channels = cfg
        .channels
        .iter()
        .filter(|channel| {
            cfg.roles.sensor_ch.contains(&channel.index)
                || cfg.roles.reference_ch == channel.index
                || cfg.roles.signal_ch.contains(&channel.index)
        })
        .collect::<Vec<_>>();
    channels.sort_by_key(|channel| channel.index);
    channels
}

pub fn validate_upstream_stage_config(cfg: &Config, stage: &str) -> Result<()> {
    crate::commands::run_dir::verify_analysis_config_snapshots(cfg)?;
    crate::commands::run_dir::verify_analysis_diagnostic_snapshots(cfg, None)?;
    let manifest = cfg.resolver().analysis_manifest();
    let contents = fs::read_to_string(&manifest)
        .with_context(|| format!("failed to read analysis manifest: {}", manifest.display()))?;
    let value: toml::Value = toml::from_str(&contents)
        .with_context(|| format!("failed to parse analysis manifest: {}", manifest.display()))?;
    let recorded = value
        .get("stages")
        .and_then(|value| value.get(stage))
        .and_then(|value| value.get("config_sha256"))
        .and_then(toml::Value::as_str)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "analysis {stage} results have no config fingerprint; run pmoke {stage} to create canonical results"
            )
        })?;
    let current = stage_config_fingerprint(cfg, stage)?;
    if recorded != current {
        bail!(
            "current config is incompatible with the published {stage} results; run pmoke {stage} before continuing"
        );
    }
    validate_upstream_artifact_checksums(cfg, &value, stage)?;
    Ok(())
}

fn validate_upstream_artifact_checksums(
    cfg: &Config,
    manifest: &toml::Value,
    stage: &str,
) -> Result<()> {
    let suffix = match stage {
        "li" => "_xy.csv",
        "phase" => "_rotated.csv",
        _ => bail!("unknown upstream analysis stage: {stage}"),
    };
    let outputs = manifest
        .get("outputs")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "analysis {stage} results have no output checksums; run pmoke {stage} before continuing"
            )
        })?;
    for channel in cfg.phase_signal_ch() {
        let relative = format!("lockin/ch{channel}{suffix}");
        let expected = outputs
            .iter()
            .find(|output| output.get("file").and_then(toml::Value::as_str) == Some(&relative))
            .and_then(|output| output.get("sha256"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "analysis {stage} output checksum is missing for {relative}; run pmoke {stage} before continuing"
                )
            })?;
        let path = cfg.paths().analysis_dir().join(&relative);
        let actual = crate::utils::checksum::file_sha256(&path).with_context(|| {
            format!(
                "failed to verify published {stage} output {}; run pmoke {stage} before continuing",
                path.display()
            )
        })?;
        if actual != expected {
            bail!(
                "published {stage} output checksum mismatch for {relative}; run pmoke {stage} before continuing"
            );
        }
    }
    Ok(())
}

fn next_generation(run_dir: &Path) -> Result<u64> {
    let manifest = ArtifactPaths::new(run_dir).analysis_manifest();
    let current = match fs::read_to_string(&manifest) {
        Ok(contents) => toml::from_str::<toml::Value>(&contents)
            .context("failed to parse current analysis generation")?
            .get("generation")
            .and_then(toml::Value::as_integer)
            .unwrap_or(0),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error).context("failed to read current analysis generation"),
    };
    let current = u64::try_from(current).context("analysis generation must not be negative")?;
    current
        .checked_add(1)
        .ok_or_else(|| anyhow::anyhow!("analysis generation overflow"))
}

pub fn write_analysis_metadata(
    cfg: &Config,
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
    let source_acquisition_sha256 = source_resolver
        .acquisition_manifest()
        .is_file()
        .then(|| crate::utils::checksum::file_sha256(&source_resolver.acquisition_manifest()))
        .transpose()?;
    let config_source_sha256 =
        crate::utils::checksum::file_sha256(&output_paths.analysis_source_config())?;
    let config_resolved_sha256 =
        crate::utils::checksum::file_sha256(&output_paths.analysis_resolved_config())?;
    let li_config_sha256 = stage_config_fingerprint(cfg, "li")?;

    let now = jiff::Timestamp::now().to_string();
    let mut stages = BTreeMap::new();
    stages.insert(
        "li".to_string(),
        StageProvenance {
            completed_at: now.clone(),
            pmoke_version: env!("CARGO_PKG_VERSION").to_string(),
            config_sha256: li_config_sha256,
            git_commit: option_env!("PMOKE_GIT_COMMIT").map(str::to_string),
        },
    );
    let has_phase = artifacts
        .iter()
        .any(|a| a.kind == "lockin_rotated" || a.kind == "phase_rotated_plot");
    let has_kerr = artifacts
        .iter()
        .any(|a| a.kind == "kerr" || a.kind == "kerr_plot");
    if has_phase {
        stages.insert(
            "phase".to_string(),
            StageProvenance {
                completed_at: now.clone(),
                pmoke_version: env!("CARGO_PKG_VERSION").to_string(),
                config_sha256: stage_config_fingerprint(cfg, "phase")?,
                git_commit: option_env!("PMOKE_GIT_COMMIT").map(str::to_string),
            },
        );
    }
    if has_kerr {
        stages.insert(
            "kerr".to_string(),
            StageProvenance {
                completed_at: now.clone(),
                pmoke_version: env!("CARGO_PKG_VERSION").to_string(),
                config_sha256: stage_config_fingerprint(cfg, "kerr")?,
                git_commit: option_env!("PMOKE_GIT_COMMIT").map(str::to_string),
            },
        );
    }

    let metadata = AnalysisMetadata {
        schema_version: ANALYSIS_MANIFEST_SCHEMA_VERSION,
        generation: next_generation(&output_paths.run_dir)?,
        pmoke_version: env!("CARGO_PKG_VERSION"),
        git_commit: option_env!("PMOKE_GIT_COMMIT"),
        timestamp: now.clone(),
        analyzed_at: now,
        stages,
        source_acquisition,
        source_acquisition_sha256,
        source_waveform,
        config_source: "config.source.toml",
        config_resolved: "config.resolved.toml",
        config_source_sha256,
        config_sha256: config_resolved_sha256.clone(),
        config_resolved_sha256,
        published_through: if has_kerr {
            "kerr"
        } else if has_phase {
            "phase"
        } else {
            "li"
        },
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

pub fn refresh_analysis_manifest_outputs(cfg: &Config, stage: &str) -> Result<()> {
    crate::commands::run_dir::verify_analysis_diagnostic_snapshots(
        cfg,
        matches!(stage, "reference" | "sensor").then_some(stage),
    )?;
    let path = cfg.paths().analysis_manifest();
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("analysis manifest has no parent directory"))?;
    let mut manifest: toml::Value = match fs::read_to_string(&path) {
        Ok(contents) => toml::from_str(&contents)
            .with_context(|| format!("failed to parse {}", path.display()))?,
        Err(error)
            if error.kind() == std::io::ErrorKind::NotFound
                && matches!(stage, "reference" | "sensor") =>
        {
            toml::Value::Table(toml::map::Map::new())
        }
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let outputs = scan_outputs(parent)?;
    let (column_sets, artifacts) = describe_analysis_artifacts(parent)?;
    let published_through = if artifacts.iter().any(|artifact| artifact.kind == "kerr") {
        Some("kerr")
    } else if artifacts
        .iter()
        .any(|artifact| artifact.kind == "lockin_rotated")
    {
        Some("phase")
    } else if artifacts
        .iter()
        .any(|artifact| artifact.kind == "lockin_xy")
    {
        Some("li")
    } else {
        None
    };
    let table = manifest
        .as_table_mut()
        .ok_or_else(|| anyhow::anyhow!("analysis manifest root must be a table"))?;
    let now = jiff::Timestamp::now().to_string();
    table.insert(
        "schema_version".to_string(),
        toml::Value::Integer(i64::from(ANALYSIS_MANIFEST_SCHEMA_VERSION)),
    );
    table.insert(
        "generation".to_string(),
        toml::Value::Integer(i64::try_from(next_generation(&cfg.paths().run_dir)?)?),
    );
    table.insert(
        "pmoke_version".to_string(),
        toml::Value::String(env!("CARGO_PKG_VERSION").to_string()),
    );
    table.insert("timestamp".to_string(), toml::Value::String(now.clone()));
    table.insert(
        "config_source".to_string(),
        toml::Value::String("config.source.toml".to_string()),
    );
    table.insert(
        "config_resolved".to_string(),
        toml::Value::String("config.resolved.toml".to_string()),
    );
    table.insert(
        "config_sha256".to_string(),
        toml::Value::String(crate::utils::checksum::file_sha256(
            &cfg.paths().analysis_resolved_config(),
        )?),
    );
    table.insert(
        "config_source_sha256".to_string(),
        toml::Value::String(crate::utils::checksum::file_sha256(
            &cfg.paths().analysis_source_config(),
        )?),
    );
    table.insert(
        "config_resolved_sha256".to_string(),
        toml::Value::String(crate::utils::checksum::file_sha256(
            &cfg.paths().analysis_resolved_config(),
        )?),
    );
    table.remove("source_config");
    let (source_acquisition, source_waveform) =
        analysis_sources(&cfg.paths().run_dir, &cfg.resolver())?;
    table.remove("source_acquisition");
    table.remove("source_acquisition_sha256");
    table.remove("source_waveform");
    if let Some(source) = source_acquisition {
        table.insert(
            "source_acquisition".to_string(),
            toml::Value::String(source),
        );
        table.insert(
            "source_acquisition_sha256".to_string(),
            toml::Value::String(crate::utils::checksum::file_sha256(
                &cfg.resolver().acquisition_manifest(),
            )?),
        );
    }
    if let Some(source) = source_waveform {
        table.insert("source_waveform".to_string(), toml::Value::String(source));
    }
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
    if let Some(published_through) = published_through {
        table.insert(
            "published_through".to_string(),
            toml::Value::String(published_through.to_string()),
        );
    } else {
        table.remove("published_through");
    }
    if stage == "export_npy" {
        table.insert(
            "exported_at".to_string(),
            toml::Value::String(jiff::Timestamp::now().to_string()),
        );
    } else if stage == "reference" || stage == "sensor" {
        table.insert(
            "plots_updated_at".to_string(),
            toml::Value::String(jiff::Timestamp::now().to_string()),
        );
    } else {
        table.insert(
            "analyzed_at".to_string(),
            toml::Value::String(jiff::Timestamp::now().to_string()),
        );
    }

    match stage {
        "li" | "phase" | "kerr" => {
            table.remove("exported_at");
        }
        "reference" | "sensor" => {}
        "export_npy" => {}
        _ => bail!("unknown analysis stage: {stage}"),
    }

    if stage != "export_npy" {
        let mut stage_prov = toml::map::Map::new();
        stage_prov.insert(
            if matches!(stage, "reference" | "sensor") {
                "updated_at"
            } else {
                "completed_at"
            }
            .to_string(),
            toml::Value::String(now),
        );
        stage_prov.insert(
            "pmoke_version".to_string(),
            toml::Value::String(env!("CARGO_PKG_VERSION").to_string()),
        );
        if matches!(stage, "li" | "phase" | "kerr") {
            stage_prov.insert(
                "config_sha256".to_string(),
                toml::Value::String(stage_config_fingerprint(cfg, stage)?),
            );
        } else if matches!(stage, "reference" | "sensor") {
            stage_prov.insert(
                "config_source".to_string(),
                toml::Value::String(format!("diagnostics/{stage}/config.source.toml")),
            );
            stage_prov.insert(
                "config_resolved".to_string(),
                toml::Value::String(format!("diagnostics/{stage}/config.resolved.toml")),
            );
            stage_prov.insert(
                "config_source_sha256".to_string(),
                toml::Value::String(crate::utils::checksum::file_sha256(
                    &cfg.paths().diagnostic_source_config(stage),
                )?),
            );
            let resolved_sha256 = crate::utils::checksum::file_sha256(
                &cfg.paths().diagnostic_resolved_config(stage),
            )?;
            stage_prov.insert(
                "config_resolved_sha256".to_string(),
                toml::Value::String(resolved_sha256.clone()),
            );
            stage_prov.insert(
                "config_sha256".to_string(),
                toml::Value::String(resolved_sha256),
            );
        }
        if let Some(git) = option_env!("PMOKE_GIT_COMMIT") {
            stage_prov.insert(
                "git_commit".to_string(),
                toml::Value::String(git.to_string()),
            );
        }
        let section = if matches!(stage, "reference" | "sensor") {
            if let Some(stages) = table.get_mut("stages").and_then(toml::Value::as_table_mut) {
                stages.remove(stage);
            }
            "diagnostics"
        } else {
            "stages"
        };
        let section = table
            .entry(section.to_string())
            .or_insert_with(|| toml::Value::Table(toml::map::Map::new()))
            .as_table_mut()
            .ok_or_else(|| anyhow::anyhow!("{section} in manifest must be a table"))?;
        if stage == "li" {
            section.remove("phase");
            section.remove("kerr");
            section.remove("export_npy");
        } else if stage == "phase" {
            section.remove("kerr");
            section.remove("export_npy");
        } else if stage == "kerr" {
            section.remove("export_npy");
        }
        section.insert(stage.to_string(), toml::Value::Table(stage_prov));
    }

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
    fn stage_fingerprints_cover_only_their_upstream_dependencies() {
        let mut cfg = crate::test_support::test_config(vec![1], vec![2]);
        let mut unused = cfg.channels[0].clone();
        unused.index = 4;
        cfg.channels.push(unused);
        let li = stage_config_fingerprint(&cfg, "li").unwrap();
        let phase = stage_config_fingerprint(&cfg, "phase").unwrap();

        let mut unused_changed = cfg.clone();
        unused_changed
            .channels
            .iter_mut()
            .find(|channel| channel.index == 4)
            .unwrap()
            .factor = Some(99.0);
        assert_eq!(stage_config_fingerprint(&unused_changed, "li").unwrap(), li);

        let mut phase_changed = cfg.clone();
        phase_changed.phase.m_omega_t0_offset = vec![0.25; 6];
        assert_eq!(stage_config_fingerprint(&phase_changed, "li").unwrap(), li);
        assert_ne!(
            stage_config_fingerprint(&phase_changed, "phase").unwrap(),
            phase
        );

        let mut kerr_changed = cfg.clone();
        kerr_changed.kerr.factor = 2.0;
        assert_eq!(
            stage_config_fingerprint(&kerr_changed, "phase").unwrap(),
            phase
        );
        assert_ne!(
            stage_config_fingerprint(&kerr_changed, "kerr").unwrap(),
            stage_config_fingerprint(&cfg, "kerr").unwrap()
        );
    }

    #[test]
    fn upstream_validation_rejects_stale_and_legacy_stage_results() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "pmoke-stage-fingerprint-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(directory.join("analysis")).unwrap();
        let mut cfg = crate::test_support::test_config(vec![1], vec![2]);
        cfg.set_artifact_root(directory.clone());
        let fingerprint = stage_config_fingerprint(&cfg, "li").unwrap();
        fs::create_dir_all(directory.join("analysis/lockin")).unwrap();
        let xy = directory.join("analysis/lockin/ch2_xy.csv");
        fs::write(&xy, b"time,value\n0,1\n").unwrap();
        let xy_sha256 = crate::utils::checksum::file_sha256(&xy).unwrap();
        fs::write(
            cfg.paths().analysis_manifest(),
            format!(
                "[stages.li]\nconfig_sha256 = \"{fingerprint}\"\n\n[[outputs]]\nfile = \"lockin/ch2_xy.csv\"\nsha256 = \"{xy_sha256}\"\n"
            ),
        )
        .unwrap();
        validate_upstream_stage_config(&cfg, "li").unwrap();

        fs::write(&xy, b"time,value\n0,2\n").unwrap();
        let error = validate_upstream_stage_config(&cfg, "li").unwrap_err();
        assert!(error.to_string().contains("checksum mismatch"));
        fs::write(&xy, b"time,value\n0,1\n").unwrap();

        let mut changed = cfg.clone();
        changed.lockin.stride_samples += 1;
        let error = validate_upstream_stage_config(&changed, "li").unwrap_err();
        assert!(error.to_string().contains("run pmoke li"));

        fs::write(
            cfg.paths().analysis_manifest(),
            "[stages.li]\ncompleted_at = \"2026-01-01T00:00:00Z\"\n",
        )
        .unwrap();
        let error = validate_upstream_stage_config(&cfg, "li").unwrap_err();
        assert!(error.to_string().contains("no config fingerprint"));
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

    #[test]
    fn test_npy_file_validation() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory =
            std::env::temp_dir().join(format!("pmoke-npy-val-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let npy_path = directory.join("test.npy");

        // Helper to write a custom NPY header
        let write_npy = |path: &Path, header_dict: &str, payload_len: usize| {
            let mut file = fs::File::create(path).unwrap();
            file.write_all(b"\x93NUMPY").unwrap();
            file.write_all(&[1, 0]).unwrap();
            let padding = (64 - ((10 + header_dict.len() + 1) % 64)) % 64;
            let header = format!("{}{}\n", header_dict, " ".repeat(padding));
            let header_len = header.len() as u16;
            file.write_all(&header_len.to_le_bytes()).unwrap();
            file.write_all(header.as_bytes()).unwrap();
            file.write_all(&vec![0u8; payload_len]).unwrap();
        };

        // 1. Valid NPY
        write_npy(
            &npy_path,
            "{'descr': '<f8', 'fortran_order': False, 'shape': (100, 3), }",
            100 * 3 * 8,
        );
        assert!(validate_npy_file(&npy_path, 100, 3).is_ok());

        // 2. Invalid magic
        let mut data = fs::read(&npy_path).unwrap();
        data[0] = b'X';
        let bad_magic_path = directory.join("bad_magic.npy");
        fs::write(&bad_magic_path, &data).unwrap();
        assert!(validate_npy_file(&bad_magic_path, 100, 3).is_err());

        // 3. Invalid descr
        let bad_descr_path = directory.join("bad_descr.npy");
        write_npy(
            &bad_descr_path,
            "{'descr': '<f4', 'fortran_order': False, 'shape': (100, 3), }",
            100 * 3 * 4,
        );
        assert!(validate_npy_file(&bad_descr_path, 100, 3).is_err());

        // 4. Invalid shape rows/cols
        assert!(validate_npy_file(&npy_path, 99, 3).is_err());
        assert!(validate_npy_file(&npy_path, 100, 4).is_err());

        // 5. Invalid file size (truncated payload)
        let truncated_path = directory.join("truncated.npy");
        write_npy(
            &truncated_path,
            "{'descr': '<f8', 'fortran_order': False, 'shape': (100, 3), }",
            100 * 3 * 8 - 1,
        );
        assert!(validate_npy_file(&truncated_path, 100, 3).is_err());

        fs::remove_dir_all(directory).unwrap();
    }
}
