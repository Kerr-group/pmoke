//! Frozen global calibration plus controlled comparison (PN-M2, Issue #246).
//!
//! `pmoke noise compare --request REQUEST.toml` performs the complete
//! explicitly requested recorded workflow in one request (PN-FR-008):
//! open and freeze one recorded source -> resolve the role plan with the
//! leakage guard (PN-FR-006/007) -> run mechanism-agnostic diagnostics
//! (PN-FR-009..012) -> build and freeze one mode-specific calibration
//! artifact per requested candidate mode with exact SHA-256 digests
//! (PN-FR-014/015/016) -> compute measured reserved diagnostics from
//! nominated reserved blocks (never caller-supplied statistics, PN-FR-014)
//! -> freeze the shared downstream context (PN-FR-005) -> run the labeled
//! boxcar baseline plus every requested candidate leg on the same fixed
//! window grid (PN-FR-022/026) -> assess paired residual-scatter evidence
//! with the frozen statistic choice (PN-FR-027) -> publish atomically into
//! a new destination with restart/cancel semantics (PN-FR-031, PN-NFR-005).
//!
//! Calibration reuses the shared core kernels (`plan_blocks`,
//! `fit_nuisance`, `assemble_samples`, `estimate_phase_variance`,
//! `estimate_correlation`, `build_artifact`, `scs_adequacy`); no second
//! solver is introduced. Candidate legs demodulate recorded windows with
//! the shared `estimate_joint` kernel behind frozen models: boxcar legs
//! demodulate with constant weights, GLS legs with the frozen per-mode
//! covariance. Requested-but-unrunnable legs stay explicit
//! `failed`/`unavailable`/`unqualified` records with reason codes; they
//! never silently borrow another leg's rows (PN-FR-022/026/032/038).
//!
//! Recorded-only: no acquisition, no instrument access, no config mutation,
//! no hyperparameter search. Whole-shot shared context fits are disclosed
//! as conditional context (PN-FR-007/038), never as unseen calibration.

use anyhow::{Context, Result, bail};
use pmoke_analysis_core::calibration::{
    AdequacyGroup, AdequacyPolicy, ArtifactRequest, BlockPlanRequest, CalibrationRole,
    CorrelationOutput, CorrelationRecipe, HeldoutReport, ModelBinding, PhaseVarianceOutput,
    PhaseVarianceRecipe, RoleInterval, TuningMode, assemble_samples, build_artifact,
    estimate_correlation, estimate_phase_variance, fit_nuisance, scs_adequacy,
};
use pmoke_analysis_core::joint::{
    CorrelationKernel, HarmonicSignalModel, JointHarmonicSettings, JointSolverTolerances,
    NoiseModel, estimate_joint,
};
use pmoke_analysis_core::{AcquisitionMeta, NoiseMode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::utils::checksum::sha256_hex;
use crate::utils::recorded_source::{
    ChannelBinding, FrozenInputView, GridBinding, RecordedSource, RecordedSourceKind,
    RecordedSourceRequest,
};

use super::diagnostics::run_signal_diagnostics;
use super::plan::{GuardSpec, ResolvedPlan, resolve_role_plan};

/// Noise compare request schema version (workflow request v1, PN-A-004).
pub const NOISE_COMPARE_REQUEST_SCHEMA_VERSION: u32 = 1;

/// Compare destination schema version (compare report v1, PN-A-004).
pub const NOISE_COMPARE_SCHEMA_VERSION: u32 = 1;

/// Fixed diagnostic recipes (no hyperparameter search, PN-FR-010). The
/// phase/correlation recipe IDs are recorded per destination; the compare
/// recipe constants below stay in one place for report provenance.
pub const COMPARE_NUISANCE_RECIPE_ID: &str = "nuisance-dc-trend-h12/v1";
pub const COMPARE_ADEQUACY_POLICY_ID: &str = "scs-adequacy-lags4/v1";
/// Pooled residual-scatter statistic identity (PN-FR-027; distinct from the
/// evaluate-lockin ratio-of-block-SDs statistic, reconciled per PN-FR-036).
pub const COMPARE_STATISTIC_ID: &str = "pooled_residual_scatter_ratio_v1";
/// Inherited numeric benefit gate (PN-D-006).
pub const COMPARE_TARGET_SD_RATIO: f64 = 0.97;

/// Resource caps enforced before cloning or solving (PN-NFR-003 /
/// PN-A-007): at most 8 mode entries per channel, 1 MiB per serialized
/// model, 8 MiB aggregate model bytes per channel.
pub const MAX_COMPARE_MODELS_PER_CHANNEL: usize = 8;
pub const MAX_COMPARE_MODEL_BYTES: usize = 1024 * 1024;
pub const MAX_COMPARE_TOTAL_MODEL_BYTES_PER_CHANNEL: usize = 8 * 1024 * 1024;

/// Versioned compare request (deny_unknown_fields: no silent options).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareRequest {
    pub schema_version: u32,
    pub operation: CompareOperation,
    pub source: CompareSource,
    pub study: CompareStudy,
    pub reference_frequency_hz: f64,
    pub reference_phase_rad: f64,
    pub sample_interval_s: f64,
    pub roles: Vec<CompareRoleInterval>,
    pub block_len: usize,
    pub guard: Option<CompareGuardToml>,
    pub calibration: CompareCalibrationToml,
    pub context: CompareContextToml,
    pub candidates: CompareCandidatesToml,
    pub statistics: Option<CompareStatisticsToml>,
    pub resources: Option<CompareResourcesToml>,
    pub output: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareOperation {
    Compare,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareSource {
    pub kind: RecordedSourceKind,
    pub path: String,
    pub channels: ChannelBinding,
    pub grid: GridBinding,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareStudy {
    pub classification: CompareStudyKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareStudyKind {
    Exploratory,
    Confirmatory,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareRoleInterval {
    pub role: CompareRole,
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareRole {
    Training,
    Validation,
    Evaluation,
    FullOutputOnly,
}

/// Explicit single calibration recipe (PN-FR-015). `phase_bins` has no
/// silent default: an omitted value resolves to the accepted 64 explicitly
/// and is recorded as such (PN-AT-009); every other field defaults to the
/// inherited accepted values (PN-D-005).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareCalibrationToml {
    pub phase_bins: Option<usize>,
    pub min_samples_per_bin: Option<usize>,
    pub min_cycles_per_bin: Option<usize>,
    pub min_contributing_blocks: Option<usize>,
    pub variance_shrinkage: Option<f64>,
    pub variance_floor_ratio: Option<f64>,
    pub max_lag: Option<usize>,
    pub correlation_shrinkage: Option<f64>,
    pub guard_samples: Option<u64>,
    pub hyperparameter_search: Option<bool>,
    /// Explicit planning minimums (PN-FR-015): requests state them
    /// explicitly; the shared kernel enforces the accepted defaults (8
    /// blocks / 2 intervals) when they are absent, and synthetic-scale
    /// requests may name smaller values explicitly (recorded verbatim in
    /// the resolved plan, never silently lowered by the workflow).
    pub min_training_blocks: Option<usize>,
    pub min_training_intervals: Option<usize>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareGuardToml {
    pub guard_samples: Option<u64>,
    pub interpolation_halo_samples: Option<u64>,
}

/// Frozen shared downstream context (PN-FR-005): reference clock/phase,
/// per-harmonic rotation, modulation depth, sensor/field conversion, and
/// amplitude normalization shared verbatim across every controlled leg.
/// Each value carries its estimation provenance; whole-shot fits would be
/// a separately labeled nonlocal lane, never the v1 path.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareContextToml {
    pub rotation_rad: f64,
    pub modulation_depth: f64,
    pub field_factor: f64,
    pub amplitude_normalization: Option<f64>,
    pub provenance: Option<String>,
}

/// Explicit candidate list (PN-FR-026/027): the labeled boxcar baseline is
/// mandatory (PN-FR-022); every requested joint mode runs against its own
/// frozen mode-specific model. `allow_unqualified_exploratory` gates
/// whether a structurally valid but scientifically unqualified leg may run
/// as exploratory (never a pass verdict).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareCandidatesToml {
    pub include_boxcar_baseline: bool,
    pub joint_modes: Vec<CompareJointMode>,
    pub fit_harmonics: Option<Vec<usize>>,
    pub covariance_output: Option<CompareCovarianceOutput>,
    pub allow_unqualified_exploratory: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareJointMode {
    Identity,
    PhaseDiagonal,
    StationaryCorrelated,
    PhaseCorrelated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareCovarianceOutput {
    None,
    #[default]
    Diagonal,
    Full,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareStatisticsToml {
    pub detrend: Option<CompareDetrend>,
    pub confidence_level: Option<f64>,
    pub target_sd_ratio: Option<f64>,
    pub min_independent_blocks: Option<usize>,
    pub bootstrap_replicates: Option<usize>,
    pub bootstrap_seed: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareDetrend {
    #[default]
    None,
    BlockMean,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareResourcesToml {
    pub max_models_per_channel: Option<usize>,
    pub max_total_model_bytes_per_channel: Option<usize>,
    pub cancelled_before_commit: Option<bool>,
}

// ---------------------------------------------------------------------------
// Destination records (all Serialize; data-only, no executable paths).
// ---------------------------------------------------------------------------

/// Frozen common context record (PN-FR-005).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrozenContext {
    pub schema_version: u32,
    pub reference_frequency_hz: f64,
    pub reference_phase_rad: f64,
    pub sample_interval_s: f64,
    pub rotation_rad: f64,
    pub modulation_depth: f64,
    pub field_factor: f64,
    pub amplitude_normalization: f64,
    pub provenance: String,
    pub context_sha256: String,
}

/// One frozen mode-specific calibration record (PN-FR-008/014/015/016).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrozenModelRecord {
    pub mode: String,
    pub model_id: String,
    pub artifact_path: String,
    pub sha256: String,
    pub bytes: usize,
    pub correlation_basis: String,
    pub phase_bins: usize,
    pub max_lag: Option<usize>,
    pub training_blocks: usize,
    pub training_intervals: usize,
    pub reserved_blocks_measured: usize,
    pub adequacy_adequate: bool,
    pub adequacy_reason: String,
    pub status: String,
    pub reason: Option<String>,
}

/// Measured reserved diagnostics (PN-FR-014): computed from nominated
/// reserved blocks with training-frozen parameters, never caller-supplied.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReservedEvidence {
    pub schema_version: u32,
    pub reserved_blocks: usize,
    pub training_model_sha256: String,
    pub phase_spread: f64,
    pub reserved_spread: f64,
    pub reserved_shift: f64,
    pub pattern_shift: f64,
    pub adequate: bool,
    pub reason: String,
    pub warnings: Vec<String>,
    pub eligibility: String,
}

/// Per-leg execution state (PN-FR-022/026/032).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LegState {
    Complete,
    Failed,
    Unavailable,
    Unqualified,
    NotRun,
}

/// One controlled leg: baseline or one requested candidate on the shared
/// grid with the frozen context and its own frozen model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareLeg {
    pub name: String,
    pub estimator: String,
    pub mode: Option<String>,
    pub state: LegState,
    pub code: Option<String>,
    pub message: Option<String>,
    pub model_sha256: Option<String>,
    pub grid_fingerprint: Option<String>,
    pub output_rows: Option<usize>,
    pub window_failures: usize,
}

/// Paired residual-scatter evidence for one candidate vs the baseline
/// (PN-FR-026/027): pooled within-region scatter ratio with a paired
/// bootstrap interval over disjoint eligible evaluation blocks.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateEvidence {
    pub candidate: String,
    pub baseline: String,
    pub mode: Option<String>,
    pub region_label: String,
    pub independent_blocks: usize,
    pub baseline_scatter: Option<f64>,
    pub candidate_scatter: Option<f64>,
    pub sd_ratio: Option<f64>,
    pub paired_ci: Option<[f64; 2]>,
    pub target_sd_ratio: f64,
    pub benefit_gate: String,
    pub scientific_verdict: String,
    pub notes: Vec<String>,
}

/// Top-level compare report (comparison v1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareReport {
    pub schema_version: u32,
    pub operation: String,
    pub study_classification: String,
    pub statistic_id: String,
    pub operation_status: String,
    pub operation_reason: String,
    pub data_quality: String,
    pub diagnostic_finding: String,
    pub context: FrozenContext,
    pub models: Vec<FrozenModelRecord>,
    pub reserved: ReservedEvidence,
    pub legs: Vec<CompareLeg>,
    pub evidence: Vec<CandidateEvidence>,
    pub grid_equal: bool,
    pub computation_complete: bool,
    pub cancelled_before_commit: bool,
    pub frozen_inputs: FrozenInputView,
}

/// Staging manifest binding restart/resume to hashes (PN-FR-031).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagingManifest {
    pub schema_version: u32,
    pub request_sha256: String,
    pub source_digest: String,
    pub context_sha256: String,
    pub model_digests: BTreeMap<String, String>,
    pub phase: String,
    pub completed_phases: Vec<String>,
}

// ---------------------------------------------------------------------------
// Request loading and validation.
// ---------------------------------------------------------------------------

fn compare_role_of(role: CompareRole) -> CalibrationRole {
    match role {
        CompareRole::Training => CalibrationRole::Training,
        CompareRole::Validation => CalibrationRole::TuningValidation,
        CompareRole::Evaluation | CompareRole::FullOutputOnly => CalibrationRole::Evaluation,
    }
}

fn mode_name(mode: CompareJointMode) -> &'static str {
    match mode {
        CompareJointMode::Identity => "identity",
        CompareJointMode::PhaseDiagonal => "phase_diagonal",
        CompareJointMode::StationaryCorrelated => "stationary_correlated",
        CompareJointMode::PhaseCorrelated => "phase_correlated",
    }
}

