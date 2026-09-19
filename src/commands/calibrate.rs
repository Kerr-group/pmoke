//! Recorded-only calibration build/inspect/validate (R1, Issue #232).
//!
//! Pure core kernels (`pmoke-analysis-core::calibration`) exposed as native
//! commands over recorded waveform data. No instrument access: every
//! entry point reads explicit file paths and publishes into a NEW output
//! directory, never into a source run tree.
//!
//! - `build`: recorded CSV + TOML request -> immutable artifact JSON bytes
//!   plus SHA-256 identity, published under a new destination; or direct
//!   RAW build from a recorded run (`--run DIR --channel N`, Issues
//!   #269/#270) with no TOML and no CSV round-trip.
//! - `inspect`: structural validation + human-readable summary of an
//!   artifact file (read-only).
//! - `validate`: applicability of an artifact file against a declared
//!   inference context (read-only).
//!
//! ## Background-interval contract (Issue #269; prepulse is KEPT)
//!
//! Calibration trains on background (no-signal) data. The canonical source
//! is a drive-off acquisition (pulse field OFF); feeding drive-on
//! (drive-response-contaminated) data as background is misuse. Neither
//! entry point can detect the drive state from samples alone, so the
//! premise is documented here and recorded per build (see
//! [`ResolvedRequestRecord::reference_provenance`]), never silently
//! assumed: every build records the requested role intervals alongside the
//! effective planned blocks and every exclusion with its reason.
//!
//! Truncation is never silent: an out-of-range or inverted interval is a
//! hard build error; trailing samples that do not fill a whole block are
//! planned as named exclusions, surfaced as warnings, and recorded in the
//! report. The SCS adequacy gate (training phase spread <= 0.2) is
//! mandatory and mode-independent: it applies unchanged even with
//! `noise_mode = "phase_correlated"`, and its 0.2 threshold is never
//! relaxed. On failure, retake the intervals or data instead of lowering
//! the bar.
//!
//! ## Direct build defaults (Issue #270)
//!
//! `build --run DIR --channel N` resolves every request field without a
//! TOML file: `sample_interval_s` and the sample count come from the run
//! acquisition manifest, `reference_frequency_hz`/`reference_phase_rad`
//! come from a same-run reference-channel fit (canonically ch3), the model
//! id is `<run>-ch<N>`, the seed is 0, `block_len` is `n // 40`, and the
//! role intervals skip the first 2% of samples before splitting the rest
//! 70/15/15 across training (two contiguous intervals),
//! tuning-validation, and evaluation. Samples are ingested directly from
//! the RAW channel files. `--request` TOML is retained for overrides and
//! back-compat: an equivalent TOML request over identical samples yields
//! the identical artifact digest, because both paths bind the artifact to
//! the canonical sample digest (see [`canonical_sample_digest`]) rather
//! than to the container file. The fully resolved request is recorded in
//! the report, so a direct build reproduces without its TOML.
//!
//! The default direct-build output is CWD-anchored (`./calibration/` with
//! per-channel artifact filenames). When that default would resolve inside
//! the source run, the build is rejected with an explicit error: pass
//! `--output` to publish elsewhere. Direct builds are read-only toward
//! the source run (no writes, no staging, no state transitions) and stay
//! runnable without a runnable ambient configuration file: channel and
//! reference defaults come from the run's own configuration snapshot,
//! falling back to explicit flags when the snapshot is unavailable.

