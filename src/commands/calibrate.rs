//! Recorded-only calibration build/inspect/validate (R1, Issue #232).
//!
//! Pure core kernels (`pmoke-analysis-core::calibration`) exposed as native
//! commands over recorded waveform CSV data. No instrument access: every
//! entry point reads explicit file paths and publishes into a NEW output
//! directory, never into a source run tree.
//!
//! - `build`: recorded CSV + TOML request -> immutable artifact JSON bytes
//!   plus SHA-256 identity, published under a new destination.
//! - `inspect`: structural validation + human-readable summary of an
//!   artifact file (read-only).
//! - `validate`: applicability of an artifact file against a declared
//!   inference context (read-only).

use anyhow::{Context, Result, bail};
use pmoke_analysis_core::calibration::{
    AcquisitionMeta, AdequacyPolicy, ApplicabilityRequest, ArtifactRequest, BlockPlanRequest,
    CALIBRATION_ALGORITHM_VERSION, CALIBRATION_ARTIFACT_SCHEMA_VERSION,
    CALIBRATION_PHASE_CONVENTION, CalibrationArtifact, CalibrationRole, CorrelationRecipe,
    HeldoutReport, ModelBinding, PhaseVarianceRecipe, RoleInterval, TuningMode, assemble_samples,
    build_artifact, estimate_correlation, estimate_phase_variance, fit_nuisance,
    inspect_applicability, plan_blocks, scs_adequacy,
};
use pmoke_analysis_core::joint::JointSolverTolerances;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Calibration build request schema version.
pub const CALIBRATE_REQUEST_SCHEMA_VERSION: u32 = 1;