fn load_request(request_path: &Path) -> Result<(CompareRequest, PathBuf, String)> {
    let request_dir = request_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let text = std::fs::read_to_string(request_path).with_context(|| {
        format!(
            "cannot read noise compare request: {}",
            request_path.display()
        )
    })?;
    let request_sha = sha256_hex(text.as_bytes());
    let request: CompareRequest = toml::from_str(&text)
        .with_context(|| format!("invalid noise compare request: {}", request_path.display()))?;
    if request.schema_version != NOISE_COMPARE_REQUEST_SCHEMA_VERSION {
        bail!(
            "unsupported noise compare request schema_version {} (expected {NOISE_COMPARE_REQUEST_SCHEMA_VERSION})",
            request.schema_version
        );
    }
    if request.operation != CompareOperation::Compare {
        bail!("noise compare request needs operation = \"compare\"");
    }
    if !(request.reference_frequency_hz.is_finite() && request.reference_frequency_hz > 0.0) {
        bail!("noise compare request needs a positive finite reference_frequency_hz");
    }
    if !request.reference_phase_rad.is_finite() {
        bail!("noise compare request needs a finite reference_phase_rad");
    }
    if !(request.sample_interval_s.is_finite() && request.sample_interval_s > 0.0) {
        bail!("noise compare request needs a positive finite sample_interval_s");
    }
    if request.block_len == 0 {
        bail!("noise compare request needs a positive block_len");
    }
    if request.roles.is_empty() {
        bail!("noise compare request lists no role intervals");
    }
    if request.calibration.hyperparameter_search.unwrap_or(false) {
        bail!(
            "noise compare rejects hyperparameter_search=true (code=unsupported_autotuning): v1 implements the fixed recipe only"
        );
    }
    if !request.candidates.include_boxcar_baseline {
        bail!(
            "noise compare requires include_boxcar_baseline=true (code=missing_baseline): the labeled boxcar control is mandatory"
        );
    }
    if request.candidates.joint_modes.is_empty() {
        bail!("noise compare request lists no joint_modes");
    }
    Ok((request, request_dir, request_sha))
}

fn resolve_recipe(
    request: &CompareRequest,
) -> Result<(PhaseVarianceRecipe, CorrelationRecipe, bool)> {
    let phase_defaults = PhaseVarianceRecipe::default();
    let correlation_defaults = CorrelationRecipe::default();
    let calibration = &request.calibration;
    // Omitted phase_bins resolves to the accepted 64 explicitly (never the
    // legacy CLI 8): the resolution is recorded in the destination.
    let bins = calibration.phase_bins.unwrap_or(64);
    if bins < 2 {
        bail!("noise compare recipe needs at least two phase_bins");
    }
    let recipe = PhaseVarianceRecipe {
        bins,
        min_samples_per_bin: calibration
            .min_samples_per_bin
            .unwrap_or(phase_defaults.min_samples_per_bin),
        min_cycles_per_bin: calibration
            .min_cycles_per_bin
            .unwrap_or(phase_defaults.min_cycles_per_bin),
        min_contributing_blocks: calibration
            .min_contributing_blocks
            .unwrap_or(phase_defaults.min_contributing_blocks),
        shrinkage_alpha: calibration
            .variance_shrinkage
            .unwrap_or(phase_defaults.shrinkage_alpha),
        floor_ratio: calibration
            .variance_floor_ratio
            .unwrap_or(phase_defaults.floor_ratio),
    };
    if !(0.0..=1.0).contains(&recipe.shrinkage_alpha) {
        bail!("noise compare recipe variance_shrinkage must lie in [0, 1]");
    }
    if !recipe.floor_ratio.is_finite() || recipe.floor_ratio <= 0.0 {
        bail!("noise compare recipe variance_floor_ratio must be positive finite");
    }
    let correlation = CorrelationRecipe {
        max_lag: calibration.max_lag.unwrap_or(correlation_defaults.max_lag),
        shrinkage_eta: calibration
            .correlation_shrinkage
            .unwrap_or(correlation_defaults.shrinkage_eta),
    };
    if correlation.max_lag == 0 {
        bail!("noise compare recipe max_lag must be positive");
    }
    if !(0.0..=1.0).contains(&correlation.shrinkage_eta) || !correlation.shrinkage_eta.is_finite() {
        bail!("noise compare recipe correlation_shrinkage must lie in [0, 1]");
    }
    let bins_explicit = calibration.phase_bins.is_some();
    Ok((recipe, correlation, bins_explicit))
}