use crate::config::{ArtifactResolver, Config};
use crate::utils::time_axis::WaveformTime;
use anyhow::{Context, Result, bail};
use pmoke_analysis_core::calibration::{
    AcquisitionMeta, AdequacyPolicy, ApplicabilityRequest, ArtifactRequest, ArtifactWithHash,
    BlockPlan, BlockPlanRequest, CALIBRATION_ALGORITHM_VERSION,
    CALIBRATION_ARTIFACT_SCHEMA_VERSION, CALIBRATION_PHASE_CONVENTION, CalibrationArtifact,
    CalibrationRole, CorrelationRecipe, DEFAULT_MIN_CYCLES_PER_BLOCK, DEFAULT_MIN_TRAINING_BLOCKS,
    DEFAULT_MIN_TRAINING_INTERVALS, HeldoutReport, ModelBinding, PhaseVarianceRecipe, RoleInterval,
    TuningMode, assemble_samples, build_artifact, estimate_correlation, estimate_phase_variance,
    fit_nuisance, inspect_applicability, plan_blocks, scs_adequacy,
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
    /// Optional planning floor on reference cycles per block (defaults to
    /// the archival 128-cycle bar). Short-block builds (including the
    /// direct RAW path, which uses 2.0 with the unchanged SCS adequacy
    /// gate as the quality backstop) set this explicitly.
    pub min_reference_cycles_per_block: Option<f64>,
    /// Optional minimum usable training blocks (defaults to 8).
    pub min_training_blocks: Option<usize>,
    /// Optional minimum contributing training intervals (defaults to 2).
    pub min_training_intervals: Option<usize>,
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

/// Fixed direct-build seed (Issue #270 FR-04).
pub const DIRECT_DEFAULT_SEED: u64 = 0;
/// Direct-build block length divisor: `block_len = n // 40`.
pub const DIRECT_BLOCK_DIVISOR: usize = 40;
/// Direct-build leading samples skipped (percent of `n`): first-2% cut.
pub const DIRECT_SKIP_FIRST_PCT: u64 = 2;
/// Direct-build role split of the post-skip samples (percent): 70/15/15.
pub const DIRECT_TRAIN_PCT: u64 = 70;
/// Direct-build tuning-validation share (percent of the post-skip samples).
pub const DIRECT_TUNE_PCT: u64 = 15;
/// Minimum block length accepted for the direct-build default. The nuisance
/// regression spans 26 parameters; shorter blocks cannot support it.
/// Below this floor the build errors and suggests `--block-len`.
pub const DIRECT_MIN_BLOCK_LEN: usize = 64;
/// Direct-build planning floor on reference cycles per block. Short blocks
/// by construction (mirrors the pre-pulse scale); the quality backstop is
/// the unchanged SCS adequacy gate (spread <= 0.2), not this floor.
pub const DIRECT_MIN_CYCLES_PER_BLOCK: f64 = 2.0;
/// Direct-build phase-variance bins (the `pmoke calibrate` CLI scale).
pub const DIRECT_BINS: usize = 8;
/// Direct-build minimum samples per phase bin (short-block scale, mirrors
/// the pre-pulse derivation).
pub const DIRECT_MIN_SAMPLES_PER_BIN: usize = 8;
/// Direct-build minimum distinct reference cycles per phase bin (mirrors
/// the pre-pulse derivation).
pub const DIRECT_MIN_CYCLES_PER_BIN: usize = 2;
/// Default direct-build output directory name, anchored at the CWD.
pub const DIRECT_CALIBRATION_DIR: &str = "calibration";

/// One source file bound into a build report (audit trail only; the
/// artifact digest binds to the canonical sample digest instead, so RAW
/// and CSV containers over identical samples yield identical digests).
#[derive(Debug, Clone, Serialize)]
pub struct SourceFileRecord {
    pub path: String,
    pub sha256: String,
}

/// Requested role interval as recorded in the build report.
#[derive(Debug, Clone, Serialize)]
pub struct ReportedInterval {
    pub role: String,
    pub start: u64,
    pub end: u64,
}

/// Effective planned block as recorded in the build report.
#[derive(Debug, Clone, Serialize)]
pub struct ReportedBlock {
    pub role: String,
    pub start: u64,
    pub end: u64,
}

/// Planned exclusion as recorded in the build report.
#[derive(Debug, Clone, Serialize)]
pub struct ReportedExclusion {
    pub start: u64,
    pub end: u64,
    pub reason: String,
}

/// Fully resolved build request as recorded in the build report (Issue
/// #270 FR-06): a direct build reproduces from this record without its
/// TOML file.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedRequestRecord {
    pub source_kind: String,
    pub model_id: String,
    pub channel: u32,
    pub sample_interval_s: f64,
    pub sample_count: usize,
    pub sample_digest: String,
    pub source_files: Vec<SourceFileRecord>,
    pub reference_frequency_hz: f64,
    pub reference_phase_rad: f64,
    pub reference_provenance: String,
    pub seed: u64,
    pub block_len: usize,
    pub intervals: Vec<ReportedInterval>,
    pub output_artifact: String,
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
    /// Role intervals exactly as requested (Issue #269 FR-01).
    pub requested_intervals: Vec<ReportedInterval>,
    /// Planned blocks actually used (Issue #269 FR-01).
    pub effective_blocks: Vec<ReportedBlock>,
    /// Planned exclusions with reasons (Issue #269 FR-01/FR-02).
    pub exclusions: Vec<ReportedExclusion>,
    /// Truncation and premise warnings surfaced during the build.
    pub warnings: Vec<String>,
    /// Fully resolved request for reproduction without TOML.
    pub resolved_request: ResolvedRequestRecord,
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

/// Canonical digest over ingested samples (times then signal, f64 LE
/// bytes under a domain separator). Both the CSV and RAW paths bind the
/// artifact to this digest instead of to the container file, so identical
/// samples yield the identical artifact digest on either path (Issue #270
/// FR-05). Container file identities stay in the report as an audit trail.
pub fn canonical_sample_digest(times: &[f64], signal: &[f64]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"pmoke-calibration-samples/v1\n");
    hasher.update((times.len() as u64).to_le_bytes());
    hasher.update((signal.len() as u64).to_le_bytes());
    for value in times.iter().chain(signal.iter()) {
        hasher.update(value.to_le_bytes());
    }
    crate::utils::checksum::finalize_sha256_hex(hasher.finalize())
}

fn role_name(role: CalibrationRole) -> &'static str {
    match role {
        CalibrationRole::Training => "training",
        CalibrationRole::TuningValidation => "tuning_validation",
        CalibrationRole::Evaluation => "evaluation",
    }
}

fn normalize_report_path(path: &Path) -> String {
    // Manifest/JSON path keys use `/` on every platform (Windows
    // `Path::display` would otherwise emit `\` separators).
    path.display().to_string().replace('\\', "/")
}

/// Fully resolved build inputs shared by the TOML and direct paths. Both
/// paths execute through [`execute_resolved_build`], so identical samples
/// plus identical parameters yield identical artifact bytes.
struct ResolvedBuild {
    model_id: String,
    channel: u32,
    sample_interval_s: f64,
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    reference_provenance: String,
    source_kind: &'static str,
    source_files: Vec<SourceFileRecord>,
    seed: u64,
    block_len: usize,
    intervals: Vec<RoleInterval>,
    phase_recipe: PhaseVarianceRecipe,
    correlation_recipe: Option<CorrelationRecipe>,
    heldout_blocks: usize,
    heldout_profile_rmse_v2: Option<f64>,
    heldout_standardized_lag1: Option<f64>,
    min_reference_cycles_per_block: f64,
    min_training_blocks: usize,
    min_training_intervals: usize,
    times: Vec<f64>,
    signal: Vec<f64>,
    sample_digest: String,
}