/// Versioned build request (deny_unknown_fields: no silent options).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrateRequest {
    pub schema_version: u32,
    pub model_id: String,
    pub waveform_csv: String,
    pub channel_column: usize,
    pub time_column: Option<usize>,
    pub sample_interval_s: f64,
    pub reference_frequency_hz: f64,
    pub reference_phase_rad: f64,
    pub channel: u32,
    pub seed: u64,
    pub block_len: usize,
    pub intervals: Vec<CalibrateInterval>,
    pub phase_recipe: Option<PhaseVarianceRecipeToml>,
    pub correlation_recipe: Option<CorrelationRecipeToml>,
    pub heldout: Option<HeldoutToml>,
    pub output: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrateInterval {
    pub role: CalibrationRoleToml,
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationRoleToml {
    Training,
    TuningValidation,
    Evaluation,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseVarianceRecipeToml {
    pub bins: Option<usize>,
    pub min_samples_per_bin: Option<usize>,
    pub min_cycles_per_bin: Option<usize>,
    pub min_contributing_blocks: Option<usize>,
    pub shrinkage_alpha: Option<f64>,
    pub floor_ratio: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CorrelationRecipeToml {
    pub max_lag: Option<usize>,
    pub shrinkage_eta: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeldoutToml {
    pub blocks: Option<usize>,
    pub profile_rmse_v2: Option<f64>,
    pub standardized_lag1: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CalibrateReport {
    pub schema_version: u32,
    pub model_id: String,
    pub artifact: String,
    pub sha256: String,
    pub bytes: usize,
    pub modes: Vec<String>,
    pub training_blocks: usize,
    pub adequate: bool,
    pub adequacy_reason: String,
}

fn role_of(role: CalibrationRoleToml) -> CalibrationRole {
    match role {
        CalibrationRoleToml::Training => CalibrationRole::Training,
        CalibrationRoleToml::TuningValidation => CalibrationRole::TuningValidation,
        CalibrationRoleToml::Evaluation => CalibrationRole::Evaluation,
    }
}

/// Refuses instrument-backed configs: calibration builds only from explicit
/// recorded files, never from live acquisition paths.
fn refuse_live_source(request_dir: &Path, waveform: &str) -> Result<PathBuf> {
    if waveform.contains("..") {
        bail!("calibration waveform path must not contain '..': {waveform}");
    }
    let path = request_dir.join(waveform);
    if !path.is_file() {
        bail!("calibration waveform does not exist: {}", path.display());
    }
    Ok(path)
}

fn ensure_new_dir(path: &Path) -> Result<()> {
    if path.exists() {
        bail!(
            "calibration output already exists (no overwrite by default): {}",
            path.display()
        );
    }
    std::fs::create_dir_all(path)
        .with_context(|| format!("cannot create calibration output: {}", path.display()))?;
    Ok(())
}

fn load_request(request_path: &Path) -> Result<(CalibrateRequest, PathBuf)> {
    let request_dir = request_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let text = std::fs::read_to_string(request_path).with_context(|| {
        format!(
            "cannot read calibration request: {}",
            request_path.display()
        )
    })?;
    let request: CalibrateRequest = toml::from_str(&text)
        .with_context(|| format!("invalid calibration request: {}", request_path.display()))?;
    if request.schema_version != CALIBRATE_REQUEST_SCHEMA_VERSION {
        bail!(
            "unsupported calibration request schema_version {} (expected {CALIBRATE_REQUEST_SCHEMA_VERSION})",
            request.schema_version
        );
    }
    Ok((request, request_dir))
}

/// Reads one recorded channel (plus optional time column) from a CSV file
/// with a header row. Returns (times_or_synthetic, signal).
fn read_recorded_channel(
    path: &Path,
    channel_column: usize,
    time_column: Option<usize>,
    sample_interval_s: f64,
) -> Result<(Vec<f64>, Vec<f64>)> {
    let columns = crate::utils::csv::read_csv(path)?;
    if columns.is_empty() {
        bail!("calibration waveform holds no columns: {}", path.display());
    }
    let rows = columns[0].len();
    let get = |index: usize| {
        columns.get(index).ok_or_else(|| {
            anyhow::anyhow!(
                "calibration waveform column {index} out of range ({} columns)",
                columns.len()
            )
        })
    };
    let signal = get(channel_column)?.clone();
    let times = match time_column {
        Some(index) => get(index)?.clone(),
        None => (0..rows).map(|i| i as f64 * sample_interval_s).collect(),
    };
    if times.len() != signal.len() {
        bail!("calibration waveform time and signal lengths disagree");
    }
    Ok((times, signal))
}

/// Builds an artifact from a recorded waveform CSV and a TOML request.
pub fn run_build(request_path: &Path, output_override: Option<&Path>) -> Result<()> {
    let (request, request_dir) = load_request(request_path)?;
    if request.model_id.trim().is_empty() {
        bail!("calibration request needs a non-empty model_id");
    }
    if !(request.sample_interval_s.is_finite() && request.sample_interval_s > 0.0) {
        bail!("calibration request needs a positive finite sample_interval_s");
    }
    if !(request.reference_frequency_hz.is_finite() && request.reference_frequency_hz > 0.0) {
        bail!("calibration request needs a positive finite reference_frequency_hz");
    }
    if !request.reference_phase_rad.is_finite() {
        bail!("calibration request needs a finite reference_phase_rad");
    }
    if request.intervals.is_empty() {
        bail!("calibration request lists no role intervals");
    }
    let waveform = refuse_live_source(&request_dir, &request.waveform_csv)?;
    let (times, signal) = read_recorded_channel(
        &waveform,
        request.channel_column,
        request.time_column,
        request.sample_interval_s,
    )?;
    let total = times.len() as u64;
    if total == 0 {
        bail!("calibration waveform holds no samples");
    }

    let role_intervals: Vec<RoleInterval> = request
        .intervals
        .iter()
        .map(|interval| RoleInterval {
            role: role_of(interval.role),
            start: interval.start,
            end: interval.end,
        })
        .collect();
    let plan_request = BlockPlanRequest {
        intervals: role_intervals,
        block_len: request.block_len,
        total_samples: total,
        reference_frequency_hz: request.reference_frequency_hz,
        sample_interval_s: request.sample_interval_s,
        ..BlockPlanRequest::default()
    };
    let plan = plan_blocks(&plan_request)?;
    let training: Vec<_> = plan
        .blocks
        .iter()
        .filter(|block| block.role == CalibrationRole::Training)
        .collect();
    if training.is_empty() {
        bail!("calibration plan holds no training blocks");
    }
    let sample_rate_hz = 1.0 / request.sample_interval_s;
    let tolerances = JointSolverTolerances::default();
    let mut block_times = Vec::with_capacity(training.len());
    let mut block_residuals = Vec::with_capacity(training.len());
    for block in &training {
        let (start, end) = (block.start as usize, block.end as usize);
        let fit = fit_nuisance(
            &times[start..end],
            &signal[start..end],
            request.reference_frequency_hz,
            request.reference_phase_rad,
            sample_rate_hz,
            tolerances,
        )?;
        block_times.push(times[start..end].to_vec());
        block_residuals.push(fit.residual);
    }
    let samples = assemble_samples(
        &block_times,
        &block_residuals,
        request.reference_frequency_hz,
        request.reference_phase_rad,
    )?;
    let phase_toml = request.phase_recipe.unwrap_or(PhaseVarianceRecipeToml {
        bins: None,
        min_samples_per_bin: None,
        min_cycles_per_bin: None,
        min_contributing_blocks: None,
        shrinkage_alpha: None,
        floor_ratio: None,
    });
    let phase_defaults = PhaseVarianceRecipe::default();
    // CLI default bins (8) differ from the library default (64): the CLI
    // builds from a single recorded channel with modest block counts, and
    // 64 bins would starve per-bin cycle coverage. Explicit requests win.
    let phase_recipe = PhaseVarianceRecipe {
        bins: phase_toml.bins.unwrap_or(8),
        min_samples_per_bin: phase_toml
            .min_samples_per_bin
            .unwrap_or(phase_defaults.min_samples_per_bin),
        min_cycles_per_bin: phase_toml
            .min_cycles_per_bin
            .unwrap_or(phase_defaults.min_cycles_per_bin),
        min_contributing_blocks: phase_toml
            .min_contributing_blocks
            .unwrap_or(phase_defaults.min_contributing_blocks),
        shrinkage_alpha: phase_toml
            .shrinkage_alpha
            .unwrap_or(phase_defaults.shrinkage_alpha),
        floor_ratio: phase_toml.floor_ratio.unwrap_or(phase_defaults.floor_ratio),
    };
    let variance = estimate_phase_variance(&samples, phase_recipe)?;

    let (correlation, correlation_recipe) = match request.correlation_recipe {
        Some(recipe_toml) => {
            let recipe_defaults = CorrelationRecipe::default();
            let recipe = CorrelationRecipe {
                max_lag: recipe_toml.max_lag.unwrap_or(recipe_defaults.max_lag),
                shrinkage_eta: recipe_toml
                    .shrinkage_eta
                    .unwrap_or(recipe_defaults.shrinkage_eta),
            };
            let standardized: Vec<Vec<f64>> = block_residuals
                .iter()
                .map(|block| {
                    let mean = block.iter().sum::<f64>() / block.len() as f64;
                    let scale = variance.v0.sqrt();
                    block.iter().map(|value| (value - mean) / scale).collect()
                })
                .collect();
            let output =
                estimate_correlation(&standardized, None, request.sample_interval_s, recipe)?;
            (Some(output), Some(recipe))
        }
        None => (None, None),
    };

    let heldout_toml = request.heldout.unwrap_or(HeldoutToml {
        blocks: None,
        profile_rmse_v2: None,
        standardized_lag1: None,
    });
    let artifact_request = ArtifactRequest {
        model_id: request.model_id.clone(),
        pmoke_version: env!("CARGO_PKG_VERSION").to_string(),
        backend_versions: BTreeMap::new(),
        binding: ModelBinding {
            channel: request.channel,
            voltage_unit: "V".to_string(),
            adc_scale_provenance: None,
            sample_interval_s: request.sample_interval_s,
            recorded_original_dt_s: Some(request.sample_interval_s),
            sample_interval_rel_tol: pmoke_analysis_core::calibration::DEFAULT_DT_REL_TOL,
            reference_frequency_hz: request.reference_frequency_hz,
            frequency_rel_tol: pmoke_analysis_core::calibration::DEFAULT_FREQ_REL_TOL,
            phase_convention: CALIBRATION_PHASE_CONVENTION.to_string(),
            acquisition: AcquisitionMeta {
                device: None,
                gain: None,
                bandwidth_hz: None,
            },
        },
        variance,
        correlation,
        recipe: phase_recipe,
        correlation_recipe,
        plan: plan.clone(),
        source_digests: vec![crate::utils::checksum::file_sha256(&waveform)?],
        seed: request.seed,
        tuning: TuningMode::Fixed,
        heldout: HeldoutReport {
            blocks: heldout_toml.blocks.unwrap_or(0),
            profile_rmse_v2: heldout_toml.profile_rmse_v2,
            standardized_lag1: heldout_toml.standardized_lag1,
        },
    };
    let built = build_artifact(artifact_request)?;

    // Adequacy gate on the training residuals (recorded, never an SNR
    // certificate): standardized by the fitted v0, phases from the block
    // times. Failure to assess is a hard build error, not a silent pass.
    let adequacy = {
        let groups: Vec<pmoke_analysis_core::calibration::AdequacyGroup> = block_times
            .iter()
            .zip(block_residuals.iter())
            .map(|(times, residuals)| {
                let mean = residuals.iter().sum::<f64>() / residuals.len() as f64;
                let scale = built.artifact.reference_variance_v2.sqrt();
                let standardized: Vec<f64> = residuals
                    .iter()
                    .map(|value| (value - mean) / scale)
                    .collect();
                let phases: Vec<f64> = times
                    .iter()
                    .map(|time| {
                        (std::f64::consts::TAU * request.reference_frequency_hz * time
                            - request.reference_phase_rad)
                            .rem_euclid(std::f64::consts::TAU)
                    })
                    .collect();
                pmoke_analysis_core::calibration::AdequacyGroup {
                    standardized,
                    phases,
                }
            })
            .collect();
        // Reserved role: reuse the training groups as the reserved set would
        // be self-agreement, so instead require adequacy against a split
        // half; an odd single block is insufficient evidence.
        if groups.len() < 2 {
            bail!("calibration adequacy needs at least two training blocks");
        }
        let (head, tail) = groups.split_at(groups.len() / 2);
        scs_adequacy(
            head,
            tail,
            AdequacyPolicy {
                lags: 4,
                min_pairs_per_cell: 10,
                max_phase_spread: 0.2,
                max_reserved_shift: 0.2,
            },
        )?
    };
    if !adequacy.adequate {
        bail!(
            "calibration residuals fail SCS adequacy ({}); refusing to publish",
            adequacy.reason
        );
    }

    let output = match output_override {
        Some(path) => path.to_path_buf(),
        None => request_dir.join(&request.output),
    };
    if output
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!(
            "calibration output must not contain '..': {}",
            output.display()
        );
    }
    ensure_new_dir(&output)?;
    let artifact_path = output.join("calibration.json");
    std::fs::write(&artifact_path, &built.json_bytes).with_context(|| {
        format!(
            "cannot write calibration artifact: {}",
            artifact_path.display()
        )
    })?;
    let report = CalibrateReport {
        schema_version: CALIBRATE_REQUEST_SCHEMA_VERSION,
        model_id: request.model_id.clone(),
        artifact: artifact_path.display().to_string(),
        sha256: built.sha256_hex.clone(),
        bytes: built.json_bytes.len(),
        modes: built.artifact.capabilities.modes.clone(),
        training_blocks: plan
            .blocks
            .iter()
            .filter(|block| block.role == CalibrationRole::Training)
            .count(),
        adequate: adequacy.adequate,
        adequacy_reason: adequacy.reason.clone(),
    };
    let report_path = output.join("calibrate-report.json");
    let text = serde_json::to_string_pretty(&report).context("cannot encode calibrate report")?;
    std::fs::write(&report_path, format!("{text}\n"))
        .with_context(|| format!("cannot write {}", report_path.display()))?;
    crate::ui::success(format!(
        "calibration artifact published: {} (sha256 {}, {} bytes)",
        artifact_path.display(),
        built.sha256_hex,
        built.json_bytes.len()
    ));
    println!("{text}");
    Ok(())
}

/// Inspects an artifact file: structural validation plus a readable summary.
pub fn run_inspect(artifact_path: &Path) -> Result<()> {
    let bytes = std::fs::read(artifact_path)
        .with_context(|| format!("cannot read artifact: {}", artifact_path.display()))?;
    if bytes.len() > pmoke_analysis_core::calibration::MAX_CALIBRATION_ARTIFACT_BYTES {
        bail!("artifact holds {} bytes above the 1 MiB cap", bytes.len());
    }
    let artifact: CalibrationArtifact = serde_json::from_slice(&bytes)
        .with_context(|| format!("not a valid artifact: {}", artifact_path.display()))?;
    if artifact.schema_version != CALIBRATION_ARTIFACT_SCHEMA_VERSION {
        bail!(
            "unsupported artifact schema_version {} (expected {CALIBRATION_ARTIFACT_SCHEMA_VERSION})",
            artifact.schema_version
        );
    }
    if artifact.algorithm_version != CALIBRATION_ALGORITHM_VERSION {
        bail!(
            "unknown artifact algorithm_version {:?}",
            artifact.algorithm_version
        );
    }
    crate::ui::settings_table(
        "Calibration artifact",
        vec![
            ("path".to_string(), artifact_path.display().to_string()),
            ("model_id".to_string(), artifact.model_id.clone()),
            (
                "schema/algorithm".to_string(),
                format!("{}/{}", artifact.schema_version, artifact.algorithm_version),
            ),
            ("channel".to_string(), artifact.binding.channel.to_string()),
            (
                "sample_interval_s".to_string(),
                format!("{:.6e}", artifact.binding.sample_interval_s),
            ),
            (
                "reference_frequency_hz".to_string(),
                format!("{:.6e}", artifact.binding.reference_frequency_hz),
            ),
            ("phase_bins".to_string(), artifact.phase.bins.to_string()),
            (
                "reference_variance_v2".to_string(),
                format!("{:.6e}", artifact.reference_variance_v2),
            ),
            ("modes".to_string(), artifact.capabilities.modes.join(",")),
            ("bytes".to_string(), bytes.len().to_string()),
            (
                "sha256".to_string(),
                crate::utils::checksum::sha256_hex(&bytes),
            ),
        ],
    );
    crate::ui::success("calibration artifact inspection completed");
    Ok(())
}

/// Validates an artifact file against a declared inference context TOML.
pub fn run_validate(artifact_path: &Path, context_path: &Path) -> Result<()> {
    let bytes = std::fs::read(artifact_path)
        .with_context(|| format!("cannot read artifact: {}", artifact_path.display()))?;
    let artifact: CalibrationArtifact = serde_json::from_slice(&bytes)
        .with_context(|| format!("not a valid artifact: {}", artifact_path.display()))?;
    let text = std::fs::read_to_string(context_path)
        .with_context(|| format!("cannot read context: {}", context_path.display()))?;
    let context: ApplicabilityRequest = toml::from_str(&text)
        .with_context(|| format!("invalid applicability context: {}", context_path.display()))?;
    let report = inspect_applicability(&artifact, &context);
    let text = serde_json::to_string_pretty(&report).context("cannot encode applicability")?;
    println!("{text}");
    if report.compatible {
        crate::ui::success("calibration artifact is applicable");
        Ok(())
    } else {
        let details = report
            .errors
            .iter()
            .map(|error| format!("{}: {}", error.code, error.message))
            .collect::<Vec<_>>()
            .join("; ");
        bail!("calibration artifact is not applicable: {details}");
    }
}

/// Dispatches the `calibrate` subcommands.
pub fn run(command: &crate::cli::CalibrateCommand) -> Result<()> {
    match command {
        crate::cli::CalibrateCommand::Build { request, output } => {
            run_build(request, output.as_deref())
        }
        crate::cli::CalibrateCommand::Inspect { artifact } => run_inspect(artifact),
        crate::cli::CalibrateCommand::Validate { artifact, context } => {
            run_validate(artifact, context)
        }
    }
}

#[cfg(test)]
#[path = "calibrate_tests.rs"]
mod tests;