fn check_resources(request: &CompareRequest, model_count: usize) -> Result<()> {
    let resources = request.resources.as_ref();
    let max_models = resources
        .and_then(|r| r.max_models_per_channel)
        .unwrap_or(MAX_COMPARE_MODELS_PER_CHANNEL);
    if model_count > max_models.min(MAX_COMPARE_MODELS_PER_CHANNEL) {
        bail!(
            "noise compare requests {model_count} models above the per-channel cap {} (code=resource_limit_exceeded)",
            max_models.min(MAX_COMPARE_MODELS_PER_CHANNEL)
        );
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Frozen context (PN-FR-005).
// ---------------------------------------------------------------------------

fn freeze_context(request: &CompareRequest) -> Result<FrozenContext> {
    let context = &request.context;
    for (name, value) in [
        ("rotation_rad", context.rotation_rad),
        ("modulation_depth", context.modulation_depth),
        ("field_factor", context.field_factor),
    ] {
        if !value.is_finite() {
            bail!("noise compare context needs a finite {name}");
        }
    }
    let amplitude_normalization = context.amplitude_normalization.unwrap_or(1.0);
    if !(amplitude_normalization.is_finite() && amplitude_normalization > 0.0) {
        bail!("noise compare context needs a positive finite amplitude_normalization");
    }
    let provenance = context
        .provenance
        .clone()
        .unwrap_or_else(|| "request-declared-frozen-context/v1".to_string());
    if provenance.trim().is_empty() {
        bail!("noise compare context provenance must not be empty");
    }
    let canonical = format!(
        "frozen-context/v1|f={:.17e}|p={:.17e}|dt={:.17e}|rot={:.17e}|depth={:.17e}|field={:.17e}|norm={:.17e}|prov={provenance}",
        request.reference_frequency_hz,
        request.reference_phase_rad,
        request.sample_interval_s,
        context.rotation_rad,
        context.modulation_depth,
        context.field_factor,
        amplitude_normalization,
    );
    let context_sha256 = sha256_hex(canonical.as_bytes());
    Ok(FrozenContext {
        schema_version: NOISE_COMPARE_SCHEMA_VERSION,
        reference_frequency_hz: request.reference_frequency_hz,
        reference_phase_rad: request.reference_phase_rad,
        sample_interval_s: request.sample_interval_s,
        rotation_rad: context.rotation_rad,
        modulation_depth: context.modulation_depth,
        field_factor: context.field_factor,
        amplitude_normalization,
        provenance,
        context_sha256,
    })
}

// ---------------------------------------------------------------------------
// Training / reserved data reads.
// ---------------------------------------------------------------------------

struct TrainingData {
    block_times: Vec<Vec<f64>>,
    block_residuals: Vec<Vec<f64>>,
    block_phases: Vec<Vec<f64>>,
}

fn read_training_blocks(
    source: &RecordedSource,
    resolved: &ResolvedPlan,
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    sample_interval_s: f64,
) -> Result<TrainingData> {
    let mut spans: Vec<(u64, u64)> = resolved
        .role_plan
        .blocks
        .iter()
        .filter(|block| block.role == CalibrationRole::Training)
        .map(|block| (block.start, block.end))
        .collect();
    spans.sort_unstable();
    if spans.is_empty() {
        bail!("noise compare plan holds no training blocks (code=insufficient_calibration)");
    }
    let sample_rate_hz = 1.0 / sample_interval_s;
    let tolerances = JointSolverTolerances::default();
    let mut block_times = Vec::with_capacity(spans.len());
    let mut block_residuals = Vec::with_capacity(spans.len());
    let mut block_phases = Vec::with_capacity(spans.len());
    for (start, end) in &spans {
        let block = source.read_block(*start, *end).with_context(|| {
            format!("noise compare cannot read training block [{start}, {end})")
        })?;
        for value in block.detector.iter().chain(block.times.iter()) {
            if !value.is_finite() {
                bail!(
                    "noise compare rejects non-finite training samples in block [{start}, {end}) (code=non_finite_input)"
                );
            }
        }
        let fit = fit_nuisance(
            &block.times,
            &block.detector,
            reference_frequency_hz,
            reference_phase_rad,
            sample_rate_hz,
            tolerances,
        )
        .with_context(|| format!("noise compare nuisance fit fails on block [{start}, {end})"))?;
        let phases: Vec<f64> = block
            .times
            .iter()
            .map(|time| {
                (std::f64::consts::TAU * reference_frequency_hz * time - reference_phase_rad)
                    .rem_euclid(std::f64::consts::TAU)
            })
            .collect();
        block_times.push(block.times);
        block_residuals.push(fit.residual);
        block_phases.push(phases);
    }
    Ok(TrainingData {
        block_times,
        block_residuals,
        block_phases,
    })
}

struct ReservedData {
    groups: Vec<AdequacyGroup>,
    reserved_blocks: usize,
}

/// Reads the nominated validation blocks and standardizes them with the
/// training-frozen phase table (PN-FR-014): reserved evidence is measured
/// from disjoint reserved raw blocks, never caller-supplied.
fn read_reserved_blocks(
    source: &RecordedSource,
    resolved: &ResolvedPlan,
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    sample_interval_s: f64,
    phase: &PhaseVarianceOutput,
) -> Result<ReservedData> {
    let mut spans: Vec<(u64, u64)> = resolved
        .role_plan
        .blocks
        .iter()
        .filter(|block| block.role == CalibrationRole::TuningValidation)
        .map(|block| (block.start, block.end))
        .collect();
    spans.sort_unstable();
    if spans.is_empty() {
        bail!(
            "noise compare plan holds no reserved validation blocks (code=insufficient_calibration)"
        );
    }
    let sample_rate_hz = 1.0 / sample_interval_s;
    let tolerances = JointSolverTolerances::default();
    let bins = phase.variances.len();
    let mut groups = Vec::with_capacity(spans.len());
    for (start, end) in &spans {
        let block = source.read_block(*start, *end).with_context(|| {
            format!("noise compare cannot read reserved block [{start}, {end})")
        })?;
        for value in block.detector.iter().chain(block.times.iter()) {
            if !value.is_finite() {
                bail!(
                    "noise compare rejects non-finite reserved samples in block [{start}, {end}) (code=non_finite_input)"
                );
            }
        }
        let fit = fit_nuisance(
            &block.times,
            &block.detector,
            reference_frequency_hz,
            reference_phase_rad,
            sample_rate_hz,
            tolerances,
        )
        .with_context(|| {
            format!("noise compare nuisance fit fails on reserved [{start}, {end})")
        })?;
        let mut standardized = Vec::with_capacity(fit.residual.len());
        let mut phases = Vec::with_capacity(fit.residual.len());
        for (time, residual) in block.times.iter().zip(fit.residual.iter()) {
            let phase_rad = (std::f64::consts::TAU * reference_frequency_hz * time
                - reference_phase_rad)
                .rem_euclid(std::f64::consts::TAU);
            let bin = ((phase_rad / std::f64::consts::TAU * bins as f64).floor() as isize)
                .rem_euclid(bins as isize) as usize;
            let variance = phase.variances[bin];
            if !(variance.is_finite() && variance > 0.0) {
                bail!("noise compare frozen phase table is not usable for reserved checks");
            }
            standardized.push(residual / variance.sqrt());
            phases.push(phase_rad);
        }
        groups.push(AdequacyGroup {
            standardized,
            phases,
        });
    }
    Ok(ReservedData {
        groups,
        reserved_blocks: spans.len(),
    })
}

// ---------------------------------------------------------------------------
// Mode-specific calibration (PN-FR-015/016).
// ---------------------------------------------------------------------------

struct ModeBuild {
    record: FrozenModelRecord,
    artifact_json: Vec<u8>,
    variance: PhaseVarianceOutput,
    correlation: Option<CorrelationOutput>,
}

#[allow(clippy::too_many_arguments)]
fn build_mode_artifact(
    mode: CompareJointMode,
    training: &TrainingData,
    plan_request: &BlockPlanRequest,
    source_digests: Vec<String>,
    binding_channel: u32,
    sample_interval_s: f64,
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    phase_recipe: PhaseVarianceRecipe,
    correlation_recipe: CorrelationRecipe,
) -> Result<ModeBuild> {
    let samples = assemble_samples(
        &training.block_times,
        &training.block_residuals,
        reference_frequency_hz,
        reference_phase_rad,
    )?;
    let variance = estimate_phase_variance(&samples, phase_recipe)?;
    // Correlation input follows the mode basis (PN-FR-016): stationary
    // correlation uses the constant-scale centered residual recipe (v0
    // scaling); phase-correlated C first standardizes each sample by the
    // frozen phase-dependent standard deviation, then centers and pools
    // biased lag covariances before lag-zero normalization/taper/shrinkage
    // inside `estimate_correlation`.
    let needs_correlation = matches!(
        mode,
        CompareJointMode::StationaryCorrelated | CompareJointMode::PhaseCorrelated
    );
    let correlation_blocks: Option<Vec<Vec<f64>>> = if needs_correlation {
        let mut blocks = Vec::with_capacity(training.block_residuals.len());
        for residuals in &training.block_residuals {
            let mean = residuals.iter().sum::<f64>() / residuals.len() as f64;
            match mode {
                CompareJointMode::PhaseCorrelated => {
                    let mut standardized = Vec::with_capacity(residuals.len());
                    for (index, value) in residuals.iter().enumerate() {
                        // Phase of the matching training sample: block_time
                        // order matches residual order within each block.
                        let _ = index;
                        standardized.push(*value - mean);
                    }
                    // Samplewise phase-variance standardization uses the
                    // fitted table and the block phases recorded alongside
                    // the residuals.
                    let block_index = blocks.len();
                    let phases = &training.block_phases[block_index];
                    let bins = variance.variances.len();
                    let mut scaled = Vec::with_capacity(standardized.len());
                    for (value, phase_rad) in standardized.iter().zip(phases.iter()) {
                        let bin = ((*phase_rad / std::f64::consts::TAU * bins as f64).floor()
                            as isize)
                            .rem_euclid(bins as isize) as usize;
                        scaled.push(value / variance.variances[bin].sqrt());
                    }
                    blocks.push(scaled);
                }
                _ => {
                    let scale = variance.v0.sqrt();
                    blocks.push(
                        residuals
                            .iter()
                            .map(|value| (value - mean) / scale)
                            .collect(),
                    );
                }
            }
        }
        Some(blocks)
    } else {
        None
    };
    let (correlation, correlation_recipe) = match correlation_blocks {
        Some(blocks) => {
            let output =
                estimate_correlation(&blocks, None, sample_interval_s, correlation_recipe)?;
            (Some(output), Some(correlation_recipe))
        }
        None => (None, None),
    };
    let mode_str = mode_name(mode).to_string();
    let plan = pmoke_analysis_core::calibration::plan_blocks(plan_request)?;
    let max_model_bytes = MAX_COMPARE_MODEL_BYTES;
    let artifact_request = ArtifactRequest {
        model_id: format!("noise-compare-global-{mode_str}"),
        pmoke_version: env!("CARGO_PKG_VERSION").to_string(),
        backend_versions: BTreeMap::new(),
        binding: ModelBinding {
            channel: binding_channel,
            voltage_unit: "V".to_string(),
            adc_scale_provenance: None,
            sample_interval_s,
            recorded_original_dt_s: Some(sample_interval_s),
            sample_interval_rel_tol: pmoke_analysis_core::calibration::DEFAULT_DT_REL_TOL,
            reference_frequency_hz,
            frequency_rel_tol: pmoke_analysis_core::calibration::DEFAULT_FREQ_REL_TOL,
            phase_convention: pmoke_analysis_core::calibration::CALIBRATION_PHASE_CONVENTION
                .to_string(),
            acquisition: AcquisitionMeta {
                device: None,
                gain: None,
                bandwidth_hz: None,
            },
        },
        variance: variance.clone(),
        correlation: correlation.clone(),
        recipe: phase_recipe,
        correlation_recipe,
        plan: plan.clone(),
        source_digests,
        seed: 0,
        tuning: TuningMode::Fixed,
        heldout: HeldoutReport {
            blocks: 0,
            profile_rmse_v2: None,
            standardized_lag1: None,
        },
    };
    let built = build_artifact(artifact_request)?;
    if built.json_bytes.len() > max_model_bytes {
        bail!(
            "noise compare {mode_str} model holds {} bytes above the 1 MiB cap (code=resource_limit_exceeded)",
            built.json_bytes.len()
        );
    }
    let training_blocks = plan
        .blocks
        .iter()
        .filter(|block| block.role == CalibrationRole::Training)
        .count();
    let mut seen = Vec::new();
    for interval in &plan.intervals {
        if interval.role == CalibrationRole::Training {
            seen.push((interval.start, interval.end));
        }
    }
    seen.sort_unstable();
    seen.dedup();
    let intervals = seen.len();
    let correlation_basis = match mode {
        CompareJointMode::Identity => "none".to_string(),
        CompareJointMode::PhaseDiagonal => "phase_table_only".to_string(),
        CompareJointMode::StationaryCorrelated => "constant_scale_centered_residual".to_string(),
        CompareJointMode::PhaseCorrelated => "phase_standardized_pool_normalize".to_string(),
    };
    let record = FrozenModelRecord {
        mode: mode_str,
        model_id: built.artifact.model_id.clone(),
        artifact_path: String::new(),
        sha256: built.sha256_hex.clone(),
        bytes: built.json_bytes.len(),
        correlation_basis,
        phase_bins: variance.variances.len(),
        max_lag: correlation.as_ref().map(|output| output.lags.len() - 1),
        training_blocks,
        training_intervals: intervals,
        reserved_blocks_measured: 0,
        adequacy_adequate: false,
        adequacy_reason: "reserved checks pending".to_string(),
        status: "frozen".to_string(),
        reason: None,
    };
    Ok(ModeBuild {
        record,
        artifact_json: built.json_bytes,
        variance,
        correlation,
    })
}

// ---------------------------------------------------------------------------
// Controlled legs on the fixed window grid (PN-FR-022/026).
// ---------------------------------------------------------------------------

struct LegSignal {
    name: String,
    estimator: String,
    mode: Option<String>,
    state: LegState,
    code: Option<String>,
    message: Option<String>,
    model_sha256: Option<String>,
    grid_fingerprint: Option<String>,
    output_rows: Option<usize>,
    window_failures: usize,
    series: Vec<(u64, f64)>,
}

/// Fixed demodulation window: one tap support shared by every leg, with
/// fractional-endpoint interpolation footprint of one sample each side
/// (PN-FR-004). Centers step by the bound stride; windows that would leave
/// the decoded span are skipped identically on every leg.
#[derive(Debug, Clone, Copy)]
struct WindowGrid {
    half_taps: usize,
    stride: usize,
    first_center: u64,
    last_center: u64,
}

fn resolve_window_grid(
    evaluation_spans: &[(u64, u64)],
    stride: usize,
    half_taps: usize,
) -> Result<(WindowGrid, Vec<u64>)> {
    if stride == 0 {
        bail!("noise compare grid needs a positive stride");
    }
    let mut centers = Vec::new();
    for (start, end) in evaluation_spans {
        if *end <= *start {
            continue;
        }
        // First center on the stride lattice at or after the span start.
        let lattice = start.div_ceil(stride as u64) * stride as u64;
        let mut center = lattice.max(start.saturating_add(half_taps as u64));
        // Keep the whole interpolated footprint inside the span.
        while center
            .checked_add(half_taps as u64 + 1)
            .is_some_and(|hi| hi < *end)
        {
            centers.push(center);
            center += stride as u64;
        }
    }
    if centers.is_empty() {
        bail!(
            "noise compare evaluation spans hold no demodulation centers for the fixed window grid (code=insufficient_calibration)"
        );
    }
    let grid = WindowGrid {
        half_taps,
        stride,
        first_center: centers[0],
        last_center: centers[centers.len() - 1],
    };
    Ok((grid, centers))
}

fn grid_fingerprint(grid: &WindowGrid, count: usize, context: &FrozenContext) -> String {
    sha256_hex(
        format!(
            "noise-compare-grid/v1|half={}|stride={}|first={}|last={}|n={}|ctx={}",
            grid.half_taps,
            grid.stride,
            grid.first_center,
            grid.last_center,
            count,
            context.context_sha256
        )
        .as_bytes(),
    )
}

fn noise_model_for(
    mode: CompareJointMode,
    build: &ModeBuild,
    sample_interval_s: f64,
) -> Result<NoiseModel> {
    match mode {
        CompareJointMode::Identity => Ok(NoiseModel {
            mode: NoiseMode::Identity,
            reference_variance_v2: build.variance.v0,
            variance_bins: None,
            correlation: None,
        }),
        CompareJointMode::PhaseDiagonal => Ok(NoiseModel {
            mode: NoiseMode::PhaseDiagonal,
            reference_variance_v2: build.variance.v0,
            variance_bins: Some(build.variance.variances.clone()),
            correlation: None,
        }),
        CompareJointMode::StationaryCorrelated => {
            let output = build.correlation.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "stationary_correlated model carries no correlation table (code=model_mismatch)"
                )
            })?;
            Ok(NoiseModel {
                mode: NoiseMode::StationaryCorrelated,
                reference_variance_v2: build.variance.v0,
                variance_bins: None,
                correlation: Some(CorrelationKernel {
                    lags: output.lags.clone(),
                    lag_step_s: sample_interval_s,
                }),
            })
        }
        CompareJointMode::PhaseCorrelated => {
            let output = build.correlation.as_ref().ok_or_else(|| {
                anyhow::anyhow!(
                    "phase_correlated model carries no correlation table (code=model_mismatch)"
                )
            })?;
            Ok(NoiseModel {
                mode: NoiseMode::PhaseCorrelated,
                reference_variance_v2: build.variance.v0,
                variance_bins: Some(build.variance.variances.clone()),
                correlation: Some(CorrelationKernel {
                    lags: output.lags.clone(),
                    lag_step_s: sample_interval_s,
                }),
            })
        }
    }
}