/// Computed build outputs (no filesystem writes).
struct BuiltCalibration {
    built: ArtifactWithHash,
    plan: BlockPlan,
    adequacy_reason: String,
    training_blocks: usize,
    warnings: Vec<String>,
}

/// Plans whole blocks inside the requested role intervals. Out-of-range,
/// inverted, or overlapping intervals are hard errors; trailing samples
/// that do not fill a whole block become named exclusions (warned and
/// recorded by the caller, never silent).
#[allow(clippy::too_many_arguments)]
fn plan_calibration(
    intervals: &[RoleInterval],
    block_len: usize,
    total_samples: u64,
    reference_frequency_hz: f64,
    sample_interval_s: f64,
    min_reference_cycles_per_block: f64,
    min_training_blocks: usize,
    min_training_intervals: usize,
) -> Result<BlockPlan> {
    if total_samples == 0 {
        bail!("calibration waveform holds no samples");
    }
    if intervals.is_empty() {
        bail!("calibration request lists no role intervals");
    }
    let plan_request = BlockPlanRequest {
        intervals: intervals.to_vec(),
        block_len,
        total_samples,
        reference_frequency_hz,
        sample_interval_s,
        min_reference_cycles_per_block,
        min_training_blocks,
        min_training_intervals,
    };
    Ok(plan_blocks(&plan_request)?)
}

fn plan_resolved_build(resolved: &ResolvedBuild) -> Result<BlockPlan> {
    plan_calibration(
        &resolved.intervals,
        resolved.block_len,
        resolved.times.len() as u64,
        resolved.reference_frequency_hz,
        resolved.sample_interval_s,
        resolved.min_reference_cycles_per_block,
        resolved.min_training_blocks,
        resolved.min_training_intervals,
    )
}