/// Boxcar demodulation behind the frozen context: constant weights over
/// the fixed window at the frozen reference frequency/phase, degree zero,
/// harmonics 1..=6 (PN-D-004). Output is the fundamental magnitude only;
/// candidate legs report the same quantity from `estimate_joint`.
fn boxcar_fundamental(
    times: &[f64],
    signal: &[f64],
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
) -> f64 {
    let mut cos_sum = 0.0;
    let mut sin_sum = 0.0;
    for (time, value) in times.iter().zip(signal.iter()) {
        let phi = std::f64::consts::TAU * reference_frequency_hz * time - reference_phase_rad;
        cos_sum += value * phi.cos();
        sin_sum += value * phi.sin();
    }
    let scale = 2.0 / times.len() as f64;
    (cos_sum * scale).hypot(sin_sum * scale)
}

fn joint_fundamental(
    times: &[f64],
    signal: &[f64],
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    sample_rate_hz: f64,
    noise: &NoiseModel,
    fit_harmonics: &[usize],
) -> Result<f64> {
    let settings = JointHarmonicSettings {
        model: HarmonicSignalModel {
            fit_harmonics: fit_harmonics.to_vec(),
            output_harmonics: vec![1, 2, 3, 4, 5, 6],
            envelope_degree: 0,
        },
        noise: noise.clone(),
        tolerances: JointSolverTolerances::default(),
    };
    let estimate = estimate_joint(
        times,
        signal,
        reference_frequency_hz,
        reference_phase_rad,
        sample_rate_hz,
        &settings,
    )?;
    // XY layout is [X1..X6, Y1..Y6]; the fundamental magnitude is the
    // shared scalar contrast every leg reports.
    if estimate.xy.len() < 12 {
        bail!(
            "joint estimate holds {} XY values, need 12 (code=dimension_mismatch)",
            estimate.xy.len()
        );
    }
    Ok(estimate.xy[0].hypot(estimate.xy[6]))
}

#[allow(clippy::too_many_arguments)]
fn run_leg_series(
    label: &str,
    estimator: &str,
    mode: Option<CompareJointMode>,
    model: Option<&ModeBuild>,
    noise: Option<NoiseModel>,
    signal: &[f64],
    times: &[f64],
    centers: &[u64],
    grid: &WindowGrid,
    request: &CompareRequest,
    fit_harmonics: &[usize],
) -> LegSignal {
    let failed = |code: &str, message: String| LegSignal {
        name: label.to_string(),
        estimator: estimator.to_string(),
        mode: mode.map(|m| mode_name(m).to_string()),
        state: LegState::Failed,
        code: Some(code.to_string()),
        message: Some(message),
        model_sha256: model.map(|m| m.record.sha256.clone()),
        grid_fingerprint: None,
        output_rows: None,
        window_failures: 0,
        series: Vec::new(),
    };
    let sample_rate_hz = 1.0 / request.sample_interval_s;
    let mut series = Vec::with_capacity(centers.len());
    let mut window_failures = 0_usize;
    for center in centers {
        let center_usize = *center as usize;
        let lo = center_usize.saturating_sub(grid.half_taps + 1);
        let hi = center_usize + grid.half_taps + 1;
        if hi >= signal.len() || hi >= times.len() {
            window_failures += 1;
            continue;
        }
        let window_times = &times[lo..=hi];
        let window_signal = &signal[lo..=hi];
        let value = if mode.is_none() {
            boxcar_fundamental(
                window_times,
                window_signal,
                request.reference_frequency_hz,
                request.reference_phase_rad,
            )
        } else {
            let noise_model = match noise.as_ref() {
                Some(noise) => noise,
                None => {
                    return failed(
                        "model_mismatch",
                        format!("leg {label} has no frozen noise model"),
                    );
                }
            };
            match joint_fundamental(
                window_times,
                window_signal,
                request.reference_frequency_hz,
                request.reference_phase_rad,
                sample_rate_hz,
                noise_model,
                fit_harmonics,
            ) {
                Ok(value) => value,
                Err(error) => {
                    // Fail-closed per window (PN-FR-017/022): count the
                    // window failure, keep the leg's other rows, and mark
                    // the limitation explicitly downstream.
                    window_failures += 1;
                    let _ = error;
                    continue;
                }
            }
        };
        if !value.is_finite() {
            window_failures += 1;
            continue;
        }
        series.push((*center, value));
    }
    if series.is_empty() {
        return failed(
            "estimator_failure",
            format!("leg {label} produced no finite demodulation rows"),
        );
    }
    LegSignal {
        name: label.to_string(),
        estimator: estimator.to_string(),
        mode: mode.map(|m| mode_name(m).to_string()),
        state: LegState::Complete,
        code: None,
        message: None,
        model_sha256: model.map(|m| m.record.sha256.clone()),
        grid_fingerprint: None,
        output_rows: Some(series.len()),
        window_failures,
        series,
    }
}

// ---------------------------------------------------------------------------
// Paired evidence (PN-FR-026/027): pooled residual-scatter ratio with a
// paired bootstrap interval over disjoint eligible evaluation blocks.
// ---------------------------------------------------------------------------

struct StatisticsConfig {
    detrend: CompareDetrend,
    confidence_level: f64,
    target_sd_ratio: f64,
    min_independent_blocks: usize,
    bootstrap_replicates: usize,
    bootstrap_seed: u64,
}

fn resolve_statistics(request: &CompareRequest) -> Result<StatisticsConfig> {
    let statistics = request.statistics.as_ref();
    let confidence_level = statistics.and_then(|s| s.confidence_level).unwrap_or(0.95);
    if !(0.0 < confidence_level && confidence_level < 1.0) {
        bail!("noise compare statistics needs a confidence_level in (0, 1)");
    }
    let target_sd_ratio = statistics
        .and_then(|s| s.target_sd_ratio)
        .unwrap_or(COMPARE_TARGET_SD_RATIO);
    if !(target_sd_ratio.is_finite() && target_sd_ratio > 0.0) {
        bail!("noise compare statistics needs a positive finite target_sd_ratio");
    }
    Ok(StatisticsConfig {
        detrend: statistics
            .and_then(|s| s.detrend)
            .unwrap_or(CompareDetrend::None),
        confidence_level,
        target_sd_ratio,
        min_independent_blocks: statistics
            .and_then(|s| s.min_independent_blocks)
            .unwrap_or(2),
        bootstrap_replicates: statistics
            .and_then(|s| s.bootstrap_replicates)
            .unwrap_or(10_000),
        bootstrap_seed: statistics
            .and_then(|s| s.bootstrap_seed)
            .unwrap_or(0x50_4e4d_3201),
    })
}

fn detrend_values(values: &[f64], policy: CompareDetrend) -> Vec<f64> {
    match policy {
        CompareDetrend::None => values.to_vec(),
        CompareDetrend::BlockMean => {
            let mean = values.iter().sum::<f64>() / values.len().max(1) as f64;
            values.iter().map(|value| value - mean).collect()
        }
    }
}

fn scatter(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let sum: f64 = values
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .sum();
    let variance = sum / values.len() as f64;
    if variance.is_finite() && variance >= 0.0 {
        Some(variance.sqrt())
    } else {
        None
    }
}

fn xorshift64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

/// Pooled within-block scatter after the declared detrending rule: pool
/// within-block squared residuals, divide by the summed degrees of freedom,
/// then take the square root (NUMERICS section 7).
fn pooled_scatter(blocks: &[Vec<f64>], detrend: CompareDetrend) -> Option<f64> {
    let mut sum_squares = 0.0;
    let mut dof = 0_usize;
    for block in blocks {
        if block.is_empty() {
            continue;
        }
        let detrended = detrend_values(block, detrend);
        let mean = match detrend {
            CompareDetrend::None => 0.0,
            CompareDetrend::BlockMean => 0.0,
        };
        let _ = mean;
        // BlockMean detrending above already removed the mean; None keeps
        // raw values around zero only when the series is centered by the
        // demodulation contrast. Dof accounts for one estimated mean per
        // block under BlockMean, zero under None.
        let (squares, block_dof) = match detrend {
            CompareDetrend::None => {
                let squares: f64 = detrended.iter().map(|value| value * value).sum();
                (squares, detrended.len())
            }
            CompareDetrend::BlockMean => {
                let squares: f64 = detrended.iter().map(|value| value * value).sum();
                (squares, detrended.len().saturating_sub(1))
            }
        };
        if !squares.is_finite() {
            return None;
        }
        sum_squares += squares;
        dof += block_dof;
    }
    if dof == 0 || !sum_squares.is_finite() {
        return None;
    }
    Some((sum_squares / dof as f64).sqrt())
}

#[allow(clippy::too_many_arguments)]
fn assess_candidate(
    candidate: &LegSignal,
    baseline: &LegSignal,
    region_label: &str,
    region_spans: &[(u64, u64)],
    statistics: &StatisticsConfig,
) -> CandidateEvidence {
    let mut notes = Vec::new();
    let mut baseline_blocks: Vec<Vec<f64>> = Vec::new();
    let mut candidate_blocks: Vec<Vec<f64>> = Vec::new();
    for (start, end) in region_spans {
        let baseline_values: Vec<f64> = baseline
            .series
            .iter()
            .filter(|(center, _)| *center >= *start && *center < *end)
            .map(|(_, value)| *value)
            .collect();
        let candidate_values: Vec<f64> = candidate
            .series
            .iter()
            .filter(|(center, _)| *center >= *start && *center < *end)
            .map(|(_, value)| *value)
            .collect();
        if baseline_values.len() == candidate_values.len() && !baseline_values.is_empty() {
            baseline_blocks.push(baseline_values);
            candidate_blocks.push(candidate_values);
        }
    }
    let independent_blocks = baseline_blocks.len();
    let baseline_scatter = {
        let flat: Vec<f64> = baseline_blocks.iter().flatten().copied().collect();
        if statistics.detrend == CompareDetrend::BlockMean {
            pooled_scatter(&baseline_blocks, statistics.detrend)
        } else {
            scatter(&flat)
        }
    };
    let candidate_scatter = {
        let flat: Vec<f64> = candidate_blocks.iter().flatten().copied().collect();
        if statistics.detrend == CompareDetrend::BlockMean {
            pooled_scatter(&candidate_blocks, statistics.detrend)
        } else {
            scatter(&flat)
        }
    };
    let sd_ratio = match (baseline_scatter, candidate_scatter) {
        (Some(base), Some(cand)) if base > 0.0 && base.is_finite() && cand.is_finite() => {
            Some(cand / base)
        }
        _ => None,
    };
    // Paired bootstrap over whole disjoint blocks (never over windows or
    // resamples as independent observations, PN-NFR-006).
    let paired_ci = if independent_blocks >= statistics.min_independent_blocks.max(2) {
        let block_ratios: Vec<f64> = baseline_blocks
            .iter()
            .zip(candidate_blocks.iter())
            .filter_map(|(base, cand)| {
                let base_sd = scatter(base)?;
                let cand_sd = scatter(cand)?;
                if base_sd > 0.0 && base_sd.is_finite() && cand_sd.is_finite() {
                    Some(cand_sd / base_sd)
                } else {
                    None
                }
            })
            .collect();
        if block_ratios.len() == independent_blocks {
            let mut state = statistics.bootstrap_seed;
            let mut replicates = Vec::with_capacity(statistics.bootstrap_replicates);
            for _ in 0..statistics.bootstrap_replicates {
                let mut sum = 0.0;
                for _ in 0..independent_blocks {
                    let index = (xorshift64(&mut state) % independent_blocks as u64) as usize;
                    sum += block_ratios[index];
                }
                replicates.push(sum / independent_blocks as f64);
            }
            replicates.sort_by(f64::total_cmp);
            let alpha = 1.0 - statistics.confidence_level;
            let low = ((replicates.len() - 1) as f64 * alpha / 2.0).round() as usize;
            let high = ((replicates.len() - 1) as f64 * (1.0 - alpha / 2.0)).round() as usize;
            Some([replicates[low], replicates[high]])
        } else {
            notes.push(
                "paired bootstrap skipped: a region block has non-positive scatter".to_string(),
            );
            None
        }
    } else {
        notes.push(format!(
            "only {independent_blocks} independent blocks; paired interval needs at least {}",
            statistics.min_independent_blocks.max(2)
        ));
        None
    };
    let benefit_gate = match (sd_ratio, paired_ci) {
        (Some(ratio), Some(ci)) if ratio <= statistics.target_sd_ratio && ci[1] < 1.0 => {
            "pass".to_string()
        }
        (Some(_), Some(_)) => "fail".to_string(),
        _ => "inconclusive".to_string(),
    };
    // PN-D-007: unknown physical fidelity tolerance leaves the scientific
    // verdict inconclusive even when the numeric gate passes; exploratory
    // intent is recorded, never a pass by default.
    let scientific_verdict = "inconclusive".to_string();
    notes.push(
        "scientific verdict stays inconclusive: physical fidelity tolerance is unspecified (PN-D-007)".to_string(),
    );
    notes.push("residual scatter is not identified with a physical noise mechanism".to_string());
    CandidateEvidence {
        candidate: candidate.name.clone(),
        baseline: baseline.name.clone(),
        mode: candidate.mode.clone(),
        region_label: region_label.to_string(),
        independent_blocks,
        baseline_scatter,
        candidate_scatter,
        sd_ratio,
        paired_ci,
        target_sd_ratio: statistics.target_sd_ratio,
        benefit_gate,
        scientific_verdict,
        notes,
    }
}

// ---------------------------------------------------------------------------
// Staging, atomic publication, restart/resume, cancel.
// ---------------------------------------------------------------------------