/// Executes the estimation pipeline over resolved inputs: per-block
/// nuisance fits, phase-variance (plus optional correlation) estimation,
/// the mandatory SCS adequacy gate, and artifact construction.
fn execute_resolved_build(resolved: &ResolvedBuild, plan: &BlockPlan) -> Result<BuiltCalibration> {
    let training: Vec<_> = plan
        .blocks
        .iter()
        .filter(|block| block.role == CalibrationRole::Training)
        .collect();
    if training.is_empty() {
        bail!("calibration plan holds no training blocks");
    }
    if !(resolved.sample_interval_s.is_finite() && resolved.sample_interval_s > 0.0) {
        bail!("calibration request needs a positive finite sample_interval_s");
    }
    if !(resolved.reference_frequency_hz.is_finite() && resolved.reference_frequency_hz > 0.0) {
        bail!("calibration request needs a positive finite reference_frequency_hz");
    }
    if !resolved.reference_phase_rad.is_finite() {
        bail!("calibration request needs a finite reference_phase_rad");
    }
    let sample_rate_hz = 1.0 / resolved.sample_interval_s;
    let tolerances = JointSolverTolerances::default();
    let mut block_times = Vec::with_capacity(training.len());
    let mut block_residuals = Vec::with_capacity(training.len());
    for block in &training {
        let (start, end) = (block.start as usize, block.end as usize);
        let fit = fit_nuisance(
            &resolved.times[start..end],
            &resolved.signal[start..end],
            resolved.reference_frequency_hz,
            resolved.reference_phase_rad,
            sample_rate_hz,
            tolerances,
        )?;
        block_times.push(resolved.times[start..end].to_vec());
        block_residuals.push(fit.residual);
    }
    let samples = assemble_samples(
        &block_times,
        &block_residuals,
        resolved.reference_frequency_hz,
        resolved.reference_phase_rad,
    )?;
    let variance = estimate_phase_variance(&samples, resolved.phase_recipe)?;

    let (correlation, correlation_recipe) = match resolved.correlation_recipe {
        Some(recipe) => {
            let standardized: Vec<Vec<f64>> = block_residuals
                .iter()
                .map(|block| {
                    let mean = block.iter().sum::<f64>() / block.len() as f64;
                    let scale = variance.v0.sqrt();
                    block.iter().map(|value| (value - mean) / scale).collect()
                })
                .collect();
            let output =
                estimate_correlation(&standardized, None, resolved.sample_interval_s, recipe)?;
            (Some(output), Some(recipe))
        }
        None => (None, None),
    };

    let artifact_request = ArtifactRequest {
        model_id: resolved.model_id.clone(),
        pmoke_version: env!("CARGO_PKG_VERSION").to_string(),
        backend_versions: BTreeMap::new(),
        binding: ModelBinding {
            channel: resolved.channel,
            voltage_unit: "V".to_string(),
            adc_scale_provenance: None,
            sample_interval_s: resolved.sample_interval_s,
            recorded_original_dt_s: Some(resolved.sample_interval_s),
            sample_interval_rel_tol: pmoke_analysis_core::calibration::DEFAULT_DT_REL_TOL,
            reference_frequency_hz: resolved.reference_frequency_hz,
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
        recipe: resolved.phase_recipe,
        correlation_recipe,
        plan: plan.clone(),
        source_digests: vec![resolved.sample_digest.clone()],
        seed: resolved.seed,
        tuning: TuningMode::Fixed,
        heldout: HeldoutReport {
            blocks: resolved.heldout_blocks,
            profile_rmse_v2: resolved.heldout_profile_rmse_v2,
            standardized_lag1: resolved.heldout_standardized_lag1,
        },
    };
    let built = build_artifact(artifact_request)?;

    // Adequacy gate on the training residuals (recorded, never an SNR
    // certificate): standardized by the fitted v0, phases from the block
    // times. Mandatory and mode-independent (Issue #269 FR-04): the
    // threshold is unchanged and applies even with
    // `noise_mode = "phase_correlated"`. Failure to assess is a hard
    // build error, not a silent pass.
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
                        (std::f64::consts::TAU * resolved.reference_frequency_hz * time
                            - resolved.reference_phase_rad)
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

    // Planned exclusions are surfaced as warnings (Issue #269 FR-02: never
    // a silent truncation) and recorded in the report.
    let mut warnings = Vec::new();
    for exclusion in plan.exclusions.iter() {
        let warning = format!(
            "excluded samples [{}, {}): {}",
            exclusion.start, exclusion.end, exclusion.reason
        );
        crate::ui::warn(warning.clone());
        warnings.push(warning);
    }
    Ok(BuiltCalibration {
        training_blocks: training.len(),
        adequacy_reason: adequacy.reason.clone(),
        built,
        plan: plan.clone(),
        warnings,
    })
}

/// Assembles the report for a built artifact: requested intervals,
/// effective blocks, exclusions, warnings, and the fully resolved request
/// (Issues #269 FR-01, #270 FR-06).
fn report_for(
    resolved: &ResolvedBuild,
    built: &BuiltCalibration,
    artifact_path: &Path,
) -> CalibrateReport {
    let requested_intervals = resolved
        .intervals
        .iter()
        .map(|interval| ReportedInterval {
            role: role_name(interval.role).to_string(),
            start: interval.start,
            end: interval.end,
        })
        .collect();
    let effective_blocks = built
        .plan
        .blocks
        .iter()
        .map(|block| ReportedBlock {
            role: role_name(block.role).to_string(),
            start: block.start,
            end: block.end,
        })
        .collect();
    let exclusions = built
        .plan
        .exclusions
        .iter()
        .map(|exclusion| ReportedExclusion {
            start: exclusion.start,
            end: exclusion.end,
            reason: exclusion.reason.clone(),
        })
        .collect();
    CalibrateReport {
        schema_version: CALIBRATE_REQUEST_SCHEMA_VERSION,
        model_id: resolved.model_id.clone(),
        artifact: normalize_report_path(artifact_path),
        sha256: built.built.sha256_hex.clone(),
        bytes: built.built.json_bytes.len(),
        modes: built.built.artifact.capabilities.modes.clone(),
        training_blocks: built.training_blocks,
        adequate: true,
        adequacy_reason: built.adequacy_reason.clone(),
        requested_intervals,
        effective_blocks,
        exclusions,
        warnings: built.warnings.clone(),
        resolved_request: ResolvedRequestRecord {
            source_kind: resolved.source_kind.to_string(),
            model_id: resolved.model_id.clone(),
            channel: resolved.channel,
            sample_interval_s: resolved.sample_interval_s,
            sample_count: resolved.times.len(),
            sample_digest: resolved.sample_digest.clone(),
            source_files: resolved.source_files.clone(),
            reference_frequency_hz: resolved.reference_frequency_hz,
            reference_phase_rad: resolved.reference_phase_rad,
            reference_provenance: resolved.reference_provenance.clone(),
            seed: resolved.seed,
            block_len: resolved.block_len,
            intervals: resolved
                .intervals
                .iter()
                .map(|interval| ReportedInterval {
                    role: role_name(interval.role).to_string(),
                    start: interval.start,
                    end: interval.end,
                })
                .collect(),
            output_artifact: normalize_report_path(artifact_path),
        },
    }
}

fn write_calibrate_report(report_path: &Path, report: &CalibrateReport) -> Result<()> {
    let text = serde_json::to_string_pretty(&report).context("cannot encode calibrate report")?;
    std::fs::write(report_path, format!("{text}\n"))
        .with_context(|| format!("cannot write {}", report_path.display()))?;
    Ok(())
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
    let correlation_recipe = match request.correlation_recipe {
        Some(recipe_toml) => {
            let recipe_defaults = CorrelationRecipe::default();
            Some(CorrelationRecipe {
                max_lag: recipe_toml.max_lag.unwrap_or(recipe_defaults.max_lag),
                shrinkage_eta: recipe_toml
                    .shrinkage_eta
                    .unwrap_or(recipe_defaults.shrinkage_eta),
            })
        }
        None => None,
    };

    let heldout_toml = request.heldout.unwrap_or(HeldoutToml {
        blocks: None,
        profile_rmse_v2: None,
        standardized_lag1: None,
    });
    // The artifact binds to the canonical sample digest (shared with the
    // direct RAW path), not to the CSV container file; the container
    // identity stays in the report as an audit trail.
    let sample_digest = canonical_sample_digest(&times, &signal);
    let waveform_sha = crate::utils::checksum::file_sha256(&waveform)?;
    // Planning floors: archival defaults unless the request overrides
    // them explicitly (short-block builds set a lower cycle floor with
    // the unchanged SCS adequacy gate as the quality backstop).
    let min_reference_cycles_per_block = request
        .min_reference_cycles_per_block
        .unwrap_or(DEFAULT_MIN_CYCLES_PER_BLOCK);
    if !(min_reference_cycles_per_block.is_finite() && min_reference_cycles_per_block > 0.0) {
        bail!("calibration request needs a positive finite min_reference_cycles_per_block");
    }
    let resolved = ResolvedBuild {
        model_id: request.model_id.clone(),
        channel: request.channel,
        sample_interval_s: request.sample_interval_s,
        reference_frequency_hz: request.reference_frequency_hz,
        reference_phase_rad: request.reference_phase_rad,
        reference_provenance: "toml-request".to_string(),
        source_kind: "csv",
        source_files: vec![SourceFileRecord {
            path: normalize_report_path(&waveform),
            sha256: waveform_sha,
        }],
        seed: request.seed,
        block_len: request.block_len,
        intervals: role_intervals,
        phase_recipe,
        correlation_recipe,
        heldout_blocks: heldout_toml.blocks.unwrap_or(0),
        heldout_profile_rmse_v2: heldout_toml.profile_rmse_v2,
        heldout_standardized_lag1: heldout_toml.standardized_lag1,
        min_reference_cycles_per_block,
        min_training_blocks: request
            .min_training_blocks
            .unwrap_or(DEFAULT_MIN_TRAINING_BLOCKS),
        min_training_intervals: request
            .min_training_intervals
            .unwrap_or(DEFAULT_MIN_TRAINING_INTERVALS),
        times,
        signal,
        sample_digest,
    };
    let plan = plan_resolved_build(&resolved)?;
    let executed = execute_resolved_build(&resolved, &plan)?;

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
    std::fs::write(&artifact_path, &executed.built.json_bytes).with_context(|| {
        format!(
            "cannot write calibration artifact: {}",
            artifact_path.display()
        )
    })?;
    let report = report_for(&resolved, &executed, &artifact_path);
    let report_path = output.join("calibrate-report.json");
    write_calibrate_report(&report_path, &report)?;
    let text = serde_json::to_string_pretty(&report).context("cannot encode calibrate report")?;
    crate::ui::success(format!(
        "calibration artifact published: {} (sha256 {}, {} bytes)",
        artifact_path.display(),
        executed.built.sha256_hex,
        executed.built.json_bytes.len()
    ));
    println!("{text}");
    Ok(())
}

/// Options for a direct RAW build, mirroring the `calibrate build` flags
/// (Issue #270). `run` defaults to `.`; `channels` defaults to the run
/// configuration signal channels.
#[derive(Debug, Clone, Default)]
pub struct DirectBuildOptions {
    pub run: Option<PathBuf>,
    pub channels: Vec<u32>,
    pub output: Option<PathBuf>,
    pub model_id: Option<String>,
    pub reference_channel: Option<u8>,
    pub reference_frequency_hz: Option<f64>,
    pub reference_phase_rad: Option<f64>,
    pub block_len: Option<usize>,
    pub seed: Option<u64>,
}

/// Resolves `--run` to a lookup root: the explicit directory, else `.`
/// (the same resolution idiom as the global `--run-dir` default). The
/// directory is a read-only lookup root only: direct builds never write,
/// stage, or transition state inside it.
pub fn resolve_direct_run_dir(run: Option<&Path>) -> PathBuf {
    run.map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Synthesizes direct-build role intervals over `total` samples: skip the
/// first 2%, then split the rest 70/15/15 across training (two contiguous
/// intervals, so the two-interval planning minimum holds by construction),
/// tuning-validation, and evaluation. Degenerate splits are hard errors,
/// never silent truncations.
fn direct_role_intervals(total: u64) -> Result<Vec<RoleInterval>> {
    if total == 0 {
        bail!("calibration waveform holds no samples");
    }
    let skip = total * DIRECT_SKIP_FIRST_PCT / 100;
    let usable = total - skip;
    let train = usable * DIRECT_TRAIN_PCT / 100;
    let tune = usable * DIRECT_TUNE_PCT / 100;
    let train_start = skip;
    let train_mid = train_start + train / 2;
    let train_end = train_start + train;
    let tune_end = train_end + tune;
    for (name, start, end) in [
        ("training", train_start, train_mid),
        ("training", train_mid, train_end),
        ("tuning_validation", train_end, tune_end),
        ("evaluation", tune_end, total),
    ] {
        if end <= start {
            bail!(
                "direct calibration intervals degenerate for {total} samples \
                 ({name} [{start}, {end})): run too short for the skip-first-2% + 70/15/15 split"
            );
        }
    }
    Ok(vec![
        RoleInterval {
            role: CalibrationRole::Training,
            start: train_start,
            end: train_mid,
        },
        RoleInterval {
            role: CalibrationRole::Training,
            start: train_mid,
            end: train_end,
        },
        RoleInterval {
            role: CalibrationRole::TuningValidation,
            start: train_end,
            end: tune_end,
        },
        RoleInterval {
            role: CalibrationRole::Evaluation,
            start: tune_end,
            end: total,
        },
    ])
}

/// Lexically absolutizes a path (resolving `.`/`..` without touching the
/// filesystem, so not-yet-created output directories compare correctly).
fn lexical_absolute(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in absolute.components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

/// Canonicalizes the existing prefix of a path (resolving symlinks such
/// as `/tmp` -> `/private/tmp`) and re-appends the not-yet-created tail
/// lexically, so not-yet-created output directories compare correctly
/// against the canonical source run.
fn canonicalize_existing_prefix(path: &Path) -> PathBuf {
    let mut current = path.to_path_buf();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(canonical) = current.canonicalize() {
            let mut resolved = canonical;
            for component in tail.iter().rev() {
                resolved.push(component);
            }
            return resolved;
        }
        match current.file_name() {
            Some(name) => {
                tail.push(name.to_os_string());
                current.pop();
            }
            None => return path.to_path_buf(),
        }
    }
}

/// Rejects the default CWD-anchored output when it would resolve inside
/// the source run, preserving the never-into-source-run-tree contract and
/// run-dir immutability. Pass an explicit `--output` to publish elsewhere.
fn reject_default_output_inside_run(output_base: &Path, run_dir: &Path) -> Result<()> {
    let run_canon = run_dir
        .canonicalize()
        .with_context(|| format!("cannot resolve run directory: {}", run_dir.display()))?;
    let output_canon = canonicalize_existing_prefix(output_base);
    if output_canon == run_canon || output_canon.starts_with(&run_canon) {
        bail!(
            "default direct-build output {} resolves inside the source run {}; \
             refusing to publish into the source run tree (pass --output to publish elsewhere)",
            output_base.display(),
            run_canon.display()
        );
    }
    Ok(())
}

/// Loads the run's own configuration snapshot (`config.resolved.toml`,
/// else `config.source.toml`) for channel and reference resolution. A
/// normalized-but-not-runnable snapshot is accepted with a warning, like
/// the CSV export path; a missing snapshot yields `None`, and the caller
/// must then require explicit flags.
fn load_run_snapshot_config(run_dir: &Path) -> Result<Option<Config>> {
    for name in ["config.resolved.toml", "config.source.toml"] {
        let path = run_dir.join(name);
        if !path.is_file() {
            continue;
        }
        match crate::config::load_from_path(&path) {
            crate::config::ConfigLoad::Ready { config, .. } => return Ok(Some(config)),
            crate::config::ConfigLoad::Diagnostics(diagnostics) => {
                if let Some(config) = diagnostics.normalized {
                    crate::ui::warn(format!(
                        "run configuration snapshot {} is not runnable; \
                         using its normalized channel/reference defaults",
                        path.display()
                    ));
                    return Ok(Some(config));
                }
            }
        }
    }
    Ok(None)
}

/// Derives the `<run>` stem for default model ids (`<run>-ch<N>`): the run
/// directory name, else the current directory name, else `run`.
fn direct_run_stem(run_dir: &Path) -> String {
    let file_stem = run_dir.file_name().and_then(|name| name.to_str());
    if let Some(stem) = file_stem.filter(|stem| !stem.is_empty() && *stem != "." && *stem != "..") {
        return stem.to_string();
    }
    if (run_dir.as_os_str().is_empty() || run_dir.as_os_str() == ".")
        && let Ok(cwd) = std::env::current_dir()
        && let Some(stem) = cwd.file_name().and_then(|name| name.to_str())
    {
        return stem.to_string();
    }
    "run".to_string()
}

fn refuse_existing_output(path: &Path) -> Result<()> {
    if path.exists() {
        bail!(
            "calibration output already exists (no overwrite by default): {}",
            path.display()
        );
    }
    Ok(())
}

/// Builds calibration artifacts directly from a recorded RAW run with no
/// TOML request and no CSV round-trip (Issue #270). Read-only toward the
/// source run: samples, manifest, and configuration snapshot are read, and
/// every artifact plus report is published under the output base (default
/// `./calibration/`, CWD-anchored).
pub fn run_build_direct(options: &DirectBuildOptions) -> Result<()> {
    let run_dir = resolve_direct_run_dir(options.run.as_deref());
    if !run_dir.is_dir() {
        bail!(
            "direct calibration run directory does not exist: {}",
            run_dir.display()
        );
    }
    let resolver = ArtifactResolver::new(&run_dir);
    let manifest_path = resolver.acquisition_manifest();
    if !manifest_path.is_file() {
        bail!(
            "direct calibration needs a recorded RAW run with an acquisition manifest \
             (looked for {}); CSV runs use --request TOML",
            manifest_path.display()
        );
    }
    let acquisition_dir = manifest_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let snapshot = load_run_snapshot_config(&run_dir)?;

    // Target channels: explicit flags win; otherwise every signal channel
    // of the run configuration (the normalized `lockin.channels`).
    let targets: Vec<u8> = if options.channels.is_empty() {
        let config = snapshot.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "direct calibration needs target channels: no --channel given and no run \
                 configuration snapshot found (missing config.resolved.toml/config.source.toml)"
            )
        })?;
        if config.roles.signal_ch.is_empty() {
            bail!("run configuration declares no signal channels; pass --channel explicitly");
        }
        config.roles.signal_ch.clone()
    } else {
        options
            .channels
            .iter()
            .map(|channel| {
                u8::try_from(*channel).map_err(|_| {
                    anyhow::anyhow!(
                        "direct calibration channel {channel} exceeds the RAW channel range 1..=255"
                    )
                })
            })
            .collect::<Result<Vec<_>>>()?
    };
    if targets.contains(&0) {
        bail!("direct calibration channels must be nonzero");
    }
    if options.model_id.is_some() && targets.len() != 1 {
        bail!(
            "--model-id needs a single target channel (got {})",
            targets.len()
        );
    }

    // Reference frequency/phase: explicit flag pairs win; otherwise fit the
    // same-run reference channel (canonically ch3).
    let need_fit =
        options.reference_frequency_hz.is_none() || options.reference_phase_rad.is_none();
    if !need_fit && options.reference_channel.is_some() {
        crate::ui::warn(
            "--reference-channel is ignored when --reference-frequency-hz \
             with --reference-phase-rad is given",
        );
    }
    if options.reference_frequency_hz.is_some() != options.reference_phase_rad.is_some() {
        bail!(
            "direct calibration needs both --reference-frequency-hz and --reference-phase-rad \
             together (or neither, for the same-run reference fit)"
        );
    }
    let reference_channel: Option<u8> = if need_fit {
        let config = snapshot.as_ref().ok_or_else(|| {
            anyhow::anyhow!(
                "direct calibration needs a run configuration snapshot for the same-run \
                 reference fit (missing config.resolved.toml/config.source.toml); pass \
                 --reference-frequency-hz with --reference-phase-rad instead"
            )
        })?;
        let reference = options
            .reference_channel
            .unwrap_or(config.roles.reference_ch);
        if reference == 0 {
            bail!(
                "run configuration declares no reference channel; pass --reference-channel \
                 (canonically 3) or --reference-frequency-hz with --reference-phase-rad"
            );
        }
        Some(reference)
    } else {
        None
    };

    // RAW ingest (verified sizes and checksums, no CSV round-trip).
    let mut read_list = targets.clone();
    if let Some(reference) = reference_channel
        && !read_list.contains(&reference)
    {
        read_list.push(reference);
    }
    let waveform =
        crate::utils::waveform::read_raw_waveform_channels_from_dir(&acquisition_dir, &read_list)
            .with_context(|| {
            format!(
                "cannot read RAW channels {read_list:?} from {}",
                acquisition_dir.display()
            )
        })?;
    let n = waveform.t.len();
    if n == 0 {
        bail!("calibration waveform holds no samples");
    }
    let sample_interval_s = match &waveform.t {
        WaveformTime::Uniform(axis) => axis.x_increment,
        WaveformTime::Explicit(_) => {
            bail!("direct calibration needs a uniform RAW timebase")
        }
    };
    if !(sample_interval_s.is_finite() && sample_interval_s > 0.0) {
        bail!("run manifest declares a non-positive sample interval");
    }
    let mut signals: BTreeMap<u8, Vec<f64>> = BTreeMap::new();
    for (channel, signal) in read_list.iter().zip(waveform.channels) {
        signals.insert(*channel, signal);
    }
    let times = waveform.t.to_vec();

    let (reference_frequency_hz, reference_phase_rad, reference_provenance) =
        match (options.reference_frequency_hz, options.reference_phase_rad) {
            (Some(frequency), Some(phase)) => {
                if !(frequency.is_finite() && frequency > 0.0) {
                    bail!("direct calibration needs a positive finite --reference-frequency-hz");
                }
                if !phase.is_finite() {
                    bail!("direct calibration needs a finite --reference-phase-rad");
                }
                (
                    frequency,
                    phase,
                    "--reference-frequency-hz/--reference-phase-rad flag override".to_string(),
                )
            }
            _ => {
                let config = snapshot.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("run configuration snapshot vanished before the reference fit")
                })?;
                let reference = reference_channel.ok_or_else(|| {
                    anyhow::anyhow!("reference channel resolution vanished before the fit")
                })?;
                let reference_signal = signals.get(&reference).ok_or_else(|| {
                    anyhow::anyhow!("reference channel {reference} missing from RAW ingest")
                })?;
                let fit = crate::lockin::reference::run_fit_ref_core_without_plot(
                    config,
                    waveform.t.as_ref(),
                    reference_signal,
                )
                .with_context(|| {
                    format!("same-run reference fit failed for channel {reference}")
                })?;
                let provenance = format!(
                    "same-run-ch{reference}-fit f_ref={:.6e} Hz omega_tref={:.6e} rad \
                     fft_window=[{:.6e}, {:.6e}] stride={} window={}",
                    fit.f_ref,
                    fit.omega_tref,
                    config.reference.fft_window.start,
                    config.reference.fft_window.end,
                    config.reference.stride_samples,
                    config.reference.window_samples,
                );
                (fit.f_ref, fit.omega_tref, provenance)
            }
        };

    let block_len = match options.block_len {
        Some(len) => {
            if len == 0 {
                bail!("direct calibration --block-len must be positive");
            }
            len
        }
        None => {
            let len = n / DIRECT_BLOCK_DIVISOR;
            if len < DIRECT_MIN_BLOCK_LEN {
                bail!(
                    "direct calibration run holds only {n} samples (default block_len {len} \
                     below the {DIRECT_MIN_BLOCK_LEN}-sample floor); pass --block-len explicitly \
                     or use a longer capture"
                );
            }
            len
        }
    };
    let intervals = direct_role_intervals(n as u64)?;
    // The plan is channel-independent: compute once, reuse per channel.
    let plan = plan_calibration(
        &intervals,
        block_len,
        n as u64,
        reference_frequency_hz,
        sample_interval_s,
        DIRECT_MIN_CYCLES_PER_BLOCK,
        pmoke_analysis_core::calibration::DEFAULT_MIN_TRAINING_BLOCKS,
        pmoke_analysis_core::calibration::DEFAULT_MIN_TRAINING_INTERVALS,
    )?;
    let training_count = plan
        .blocks
        .iter()
        .filter(|block| block.role == CalibrationRole::Training)
        .count();
    let phase_recipe = PhaseVarianceRecipe {
        bins: DIRECT_BINS,
        min_samples_per_bin: DIRECT_MIN_SAMPLES_PER_BIN,
        min_cycles_per_bin: DIRECT_MIN_CYCLES_PER_BIN,
        min_contributing_blocks: training_count,
        shrinkage_alpha: pmoke_analysis_core::calibration::DEFAULT_SHRINKAGE_ALPHA,
        floor_ratio: pmoke_analysis_core::calibration::DEFAULT_FLOOR_RATIO,
    };

    // Output base: explicit `--output` wins, else CWD-anchored
    // `./calibration/` with per-channel artifact filenames. The default is
    // rejected when it would resolve inside the source run.
    let (output_base, is_default) = match &options.output {
        Some(output) => (output.clone(), false),
        None => (
            std::env::current_dir()
                .context("cannot resolve the current directory for the default output")?
                .join(DIRECT_CALIBRATION_DIR),
            true,
        ),
    };
    let output_base = lexical_absolute(&output_base);
    if is_default {
        reject_default_output_inside_run(&output_base, &run_dir)?;
    }
    std::fs::create_dir_all(&output_base).with_context(|| {
        format!(
            "cannot create calibration output: {}",
            output_base.display()
        )
    })?;
    let run_stem = direct_run_stem(&run_dir);
    let seed = options.seed.unwrap_or(DIRECT_DEFAULT_SEED);

    // Report audit trail: manifest plus the ingested channel files.
    let base_sources = vec![SourceFileRecord {
        path: normalize_report_path(&manifest_path),
        sha256: crate::utils::checksum::file_sha256(&manifest_path)?,
    }];
    let mut channel_file_cache: BTreeMap<u8, SourceFileRecord> = BTreeMap::new();
    for channel in read_list.iter() {
        let path = resolver.raw_channel(*channel);
        let record = SourceFileRecord {
            path: normalize_report_path(&path),
            sha256: crate::utils::checksum::file_sha256(&path)
                .with_context(|| format!("cannot hash RAW channel file: {}", path.display()))?,
        };
        channel_file_cache.insert(*channel, record);
    }

    for target in targets.iter() {
        let signal = signals
            .get(target)
            .ok_or_else(|| anyhow::anyhow!("target channel {target} missing from RAW ingest"))?;
        let model_id = match &options.model_id {
            Some(id) => {
                if id.trim().is_empty() {
                    bail!("direct calibration needs a non-empty --model-id");
                }
                id.clone()
            }
            None => format!("{run_stem}-ch{target}"),
        };
        let sample_digest = canonical_sample_digest(&times, signal);
        let mut source_files = base_sources.clone();
        if let Some(reference) = reference_channel
            && reference != *target
            && let Some(record) = channel_file_cache.get(&reference)
        {
            source_files.push(record.clone());
        }
        if let Some(record) = channel_file_cache.get(target) {
            source_files.push(record.clone());
        }
        let resolved = ResolvedBuild {
            model_id,
            channel: u32::from(*target),
            sample_interval_s,
            reference_frequency_hz,
            reference_phase_rad,
            reference_provenance: reference_provenance.clone(),
            source_kind: "raw",
            source_files,
            seed,
            block_len,
            intervals: intervals.clone(),
            phase_recipe,
            correlation_recipe: None,
            heldout_blocks: 0,
            heldout_profile_rmse_v2: None,
            heldout_standardized_lag1: None,
            min_reference_cycles_per_block: DIRECT_MIN_CYCLES_PER_BLOCK,
            min_training_blocks: pmoke_analysis_core::calibration::DEFAULT_MIN_TRAINING_BLOCKS,
            min_training_intervals:
                pmoke_analysis_core::calibration::DEFAULT_MIN_TRAINING_INTERVALS,
            times: times.clone(),
            signal: signal.clone(),
            sample_digest,
        };
        let executed = execute_resolved_build(&resolved, &plan)?;
        let artifact_path = output_base.join(format!("ch{target}.json"));
        let report_path = output_base.join(format!("calibrate-report-ch{target}.json"));
        refuse_existing_output(&artifact_path)?;
        refuse_existing_output(&report_path)?;
        std::fs::write(&artifact_path, &executed.built.json_bytes).with_context(|| {
            format!(
                "cannot write calibration artifact: {}",
                artifact_path.display()
            )
        })?;
        let report = report_for(&resolved, &executed, &artifact_path);
        write_calibrate_report(&report_path, &report)?;
        let text =
            serde_json::to_string_pretty(&report).context("cannot encode calibrate report")?;
        crate::ui::success(format!(
            "calibration artifact published: {} (sha256 {}, {} bytes)",
            artifact_path.display(),
            executed.built.sha256_hex,
            executed.built.json_bytes.len()
        ));
        println!("{text}");
    }
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
        crate::cli::CalibrateCommand::Build {
            request,
            output,
            run,
            channel,
            model_id,
            reference_channel,
            reference_frequency_hz,
            reference_phase_rad,
            block_len,
            seed,
        } => {
            if let Some(request_path) = request {
                run_build(request_path, output.as_deref())
            } else {
                run_build_direct(&DirectBuildOptions {
                    run: run.clone(),
                    channels: channel.clone(),
                    output: output.clone(),
                    model_id: model_id.clone(),
                    reference_channel: *reference_channel,
                    reference_frequency_hz: *reference_frequency_hz,
                    reference_phase_rad: *reference_phase_rad,
                    block_len: *block_len,
                    seed: *seed,
                })
            }
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