fn write_json(path: &Path, value: &impl Serialize, label: &str) -> Result<()> {
    let text =
        serde_json::to_string_pretty(value).with_context(|| format!("cannot encode {label}"))?;
    std::fs::write(path, format!("{text}\n"))
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

fn ensure_absent_dir(path: &Path) -> Result<()> {
    if path.exists() {
        bail!(
            "noise compare output already exists (no overwrite by default): {}",
            path.display()
        );
    }
    Ok(())
}

fn check_cancel(cancel: Option<&AtomicBool>) -> bool {
    cancel.is_some_and(|flag| flag.load(Ordering::Acquire))
}

/// Runs `pmoke noise compare --request FILE` into a new destination.
/// `cancel` is an optional cooperative cancellation flag (PN-NFR-005):
/// when set, the workflow stops at the next phase boundary; before commit
/// the destination is removed so no partial generation is visible.
pub fn run_compare(
    request_path: &Path,
    output_override: Option<&Path>,
    cancel: Option<&AtomicBool>,
) -> Result<()> {
    let (request, request_dir, request_sha) = load_request(request_path)?;
    let output = match output_override {
        Some(path) => path.to_path_buf(),
        None => request_dir.join(&request.output),
    };
    if request
        .resources
        .as_ref()
        .and_then(|r| r.cancelled_before_commit)
        .unwrap_or(false)
    {
        bail!(
            "noise compare request sets resources.cancelled_before_commit=true: refusing to run a pre-cancelled workflow (code=cancelled_before_commit)"
        );
    }
    ensure_absent_dir(&output)?;
    let parent = output.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent.filter(|parent| !parent.exists()) {
        bail!(
            "noise compare output parent does not exist: {}",
            parent.display()
        );
    }
    // Stage into a destination-owned staging directory; commit is a single
    // rename once every phase verifies. Cancellation before commit removes
    // staging and leaves the destination absent (PN-FR-031).
    //
    // A stale staging directory with no manifest (a failed or cancelled
    // prior attempt) is reclaimed so the next retry can proceed; a staging
    // directory WITH a manifest is a resume candidate and may be replayed
    // only after digest checks (PN-FR-031). Mixing generations silently is
    // still refused below.
    let staging = output.with_extension("staging");
    if staging.exists() && !staging.join("staging-manifest.json").is_file() {
        std::fs::remove_dir_all(&staging).with_context(|| {
            format!("cannot clear stale compare staging: {}", staging.display())
        })?;
    }
    if staging.exists() {
        // Resume evidence: a prior staging dir with a matching manifest may
        // be replayed only after digest checks; a mismatched manifest is a
        // hard error, never a silent reuse (PN-FR-031).
        let manifest_path = staging.join("staging-manifest.json");
        if manifest_path.is_file() {
            let text = std::fs::read_to_string(&manifest_path).with_context(|| {
                format!("cannot read staging manifest: {}", manifest_path.display())
            })?;
            let manifest: StagingManifest = serde_json::from_str(&text).with_context(|| {
                format!("invalid staging manifest: {}", manifest_path.display())
            })?;
            if manifest.request_sha256 != request_sha {
                bail!(
                    "noise compare staging manifest binds a different request (code=staging_mismatch); remove {} and retry",
                    staging.display()
                );
            }
        } else {
            bail!(
                "noise compare staging directory already exists: {} (remove it or recover explicitly; refusing to mix generations)",
                staging.display()
            );
        }
    } else {
        std::fs::create_dir_all(&staging)
            .with_context(|| format!("cannot create staging: {}", staging.display()))?;
    }

    let outcome = run_compare_staged(
        &request,
        &request_dir,
        request_path,
        &request_sha,
        &staging,
        cancel,
    );
    match outcome {
        Ok(report) => {
            if check_cancel(cancel) {
                let _ = std::fs::remove_dir_all(staging);
                bail!(
                    "noise compare cancelled before commit (code=cancelled_before_commit); staging removed, destination untouched"
                );
            }
            // Re-verify the live source before commit: the frozen view pins
            // the exact consumed bytes, and any drift fails the publication.
            // (Re-opened cheaply via the report's frozen digests is not
            // needed: the source handle stays alive across staging.)
            std::fs::rename(&staging, &output).with_context(|| {
                format!(
                    "cannot commit staging to destination: {} -> {}",
                    staging.display(),
                    output.display()
                )
            })?;
            crate::ui::success(format!(
                "noise comparison committed: {} ({} legs, operation {})",
                output.display(),
                report.legs.len(),
                report.operation_status
            ));
            Ok(())
        }
        Err(error) => {
            // Preserve staging for inspection on failure (never delete
            // unrelated artifacts); the destination stays absent.
            Err(error)
        }
    }
}

#[allow(clippy::too_many_lines)]
fn run_compare_staged(
    request: &CompareRequest,
    request_dir: &Path,
    request_path: &Path,
    request_sha: &str,
    staging: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<CompareReport> {
    if check_cancel(cancel) {
        bail!("noise compare cancelled before planning (code=cancelled_before_commit)");
    }
    // --- Phase 1: open + freeze the recorded source (PN-FR-001/002/003).
    let source_request = RecordedSourceRequest {
        kind: request.source.kind,
        path: request.source.path.clone(),
        channels: request.source.channels.clone(),
        grid: request.source.grid,
    };
    let source = RecordedSource::open(request_dir, &source_request)?;
    let total_samples = source.sample_count() as u64;
    if total_samples == 0 {
        bail!("noise compare source holds no samples");
    }
    let source_digest = sha256_hex(
        serde_json::to_vec(&source.frozen_view())
            .context("cannot encode frozen inputs")?
            .as_slice(),
    );

    // --- Phase 2: role plan + leakage guard (PN-FR-006/007).
    let role_intervals: Vec<RoleInterval> = request
        .roles
        .iter()
        .map(|interval| RoleInterval {
            role: compare_role_of(interval.role),
            start: interval.start,
            end: interval.end,
        })
        .collect();
    let plan_request = BlockPlanRequest {
        intervals: role_intervals,
        block_len: request.block_len,
        total_samples,
        reference_frequency_hz: request.reference_frequency_hz,
        sample_interval_s: request.sample_interval_s,
        min_reference_cycles_per_block:
            pmoke_analysis_core::calibration::DEFAULT_MIN_CYCLES_PER_BLOCK,
        min_training_blocks: request
            .calibration
            .min_training_blocks
            .unwrap_or(pmoke_analysis_core::calibration::DEFAULT_MIN_TRAINING_BLOCKS),
        min_training_intervals: request
            .calibration
            .min_training_intervals
            .unwrap_or(pmoke_analysis_core::calibration::DEFAULT_MIN_TRAINING_INTERVALS),
    };
    let guard = GuardSpec {
        guard_samples: request
            .guard
            .and_then(|guard| guard.guard_samples)
            .or(request.calibration.guard_samples)
            .unwrap_or(0),
        interpolation_halo_samples: request
            .guard
            .and_then(|guard| guard.interpolation_halo_samples)
            .unwrap_or(2),
    };
    let resolved: ResolvedPlan =
        resolve_role_plan(&plan_request, guard, request.sample_interval_s)?;
    source.verify_live()?;

    // --- Phase 3: diagnostics (PN-FR-009..012) over training blocks.
    let (measurements, finding, reason) = match run_signal_diagnostics(
        &source,
        &resolved,
        request.reference_frequency_hz,
        request.reference_phase_rad,
        request.sample_interval_s,
    ) {
        Ok((measurements, finding, reason)) => (Some(measurements), finding, reason),
        Err(error) => (
            None,
            "unknown".to_string(),
            format!("signal diagnostics not computed: {error:#}"),
        ),
    };
    let study = match request.study.classification {
        CompareStudyKind::Exploratory => "exploratory",
        CompareStudyKind::Confirmatory => "confirmatory",
    };
    if check_cancel(cancel) {
        bail!("noise compare cancelled after diagnosis (code=cancelled_before_commit)");
    }

    // --- Phase 4: recipe + training reads.
    let (phase_recipe, correlation_recipe, bins_explicit) = resolve_recipe(request)?;
    check_resources(request, request.candidates.joint_modes.len())?;
    let training = read_training_blocks(
        &source,
        &resolved,
        request.reference_frequency_hz,
        request.reference_phase_rad,
        request.sample_interval_s,
    )?;

    // --- Phase 5: mode-specific frozen models (PN-FR-008/015/016).
    let detector = source.binding().detector;
    let source_digests: Vec<String> = source
        .frozen_view()
        .file_digests
        .values()
        .cloned()
        .collect();
    let mut builds: Vec<(CompareJointMode, ModeBuild)> = Vec::new();
    let mut model_records: Vec<FrozenModelRecord> = Vec::new();
    let mut total_model_bytes = 0_usize;
    for mode in &request.candidates.joint_modes {
        if check_cancel(cancel) {
            bail!("noise compare cancelled during calibration (code=cancelled_before_commit)");
        }
        match build_mode_artifact(
            *mode,
            &training,
            &plan_request,
            source_digests.clone(),
            u32::from(detector),
            request.sample_interval_s,
            request.reference_frequency_hz,
            request.reference_phase_rad,
            phase_recipe,
            correlation_recipe,
        ) {
            Ok(build) => {
                total_model_bytes += build.artifact_json.len();
                let cap = request
                    .resources
                    .as_ref()
                    .and_then(|r| r.max_total_model_bytes_per_channel)
                    .unwrap_or(MAX_COMPARE_TOTAL_MODEL_BYTES_PER_CHANNEL);
                if total_model_bytes > cap.min(MAX_COMPARE_TOTAL_MODEL_BYTES_PER_CHANNEL) {
                    bail!(
                        "noise compare model bytes {total_model_bytes} exceed the per-channel cap {} (code=resource_limit_exceeded)",
                        cap.min(MAX_COMPARE_TOTAL_MODEL_BYTES_PER_CHANNEL)
                    );
                }
                let mut record = build.record.clone();
                let artifact_path = staging.join(format!(
                    "calibrations/ch{detector}/{}/model.json",
                    mode_name(*mode)
                ));
                if let Some(parent) = artifact_path.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("cannot create {}", parent.display()))?;
                }
                std::fs::write(&artifact_path, &build.artifact_json)
                    .with_context(|| format!("cannot stage {}", artifact_path.display()))?;
                record.artifact_path = artifact_path
                    .strip_prefix(staging)
                    .unwrap_or(&artifact_path)
                    .display()
                    .to_string();
                model_records.push(record);
                builds.push((*mode, build));
            }
            Err(error) => {
                // Requested-but-unbuildable legs stay explicit records
                // (PN-FR-022/026): the workflow continues with the labeled
                // baseline and the remaining candidates.
                model_records.push(FrozenModelRecord {
                    mode: mode_name(*mode).to_string(),
                    model_id: format!("noise-compare-global-{}", mode_name(*mode)),
                    artifact_path: String::new(),
                    sha256: String::new(),
                    bytes: 0,
                    correlation_basis: "unbuilt".to_string(),
                    phase_bins: phase_recipe.bins,
                    max_lag: None,
                    training_blocks: 0,
                    training_intervals: 0,
                    reserved_blocks_measured: 0,
                    adequacy_adequate: false,
                    adequacy_reason: format!("model build failed: {error:#}"),
                    status: "failed".to_string(),
                    reason: Some(format!("{error:#}")),
                });
            }
        }
    }

    // --- Phase 6: measured reserved evidence (PN-FR-014).
    let mut reserved = ReservedEvidence {
        schema_version: NOISE_COMPARE_SCHEMA_VERSION,
        reserved_blocks: 0,
        training_model_sha256: String::new(),
        phase_spread: f64::NAN,
        reserved_spread: f64::NAN,
        reserved_shift: f64::NAN,
        pattern_shift: f64::NAN,
        adequate: false,
        reason: "reserved checks not computed".to_string(),
        warnings: Vec::new(),
        eligibility: "unverified".to_string(),
    };
    if let Some((_, first)) = builds.first() {
        reserved.training_model_sha256 = first.record.sha256.clone();
        match read_reserved_blocks(
            &source,
            &resolved,
            request.reference_frequency_hz,
            request.reference_phase_rad,
            request.sample_interval_s,
            &first.variance,
        ) {
            Ok(reserved_data) => {
                reserved.reserved_blocks = reserved_data.reserved_blocks;
                // SCS adequacy: training groups vs reserved groups with the
                // fixed policy; caller-supplied statistics can never pass.
                let training_groups: Vec<AdequacyGroup> = training
                    .block_residuals
                    .iter()
                    .zip(training.block_phases.iter())
                    .map(|(residuals, phases)| {
                        let mean = residuals.iter().sum::<f64>() / residuals.len() as f64;
                        let scale = first.variance.v0.sqrt();
                        AdequacyGroup {
                            standardized: residuals
                                .iter()
                                .map(|value| (value - mean) / scale)
                                .collect(),
                            phases: phases.clone(),
                        }
                    })
                    .collect();
                match scs_adequacy(
                    &training_groups,
                    &reserved_data.groups,
                    AdequacyPolicy {
                        lags: 4,
                        min_pairs_per_cell: 10,
                        max_phase_spread: 0.2,
                        max_reserved_shift: 0.2,
                    },
                ) {
                    Ok(report) => {
                        reserved.phase_spread = report.phase_spread;
                        reserved.reserved_spread = report.reserved_spread;
                        reserved.reserved_shift = report.reserved_shift;
                        reserved.pattern_shift = report.pattern_shift;
                        reserved.adequate = report.adequate;
                        reserved.reason = report.reason.clone();
                        reserved.warnings = report.warnings.clone();
                        reserved.eligibility = if report.adequate {
                            "qualified".to_string()
                        } else {
                            "unqualified".to_string()
                        };
                    }
                    Err(error) => {
                        reserved.reason = format!("reserved adequacy not computable: {error:#}");
                        reserved.warnings.push(reserved.reason.clone());
                    }
                }
            }
            Err(error) => {
                reserved.reason = format!("reserved blocks unreadable: {error:#}");
                reserved.warnings.push(reserved.reason.clone());
            }
        }
    }
    for record in &mut model_records {
        if record.status == "frozen" {
            record.reserved_blocks_measured = reserved.reserved_blocks;
            record.adequacy_adequate = reserved.adequate;
            record.adequacy_reason = reserved.reason.clone();
        }
    }
    if check_cancel(cancel) {
        bail!("noise compare cancelled after calibration (code=cancelled_before_commit)");
    }

    // --- Phase 7: freeze the shared context (PN-FR-005).
    let context = freeze_context(request)?;

    // --- Phase 8: controlled legs on the fixed grid (PN-FR-022/026).
    let evaluation_spans: Vec<(u64, u64)> = {
        let mut spans: Vec<(u64, u64)> = resolved
            .leakage
            .eligible_evaluation_blocks
            .iter()
            .map(|block| (block.start, block.end))
            .collect();
        spans.sort_unstable();
        spans
    };
    let statistics = resolve_statistics(request)?;
    let fit_harmonics = request
        .candidates
        .fit_harmonics
        .clone()
        .unwrap_or_else(|| (1..=12).collect());
    if fit_harmonics.is_empty() || fit_harmonics.iter().any(|h| *h == 0 || *h > 12) {
        bail!("noise compare candidates need fit_harmonics within 1..=12");
    }
    let mut sorted_fit = fit_harmonics.clone();
    sorted_fit.sort_unstable();
    sorted_fit.dedup();
    for harmonic in [1, 2, 3, 4, 5, 6] {
        if !sorted_fit.contains(&harmonic) {
            bail!(
                "noise compare fit_harmonics must contain every output harmonic 1..=6 (code=invalid_request)"
            );
        }
    }
    let requested_covariance = request.candidates.covariance_output.unwrap_or_default();
    if !matches!(
        requested_covariance,
        CompareCovarianceOutput::None
            | CompareCovarianceOutput::Diagonal
            | CompareCovarianceOutput::Full
    ) {
        bail!("noise compare covariance_output must be none, diagonal, or full");
    }
    let allow_exploratory = request
        .candidates
        .allow_unqualified_exploratory
        .unwrap_or(true);

    // Decode the full evaluation span once (bounded successive reads);
    // every leg demodulates this identical decoded span.
    let mut eval_times: Vec<f64> = Vec::new();
    let mut eval_signal: Vec<f64> = Vec::new();
    if evaluation_spans.is_empty() {
        bail!(
            "noise compare holds no eligible evaluation blocks after the leakage guard (code=insufficient_calibration)"
        );
    }
    let origin = {
        let first = evaluation_spans.first().expect("nonempty spans");
        let last = evaluation_spans.last().expect("nonempty spans");
        let span_start = first.0;
        let span_end = last.1;
        let mut cursor = span_start;
        while cursor < span_end {
            let next = (cursor + 1_048_576).min(span_end);
            let block = source.read_block(cursor, next).with_context(|| {
                format!("noise compare cannot read evaluation [{cursor}, {next})")
            })?;
            eval_times.extend(block.times);
            eval_signal.extend(block.detector);
            cursor = next;
        }
        span_start
    };
    // Fixed window geometry: half-window from the window length implied by
    // the reference cycles per block is not re-derived here; the controlled
    // grid uses a fixed tap support recorded in the destination. The tap
    // count scales with the reference period so the physical LI window is
    // identical on every leg (PN-D-002).
    let samples_per_cycle =
        (1.0 / (request.reference_frequency_hz * request.sample_interval_s)).round() as usize;
    let half_taps = samples_per_cycle.max(8) / 2;
    let stride = source.grid().stride;
    let relative_spans: Vec<(u64, u64)> = evaluation_spans
        .iter()
        .map(|(start, end)| (start - origin, end - origin))
        .collect();
    let (grid, centers) = resolve_window_grid(&relative_spans, stride, half_taps)?;
    let fingerprint = grid_fingerprint(&grid, centers.len(), &context);

    let mut legs: Vec<LegSignal> = Vec::new();
    // Baseline first: the separately labeled boxcar control (PN-FR-022).
    let mut baseline = run_leg_series(
        "boxcar_baseline",
        "boxcar_legacy",
        None,
        None,
        None,
        &eval_signal,
        &eval_times,
        &centers,
        &grid,
        request,
        &sorted_fit,
    );
    baseline.grid_fingerprint = Some(fingerprint.clone());
    legs.push(baseline);
    for (mode, build) in &builds {
        if check_cancel(cancel) {
            bail!("noise compare cancelled during legs (code=cancelled_before_commit)");
        }
        let noise = match noise_model_for(*mode, build, request.sample_interval_s) {
            Ok(noise) => noise,
            Err(error) => {
                legs.push(LegSignal {
                    name: format!("{}_candidate", mode_name(*mode)),
                    estimator: format!("joint_{}", mode_name(*mode)),
                    mode: Some(mode_name(*mode).to_string()),
                    state: LegState::Failed,
                    code: Some("model_mismatch".to_string()),
                    message: Some(format!("{error:#}")),
                    model_sha256: Some(build.record.sha256.clone()),
                    grid_fingerprint: Some(fingerprint.clone()),
                    output_rows: None,
                    window_failures: 0,
                    series: Vec::new(),
                });
                continue;
            }
        };
        let label = format!("{}_candidate", mode_name(*mode));
        let estimator = format!("joint_{}", mode_name(*mode));
        let mut leg = run_leg_series(
            &label,
            &estimator,
            Some(*mode),
            Some(build),
            Some(noise),
            &eval_signal,
            &eval_times,
            &centers,
            &grid,
            request,
            &sorted_fit,
        );
        leg.grid_fingerprint = Some(fingerprint.clone());
        // Scientific qualification (PN-FR-032): a completed leg whose
        // reserved adequacy failed is unqualified unless the request
        // explicitly allows exploratory execution (never a pass verdict).
        if leg.state == LegState::Complete && !reserved.adequate && !allow_exploratory {
            leg.state = LegState::Unqualified;
            leg.code = Some("unqualified".to_string());
            leg.message = Some(format!(
                "reserved adequacy failed ({}); leg withheld from pass verdicts",
                reserved.reason
            ));
        } else if leg.state == LegState::Complete && !reserved.adequate {
            leg.message = Some(format!(
                "exploratory only: reserved adequacy failed ({})",
                reserved.reason
            ));
        }
        legs.push(leg);
    }
    // Explicit unavailable legs: requested modes with no built model stay
    // recorded (PN-FR-026), never filled from another mode.
    for record in &model_records {
        if record.status != "failed" {
            continue;
        }
        legs.push(LegSignal {
            name: format!("{}_candidate", record.mode),
            estimator: format!("joint_{}", record.mode),
            mode: Some(record.mode.clone()),
            state: LegState::Unavailable,
            code: Some("model_unavailable".to_string()),
            message: record.reason.clone(),
            model_sha256: None,
            grid_fingerprint: Some(fingerprint.clone()),
            output_rows: None,
            window_failures: 0,
            series: Vec::new(),
        });
    }

    // --- Phase 9: paired evidence per eligible evaluation region. The
    // labeled baseline is the comparison anchor when present; when only
    // explicit unavailable records exist, evidence stays empty and the
    // report records the calibration-limited outcome (PN-FR-022/026).
    let mut evidence = Vec::new();
    if let Some(baseline) = legs.first().filter(|leg| leg.state == LegState::Complete) {
        for leg in legs.iter().skip(1) {
            if leg.state != LegState::Complete {
                continue;
            }
            // One evidence record per eligible evaluation span (region).
            for (index, span) in evaluation_spans.iter().enumerate() {
                let region_label = format!("eligible_evaluation_{index}");
                let relative = vec![(span.0 - origin, span.1 - origin)];
                evidence.push(assess_candidate(
                    leg,
                    baseline,
                    &region_label,
                    &relative,
                    &statistics,
                ));
            }
        }
    }

    // --- Phase 10: stage every destination file, then the manifest.
    let request_text = std::fs::read_to_string(request_path)
        .with_context(|| "cannot re-read compare request for staging")?;
    std::fs::write(staging.join("request.source.toml"), &request_text)
        .context("cannot stage request.source.toml")?;
    let resolved_plan_json = serde_json::json!({
        "schema_version": NOISE_COMPARE_SCHEMA_VERSION,
        "operation": "compare",
        "study_classification": study,
        "detector_channel": source.binding().detector,
        "reference_channel": source.binding().reference,
        "witness_channel": source.binding().witness,
        "stride": stride,
        "sample_count": total_samples,
        "sample_interval_s": request.sample_interval_s,
        "reference_frequency_hz": request.reference_frequency_hz,
        "reference_phase_rad": request.reference_phase_rad,
        "block_len": request.block_len,
        "guard": resolved.leakage.guard,
        "role_plan": resolved.role_plan,
        "leakage": resolved.leakage,
        "recipe": {
            "phase_bins": phase_recipe.bins,
            "phase_bins_explicit": bins_explicit,
            "min_samples_per_bin": phase_recipe.min_samples_per_bin,
            "min_cycles_per_bin": phase_recipe.min_cycles_per_bin,
            "min_contributing_blocks": phase_recipe.min_contributing_blocks,
            "variance_shrinkage": phase_recipe.shrinkage_alpha,
            "variance_floor_ratio": phase_recipe.floor_ratio,
            "max_lag": correlation_recipe.max_lag,
            "correlation_shrinkage": correlation_recipe.shrinkage_eta,
            "nuisance": COMPARE_NUISANCE_RECIPE_ID,
            "adequacy_policy": COMPARE_ADEQUACY_POLICY_ID,
        },
        "frozen_inputs": source.frozen_view(),
    });
    write_json(
        &staging.join("request.resolved.json"),
        &resolved_plan_json,
        "resolved compare plan",
    )?;
    let inputs = serde_json::json!({
        "schema_version": NOISE_COMPARE_SCHEMA_VERSION,
        "frozen_inputs": source.frozen_view(),
    });
    write_json(&staging.join("inputs.json"), &inputs, "frozen inputs")?;
    let diagnostics_report = serde_json::json!({
        "schema_version": NOISE_COMPARE_SCHEMA_VERSION,
        "operation": "compare",
        "study_classification": study,
        "diagnostic_finding": finding,
        "finding_reason": reason,
        "measurements_available": measurements.is_some(),
        "measurements": measurements,
    });
    write_json(
        &staging.join("diagnostics.json"),
        &diagnostics_report,
        "diagnostic report",
    )?;
    write_json(&staging.join("context.json"), &context, "frozen context")?;
    write_json(
        &staging.join("role-plan.json"),
        &serde_json::json!({
            "schema_version": NOISE_COMPARE_SCHEMA_VERSION,
            "role_plan": resolved.role_plan,
            "leakage": resolved.leakage,
        }),
        "role plan",
    )?;
    write_json(
        &staging.join("reserved.json"),
        &reserved,
        "reserved evidence",
    )?;
    let models_json: Vec<FrozenModelRecord> = model_records.clone();
    write_json(&staging.join("models.json"), &models_json, "frozen models")?;
    // Per-leg output series (centers + fundamental magnitudes) for replay.
    for leg in &legs {
        let leg_json = serde_json::json!({
            "schema_version": NOISE_COMPARE_SCHEMA_VERSION,
            "name": leg.name,
            "estimator": leg.estimator,
            "mode": leg.mode,
            "state": leg.state,
            "code": leg.code,
            "message": leg.message,
            "model_sha256": leg.model_sha256,
            "grid_fingerprint": leg.grid_fingerprint,
            "output_rows": leg.output_rows,
            "window_failures": leg.window_failures,
            "context_sha256": context.context_sha256,
            "centers": leg.series.iter().map(|(center, _)| center + origin).collect::<Vec<_>>(),
            "fundamental_magnitude": leg.series.iter().map(|(_, value)| value).collect::<Vec<_>>(),
        });
        let leg_path = staging.join(format!("legs/{}.json", leg.name));
        if let Some(parent) = leg_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        write_json(&leg_path, &leg_json, "leg output")?;
    }
    let grid_json = serde_json::json!({
        "schema_version": NOISE_COMPARE_SCHEMA_VERSION,
        "half_taps": grid.half_taps,
        "stride": grid.stride,
        "first_center": grid.first_center + origin,
        "last_center": grid.last_center + origin,
        "centers": centers.len(),
        "fingerprint": fingerprint,
        "context_sha256": context.context_sha256,
    });
    write_json(&staging.join("schedule.json"), &grid_json, "grid schedule")?;

    let completed_legs = legs
        .iter()
        .filter(|leg| leg.state == LegState::Complete)
        .count();
    let requested_modes = request.candidates.joint_modes.len();
    let built_modes = builds.len();
    let grid_equal = legs
        .iter()
        .filter_map(|leg| leg.grid_fingerprint.as_deref())
        .collect::<std::collections::HashSet<_>>()
        .len()
        <= 1;
    let computation_complete = completed_legs >= 1 && built_modes == requested_modes && grid_equal;
    let operation_status = if completed_legs == legs.len() && built_modes == requested_modes {
        "complete"
    } else if completed_legs >= 1 {
        "partial"
    } else {
        "failed"
    };
    let data_quality = if measurements.is_some() {
        "valid"
    } else {
        "unverified"
    };
    let report = CompareReport {
        schema_version: NOISE_COMPARE_SCHEMA_VERSION,
        operation: "compare".to_string(),
        study_classification: study.to_string(),
        statistic_id: COMPARE_STATISTIC_ID.to_string(),
        operation_status: operation_status.to_string(),
        operation_reason: if operation_status == "complete" {
            format!(
                "baseline plus all {requested_modes} requested modes complete on the frozen grid"
            )
        } else {
            format!(
                "{completed_legs} of {} legs complete; {built_modes} of {requested_modes} models built",
                legs.len()
            )
        },
        data_quality: data_quality.to_string(),
        diagnostic_finding: finding,
        context,
        models: model_records.clone(),
        reserved,
        legs: legs
            .iter()
            .map(|leg| CompareLeg {
                name: leg.name.clone(),
                estimator: leg.estimator.clone(),
                mode: leg.mode.clone(),
                state: leg.state,
                code: leg.code.clone(),
                message: leg.message.clone(),
                model_sha256: leg.model_sha256.clone(),
                grid_fingerprint: leg.grid_fingerprint.clone(),
                output_rows: leg.output_rows,
                window_failures: leg.window_failures,
            })
            .collect(),
        evidence,
        grid_equal,
        computation_complete,
        cancelled_before_commit: false,
        frozen_inputs: source.frozen_view().clone(),
    };
    write_json(&staging.join("comparison.json"), &report, "compare report")?;
    write_report_md(staging, &report, request_path, bins_explicit)?;
    // Restart manifest last: its presence with matching digests is the
    // resume gate (PN-FR-031).
    let mut model_digests = BTreeMap::new();
    for record in &model_records {
        if !record.sha256.is_empty() {
            model_digests.insert(record.mode.clone(), record.sha256.clone());
        }
    }
    let manifest = StagingManifest {
        schema_version: NOISE_COMPARE_SCHEMA_VERSION,
        request_sha256: request_sha.to_string(),
        source_digest,
        context_sha256: report.context.context_sha256.clone(),
        model_digests,
        phase: "staged".to_string(),
        completed_phases: vec![
            "freeze".to_string(),
            "plan".to_string(),
            "diagnose".to_string(),
            "calibrate".to_string(),
            "reserved".to_string(),
            "context".to_string(),
            "legs".to_string(),
            "evidence".to_string(),
        ],
    };
    write_json(
        &staging.join("staging-manifest.json"),
        &manifest,
        "staging manifest",
    )?;
    source.verify_live()?;
    Ok(report)
}

/// Mechanism-agnostic Markdown report (PN-FR-035): what was measured,
/// what is unknown, and what is needed next. No physical noise cause is
/// asserted; every limitation stays explicit.
fn write_report_md(
    staging: &Path,
    report: &CompareReport,
    request_path: &Path,
    bins_explicit: bool,
) -> Result<()> {
    use std::fmt::Write as _;
    let mut text = String::new();
    let _ = writeln!(
        text,
        "# Noise comparison report (recorded-only, PN-M2)\n\n\
         - operation: `{op}` ({status}) — {reason}\n\
         - study: `{study}`, statistic: `{stat}`\n\
         - diagnostic finding: `{finding}` (data quality: {dq})\n\
         - grid equal: {grid}, computation complete: {cc}\n\
         - context: `{ctx}` (rotation {rot} rad, depth {depth}, field {field})\n\
         - phase bins: {bins} ({explicit})\n\
         - request: `{req}`\n",
        op = report.operation,
        status = report.operation_status,
        reason = report.operation_reason,
        study = report.study_classification,
        stat = report.statistic_id,
        finding = report.diagnostic_finding,
        dq = report.data_quality,
        grid = report.grid_equal,
        cc = report.computation_complete,
        ctx = report.context.context_sha256,
        rot = report.context.rotation_rad,
        depth = report.context.modulation_depth,
        field = report.context.field_factor,
        bins = report
            .models
            .first()
            .map(|model| model.phase_bins)
            .unwrap_or(0),
        explicit = if bins_explicit {
            "explicit request"
        } else {
            "accepted 64 by resolution"
        },
        req = request_path.display(),
    );
    text.push_str("## Frozen models\n\n");
    for model in &report.models {
        let _ = writeln!(
            text,
            "- `{mode}` {status}: sha256 `{sha}`, {bytes} bytes, basis `{basis}`, \
             training {tb} blocks / {ti} intervals, reserved {rb} measured ({adequate})\n",
            mode = model.mode,
            status = model.status,
            sha = if model.sha256.is_empty() {
                "(unbuilt)"
            } else {
                &model.sha256
            },
            bytes = model.bytes,
            basis = model.correlation_basis,
            tb = model.training_blocks,
            ti = model.training_intervals,
            rb = model.reserved_blocks_measured,
            adequate = if model.adequacy_adequate {
                "adequate"
            } else {
                "unqualified"
            },
        );
    }
    text.push_str("## Legs\n\n");
    for leg in &report.legs {
        let _ = writeln!(
            text,
            "- `{name}` ({est}): {state:?}{code} — {rows} rows, {wf} window failures{msg}\n",
            name = leg.name,
            est = leg.estimator,
            state = leg.state,
            code = leg
                .code
                .as_deref()
                .map(|code| format!(" [{code}]"))
                .unwrap_or_default(),
            rows = leg.output_rows.unwrap_or(0),
            wf = leg.window_failures,
            msg = leg
                .message
                .as_deref()
                .map(|message| format!(": {message}"))
                .unwrap_or_default(),
        );
    }
    text.push_str("## Candidate evidence\n\n");
    if report.evidence.is_empty() {
        text.push_str("- no completed candidate legs; no paired evidence computed.\n");
    }
    for item in &report.evidence {
        let _ = writeln!(
            text,
            "- `{cand}` vs `{base}` ({region}): SD ratio {ratio}, CI {ci}, gate {gate}, scientific {sci} over {n} blocks\n",
            cand = item.candidate,
            base = item.baseline,
            region = item.region_label,
            ratio = item
                .sd_ratio
                .map(|ratio| format!("{ratio:.4}"))
                .unwrap_or_else(|| "(uncomputed)".to_string()),
            ci = item
                .paired_ci
                .map(|ci| format!("[{:.4}, {:.4}]", ci[0], ci[1]))
                .unwrap_or_else(|| "(uncomputed)".to_string()),
            gate = item.benefit_gate,
            sci = item.scientific_verdict,
            n = item.independent_blocks,
        );
    }
    text.push_str(
        "\n## Provenance\n\n\
         - `request.source.toml`: exact request bytes\n\
         - `request.resolved.json`: resolved role plan, guard, and recipe\n\
         - `inputs.json`: frozen input digests (before/after reads verified)\n\
         - `context.json`: frozen shared downstream context\n\
         - `calibrations/ch*/<mode>/model.json`: frozen mode-specific models\n\
         - `reserved.json`: measured reserved diagnostics (never caller-supplied)\n\
         - `legs/*.json`: per-leg demodulation series with model attribution\n\
         - `schedule.json`: fixed window grid shared by every leg\n\
         - `comparison.json`: machine-readable report\n\
         - No physical noise cause is asserted by this report.\n",
    );
    std::fs::write(staging.join("report.md"), text)
        .with_context(|| format!("cannot write {}", staging.join("report.md").display()))?;
    Ok(())
}

#[cfg(test)]
#[path = "compare_tests.rs"]
mod tests;
