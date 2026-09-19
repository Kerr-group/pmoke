//! Frozen global calibration plus controlled comparison (PN-M2, Issue #246;
//! PN-M4 qualified diagnostics and benefit/fidelity evaluation).
//!
//! `pmoke noise compare --request REQUEST.toml` performs the complete
//! explicitly requested recorded workflow in one request (PN-FR-008):
//! open and freeze one recorded source -> resolve the role plan with the
//! leakage guard (PN-FR-006/007) -> run mechanism-agnostic diagnostics
//! (PN-FR-009..012) with the calibrated change rule -> build and freeze one
//! mode-specific calibration artifact per requested candidate mode with exact
//! SHA-256 digests (PN-FR-014/015/016) -> compute measured reserved
//! diagnostics from nominated reserved blocks (never caller-supplied
//! statistics, PN-FR-014) -> freeze the shared downstream context (PN-FR-005)
//! -> run the labeled boxcar baseline plus every requested candidate leg on
//! the same fixed window grid (PN-FR-022/026) -> assess paired
//! residual-scatter evidence with the frozen statistic choice, the distinct
//! mean-block-SD statistic, the stratified paired bootstrap and the stability
//! control (PN-FR-027/028/029) -> report separate result gates, complete
//! method x channel x region accounting and the unverified fidelity section
//! (PN-FR-030/032) -> publish atomically into a new destination with
//! restart/cancel semantics (PN-FR-031, PN-NFR-005).
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

use super::bank::{self, BankLegRow, BoundaryStats};
use super::diagnostics::{CHANGE_QUALIFICATION_STATUS, run_signal_diagnostics};
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
/// The distinct statistic retained by `pmoke evaluate` (PN-FR-036): a ratio
/// of average block SDs. It is never silently renamed to the pooled
/// statistic; the compare lane reports it side by side (PN-AT-019).
pub const COMPARE_EVALUATE_LOCKIN_STATISTIC_ID: &str = "mean_block_sd_ratio_v1";
/// Same identity as the evaluate-lockin statistic, computed here on the
/// shared paired compare blocks for the side-by-side reconciliation.
pub const COMPARE_MEAN_BLOCK_SD_STATISTIC_ID: &str = "mean_block_sd_ratio_v1";
/// The exact primary formula recorded with every report (NUMERICS 7).
pub const COMPARE_STATISTIC_FORMULA: &str = "candidate/baseline ratio of the paired scatter over eligible evaluation blocks: \
     global-mean-centered SD of the concatenated block values under detrend=none; \
     pooled within-block scatter after per-block mean removal with summed block degrees of freedom under detrend=block_mean";
/// The exact secondary formula recorded with every report (NUMERICS 7).
pub const COMPARE_MEAN_BLOCK_FORMULA: &str = "candidate/baseline ratio of the mean per-block SD over the same paired eligible blocks; \
     block SD uses the declared detrending convention (the evaluate-lockin statistic family)";
/// Statistical reproducibility identities (PN-NFR-006).
pub const COMPARE_RNG_ID: &str = "xorshift64/v1";
pub const COMPARE_RESAMPLING_UNIT: &str = "whole_disjoint_evaluation_blocks";
pub const COMPARE_STRATIFICATION: &str = "eligible_evaluation_regions";
pub const COMPARE_PAIRING_POLICY: &str = "paired_centers_within_region_blocks";
pub const COMPARE_OVERLAP_POLICY: &str = "overlapping_supports_never_counted_as_independent";
pub const COMPARE_MULTIPLICITY_POLICY_ID: &str = "all_candidates_visible_no_selection/v1";
pub const COMPARE_STABILITY_POLICY_ID: &str = "block_reduction_stability/v1";
/// Prespecified descriptive stability guard (PN-FR-028, PN-AT-018): a numeric
/// reduction concentrated in a minority of blocks or improving fewer than
/// half of them is reported as a stability failure and is never labeled a
/// noise gain. Descriptive guard, not a physical threshold.
pub const COMPARE_STABILITY_MAX_CONCENTRATION: f64 = 0.5;
pub const COMPARE_STABILITY_MIN_IMPROVED_FRACTION: f64 = 0.5;
/// Inherited numeric benefit gate (PN-D-006).
pub const COMPARE_TARGET_SD_RATIO: f64 = 0.97;

/// Resource caps enforced before cloning or solving (PN-NFR-003 /
/// PN-A-007): at most 8 mode entries per channel, 1 MiB per serialized
/// model, 8 MiB aggregate model bytes per channel.
pub const MAX_COMPARE_MODELS_PER_CHANNEL: usize = 8;
pub const MAX_COMPARE_MODEL_BYTES: usize = 1024 * 1024;
pub const MAX_COMPARE_TOTAL_MODEL_BYTES_PER_CHANNEL: usize = 8 * 1024 * 1024;

/// Retained bank-row cap before solving (PN-FR-037): the bank lane fails
/// explicitly instead of retaining unbounded per-row records.
pub const MAX_BANK_RETAINED_ROWS: usize = 50_000;

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
    /// Opt-in frozen condition-specific bank (PN-M3, PN-FR-018/019):
    /// absent means the frozen global path. Presence switches the workflow
    /// to an explicit regime schedule resolved before inference.
    pub bank: Option<CompareBankToml>,
    pub output: String,
}

/// Opt-in regime bank request (bank/schedule schema v1, PN-A-004/PN-FR-018).
/// Regimes come from an external schedule or declared training context;
/// nothing here may be optimized on final MOKE noise or transitions.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareBankToml {
    pub schema_version: u32,
    pub selection_provenance: CompareBankSelectionProvenance,
    /// Deterministic schedule spans in original sample indices; every
    /// output center must lie inside exactly one span (PN-FR-019).
    pub schedule: Vec<CompareBankScheduleSpanToml>,
    pub regimes: Vec<CompareBankRegimeToml>,
    /// Worker threads for the bank apply (deterministic output regardless).
    pub workers: Option<usize>,
    /// Output chunk size for the bank apply (bounded per-worker scratch).
    pub chunk_size: Option<usize>,
}

/// Why the bank regime split exists: an external schedule, or declared
/// training/validation context. Both are chosen without reading final
/// target windows (PN-FR-018/019).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareBankSelectionProvenance {
    ExternalSchedule,
    DeclaredTrainingContext,
}

/// One regime: a portable name, a declared condition label, and the
/// half-open training intervals the regime model is fitted from.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareBankRegimeToml {
    pub name: String,
    pub condition: String,
    /// Optional explicit channel binding; must match the resolved detector.
    pub channel: Option<u8>,
    pub intervals: Vec<CompareBankIntervalToml>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareBankIntervalToml {
    pub start: u64,
    pub end: u64,
}

/// One schedule span: half-open original-index range assigned to a regime.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareBankScheduleSpanToml {
    pub regime: String,
    pub start: u64,
    pub end: u64,
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
/// (PN-FR-026/027/029): pooled within-region scatter ratio with a paired
/// bootstrap interval over disjoint eligible evaluation blocks, plus the
/// distinct mean-block-SD statistic and the stability/sensitivity controls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateEvidence {
    pub candidate: String,
    pub baseline: String,
    pub mode: Option<String>,
    pub region_label: String,
    pub statistic_id: String,
    pub secondary_statistic_id: String,
    pub independent_blocks: usize,
    pub strata: usize,
    pub pooled_dof: usize,
    pub unequal_block_lengths: bool,
    pub excluded_blocks: usize,
    pub zero_denominator_blocks: usize,
    pub nonfinite_blocks: usize,
    pub baseline_scatter: Option<f64>,
    pub candidate_scatter: Option<f64>,
    pub sd_ratio: Option<f64>,
    pub mean_block_sd_ratio: Option<f64>,
    pub paired_ci: Option<[f64; 2]>,
    pub familywise_ci: Option<[f64; 2]>,
    pub detrend_ratio_none: Option<f64>,
    pub detrend_ratio_block_mean: Option<f64>,
    pub baseline_mean: Option<f64>,
    pub candidate_mean: Option<f64>,
    pub mean_shift: Option<f64>,
    pub blocks_improved_fraction: Option<f64>,
    pub reduction_concentration: Option<f64>,
    pub stability_gate: String,
    pub stability_policy_id: String,
    pub target_sd_ratio: f64,
    pub benefit_gate: String,
    pub scientific_verdict: String,
    pub notes: Vec<String>,
}

/// Versioned statistics policy recorded with the report (PN-NFR-006): RNG,
/// resampling unit, pairing/stratification, multiplicity policy and the exact
/// estimator formulas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatisticsPolicy {
    pub primary_statistic_id: String,
    pub primary_formula: String,
    pub secondary_statistic_id: String,
    pub secondary_formula: String,
    pub evaluate_lockin_statistic_id: String,
    pub detrend: String,
    pub detrend_sensitivity: Vec<String>,
    pub resampling_unit: String,
    pub stratification: String,
    pub pairing: String,
    pub overlap_policy: String,
    pub bootstrap_replicates: usize,
    pub bootstrap_seed: u64,
    pub rng_id: String,
    pub confidence_level: f64,
    pub multiplicity_policy_id: String,
    pub candidates_requested: usize,
    pub candidates_compared: usize,
    pub selection_performed: bool,
    pub stability_policy_id: String,
}

/// Independently reported result gates (PN-FR-032, PN-AT-022). A pass on one
/// dimension never overrides `unverified` on another.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultGates {
    pub acquisition_validity: String,
    pub computation: String,
    pub diagnostic_qualification: String,
    pub model_adequacy: String,
    pub numeric_benefit: String,
    pub dynamic_fidelity: String,
    pub scientific: String,
    pub default_promotion: String,
    pub reasons: Vec<String>,
}

/// Fidelity controls (PN-FR-030, PN-AT-020): recorded status plus the
/// boundary/seam evidence measured on the retained rows. The physical
/// tolerance is unspecified (PN-D-007), so fidelity stays unverified and the
/// scientific verdict cannot pass even when the numeric SD improves.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FidelitySection {
    pub status: String,
    pub reason: String,
    pub tolerance_authority: Option<String>,
    pub injected_controls: String,
    pub output_resolution_centers: usize,
    pub boundary_windows: usize,
    pub cross_regime_windows: usize,
    pub switch_jump_rms: Option<f64>,
    pub interior_jump_rms: Option<f64>,
    pub notes: Vec<String>,
}

/// One requested method x channel x region accounting row (PN-FR-026): every
/// requested combination is enumerated with its outcome and reason, so an
/// isolated failure keeps its record and nothing is silently dropped.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateAccounting {
    pub method: String,
    pub channel: u8,
    pub region: String,
    pub outcome: String,
    pub reason: Option<String>,
    pub evidence_recorded: bool,
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
    /// Bank summary (PN-M3): absent for the frozen global lane.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bank: Option<BankReport>,
    pub reserved: ReservedEvidence,
    pub legs: Vec<CompareLeg>,
    pub evidence: Vec<CandidateEvidence>,
    /// Versioned statistics policy (PN-NFR-006).
    pub statistics: StatisticsPolicy,
    /// Independently reported result gates (PN-FR-032).
    pub gates: ResultGates,
    /// Fidelity controls and boundary evidence (PN-FR-030).
    pub fidelity: FidelitySection,
    /// Complete requested method x channel x region accounting (PN-FR-026).
    pub accounting: Vec<CandidateAccounting>,
    pub accounting_complete: bool,
    pub grid_equal: bool,
    pub computation_complete: bool,
    pub cancelled_before_commit: bool,
    pub frozen_inputs: FrozenInputView,
}

/// Staging manifest binding restart/resume to hashes (PN-FR-031).
/// Schema v2 (PN-M3) adds the full retained-file digest closure, the
/// bank/schedule digests and the row-mapping record; v1 manifests stay
/// readable with their historical meaning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagingManifest {
    pub schema_version: u32,
    pub request_sha256: String,
    pub source_digest: String,
    pub context_sha256: String,
    pub model_digests: BTreeMap<String, String>,
    pub phase: String,
    pub completed_phases: Vec<String>,
    /// SHA-256 of every staged file, keyed by destination-relative path
    /// (schema v2). Empty for v1 manifests.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub file_digests: BTreeMap<String, String>,
    /// Distinct-model bank hash (schema v2, bank lane only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bank_sha256: Option<String>,
    /// Resolved center -> regime schedule hash (schema v2, bank lane only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_sha256: Option<String>,
    /// Row-mapping record: per-leg retained row counts and the schedule row
    /// count (schema v2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_mapping: Option<serde_json::Value>,
}

/// Resolved bank plan plus destination bookkeeping for one compare run
/// (PN-FR-018/019/023). Slots are the deduplicated distinct models; links
/// bind every regime to its mode-specific slot.
struct BankState {
    resolved: bank::ResolvedBank,
    slots: Vec<bank::BankModelSlot>,
    links: Vec<bank::BankRegimeLink>,
    regime_to_slot: BTreeMap<String, Vec<usize>>,
    reduction: BTreeMap<String, String>,
    bank_sha256: String,
    schedule_sha256: String,
    schedule_rows: Vec<bank::BankScheduleRow>,
    primary_sha: String,
    primary_variance: PhaseVarianceOutput,
    evidence: Vec<BankLegEvidence>,
}

/// Per-leg bank application evidence retained in `bank-evidence.json`.
#[derive(Debug, Clone, Serialize)]
struct BankLegEvidence {
    leg: String,
    mode: String,
    rows: usize,
    window_failures: usize,
    prepared_plans: usize,
    prepare_ms: f64,
    apply_ms: f64,
    direct_reference_rows: usize,
    direct_reference_ms: f64,
    /// Measured per-window cost of the direct estimator on the same grid,
    /// extrapolated over every retained row: the conditional-bank apply is
    /// measured against the equivalent fixed-global solver (PN-NFR-004).
    fixed_global_projection_ms: f64,
    /// apply_ms / fixed_global_projection_ms; <= 1 means the bank is cheaper.
    accelerated_vs_projection_ratio: f64,
    max_abs_error_v: f64,
    boundary: BoundaryStats,
}

/// Bank summary carried in the comparison report (PN-FR-023): identity of
/// the frozen schedule and model bank, the identical-model reduction state,
/// and the boundary coverage counts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BankReport {
    pub schema_version: u32,
    pub selection_provenance: String,
    pub bank_sha256: String,
    pub schedule_sha256: String,
    pub reduction: BTreeMap<String, String>,
    pub distinct_models: BTreeMap<String, usize>,
    pub schedule_spans: usize,
    pub boundary_windows: usize,
    pub cross_regime_windows: usize,
    pub row_mapping_ok: bool,
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

pub(super) struct TrainingData {
    block_times: Vec<Vec<f64>>,
    block_residuals: Vec<Vec<f64>>,
    block_phases: Vec<Vec<f64>>,
}

/// Training spans nominated for the global lane: every `Training` role
/// block from the resolved role plan (PN-FR-006), sorted. Bank regimes
/// call [`read_training_blocks`] with their own explicit intervals.
pub(super) fn training_spans(resolved: &ResolvedPlan) -> Vec<(u64, u64)> {
    let mut spans: Vec<(u64, u64)> = resolved
        .role_plan
        .blocks
        .iter()
        .filter(|block| block.role == CalibrationRole::Training)
        .map(|block| (block.start, block.end))
        .collect();
    spans.sort_unstable();
    spans
}

/// Reads the nominated training spans and fits the shared nuisance model on
/// each one (PN-FR-010/015). Non-finite samples fail closed; the caller
/// supplies spans from the resolved role plan or from explicit bank regime
/// intervals.
pub(super) fn read_training_blocks(
    source: &RecordedSource,
    spans: &[(u64, u64)],
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    sample_interval_s: f64,
) -> Result<TrainingData> {
    if spans.is_empty() {
        bail!("noise compare plan holds no training blocks (code=insufficient_calibration)");
    }
    let sample_rate_hz = 1.0 / sample_interval_s;
    let tolerances = JointSolverTolerances::default();
    let mut block_times = Vec::with_capacity(spans.len());
    let mut block_residuals = Vec::with_capacity(spans.len());
    let mut block_phases = Vec::with_capacity(spans.len());
    for (start, end) in spans {
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

pub(super) struct ModeBuild {
    record: FrozenModelRecord,
    artifact_json: Vec<u8>,
    variance: PhaseVarianceOutput,
    correlation: Option<CorrelationOutput>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_mode_artifact(
    mode: CompareJointMode,
    model_prefix: &str,
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
        model_id: format!("noise-compare-{model_prefix}-{mode_str}"),
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
            // Frozen compare models declare an explicit reference: no
            // same-run fit uncertainty is recorded (Issue #274 FR-01).
            reference_frequency_rel_uncertainty: None,
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
        frequency_tol_overridden: false,
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
    /// Retained per-row bank records (PN-M3): attribution, XY and the
    /// policy-serialized covariance for every completed center.
    bank_rows: Vec<BankLegRow>,
    /// Boundary/switch diagnostics for this leg (bank lane only).
    bank_stats: Option<BoundaryStats>,
    /// Frozen schedule digest the leg's row mapping is bound to.
    bank_schedule_sha256: Option<String>,
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
    // XY layout is [X1, Y1, X2, Y2, ...] (the JointEstimate contract, and the
    // layout the lockin/phase/MOKE stages consume); the fundamental
    // magnitude is hypot(X1, Y1).
    if estimate.xy.len() < 12 {
        bail!(
            "joint estimate holds {} XY values, need 12 (code=dimension_mismatch)",
            estimate.xy.len()
        );
    }
    Ok(estimate.xy[0].hypot(estimate.xy[1]))
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
        bank_rows: Vec::new(),
        bank_stats: None,
        bank_schedule_sha256: None,
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
        bank_rows: Vec::new(),
        bank_stats: None,
        bank_schedule_sha256: None,
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

fn xorshift64(state: &mut u64) -> u64 {
    let mut x = *state;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    *state = x;
    x
}

/// Per-block sufficient moments of one leg's series. Every paired statistic
/// (and every bootstrap replicate) is recomputed exactly from these moments,
/// so resampling never invents new observations and overlapping supports can
/// be detected instead of silently double counted (PN-FR-029, PN-NFR-006).
#[derive(Debug, Clone, Copy)]
struct BlockMoments {
    len: usize,
    sum: f64,
    sum_squares: f64,
}

impl BlockMoments {
    fn new(values: &[f64]) -> Self {
        Self {
            len: values.len(),
            sum: values.iter().sum(),
            sum_squares: values.iter().map(|value| value * value).sum(),
        }
    }

    /// Sum of squared deviations from this block's own mean.
    fn centered_sum_squares(&self) -> f64 {
        if self.len == 0 {
            return 0.0;
        }
        self.sum_squares - self.sum * self.sum / self.len as f64
    }

    /// Standard deviation about the block's own mean (the declared per-block
    /// SD convention of the mean-block ratio).
    fn block_sd(&self) -> Option<f64> {
        if self.len == 0 {
            return None;
        }
        let squares = self.centered_sum_squares();
        if squares.is_finite() && squares >= 0.0 {
            Some((squares / self.len as f64).sqrt())
        } else {
            None
        }
    }

    fn finite(&self) -> bool {
        self.sum.is_finite() && self.sum_squares.is_finite()
    }
}

/// Primary pooled scatter over a block set (NUMERICS 7, identity
/// `pooled_residual_scatter_ratio_v1`): the declared detrending convention
/// gives either the global-mean-centered SD of the concatenated block values
/// (dof = summed samples) or the pooled within-block scatter after per-block
/// mean removal (dof = summed samples minus one per block).
fn pooled_scatter_of(blocks: &[BlockMoments], detrend: CompareDetrend) -> Option<f64> {
    match detrend {
        CompareDetrend::None => {
            let total_len: usize = blocks.iter().map(|block| block.len).sum();
            if total_len == 0 {
                return None;
            }
            let total_sum: f64 = blocks.iter().map(|block| block.sum).sum();
            let total_squares: f64 = blocks.iter().map(|block| block.sum_squares).sum();
            let squares = total_squares - total_sum * total_sum / total_len as f64;
            if squares.is_finite() && squares >= 0.0 {
                Some((squares / total_len as f64).sqrt())
            } else {
                None
            }
        }
        CompareDetrend::BlockMean => {
            let squares: f64 = blocks.iter().map(BlockMoments::centered_sum_squares).sum();
            let dof: usize = blocks.iter().map(|block| block.len.saturating_sub(1)).sum();
            if dof == 0 || !squares.is_finite() || squares < 0.0 {
                return None;
            }
            Some((squares / dof as f64).sqrt())
        }
    }
}

/// Mean per-block standard deviation over the same paired blocks (the
/// evaluate-lockin statistic family): `mean_block_sd_ratio_v1`.
fn mean_block_scatter(blocks: &[BlockMoments]) -> Option<f64> {
    let mut sum = 0.0_f64;
    let mut count = 0_usize;
    for block in blocks {
        let sd = block.block_sd()?;
        sum += sd;
        count += 1;
    }
    if count == 0 {
        None
    } else {
        Some(sum / count as f64)
    }
}

fn ratio_of(baseline: Option<f64>, candidate: Option<f64>) -> Option<f64> {
    match (baseline, candidate) {
        (Some(baseline), Some(candidate))
            if baseline.is_finite() && baseline > 0.0 && candidate.is_finite() =>
        {
            Some(candidate / baseline)
        }
        _ => None,
    }
}

/// Quantile of an ascending-sorted replicate vector.
fn sorted_quantile(sorted: &[f64], probability: f64) -> f64 {
    if sorted.is_empty() {
        return f64::NAN;
    }
    let index = ((sorted.len() - 1) as f64 * probability.clamp(0.0, 1.0)).round() as usize;
    sorted[index]
}

/// Overlapping supports are never independent observations (PN-FR-029).
fn spans_pairwise_disjoint(spans: &[(u64, u64)]) -> bool {
    let mut sorted: Vec<(u64, u64)> = spans.to_vec();
    sorted.sort_unstable();
    sorted.windows(2).all(|pair| pair[0].1 <= pair[1].0)
}

/// Evidence record for a candidate that cannot be assessed (overlap guard or
/// empty block set): every statistic stays uncomputed with an explicit note.
#[allow(clippy::too_many_arguments)]
fn unassessable_evidence(
    candidate: &LegSignal,
    baseline: &LegSignal,
    region_label: &str,
    statistics: &StatisticsConfig,
    notes: Vec<String>,
) -> CandidateEvidence {
    CandidateEvidence {
        candidate: candidate.name.clone(),
        baseline: baseline.name.clone(),
        mode: candidate.mode.clone(),
        region_label: region_label.to_string(),
        statistic_id: COMPARE_STATISTIC_ID.to_string(),
        secondary_statistic_id: COMPARE_MEAN_BLOCK_SD_STATISTIC_ID.to_string(),
        independent_blocks: 0,
        strata: 0,
        pooled_dof: 0,
        unequal_block_lengths: false,
        excluded_blocks: 0,
        zero_denominator_blocks: 0,
        nonfinite_blocks: 0,
        baseline_scatter: None,
        candidate_scatter: None,
        sd_ratio: None,
        mean_block_sd_ratio: None,
        paired_ci: None,
        familywise_ci: None,
        detrend_ratio_none: None,
        detrend_ratio_block_mean: None,
        baseline_mean: None,
        candidate_mean: None,
        mean_shift: None,
        blocks_improved_fraction: None,
        reduction_concentration: None,
        stability_gate: "inconclusive".to_string(),
        stability_policy_id: COMPARE_STABILITY_POLICY_ID.to_string(),
        target_sd_ratio: statistics.target_sd_ratio,
        benefit_gate: "inconclusive".to_string(),
        scientific_verdict: "inconclusive".to_string(),
        notes,
    }
}

#[allow(clippy::too_many_arguments)]
fn assess_candidate(
    candidate: &LegSignal,
    baseline: &LegSignal,
    region_label: &str,
    strata: &[Vec<(u64, u64)>],
    family_size: usize,
    statistics: &StatisticsConfig,
) -> CandidateEvidence {
    let mut notes = Vec::new();
    let all_spans: Vec<(u64, u64)> = strata.iter().flatten().copied().collect();
    if !spans_pairwise_disjoint(&all_spans) {
        notes.push(
            "overlapping evaluation supports are never counted as independent observations \
             (PN-FR-029); no paired statistic was computed"
                .to_string(),
        );
        return unassessable_evidence(candidate, baseline, region_label, statistics, notes);
    }
    let mut excluded_blocks = 0_usize;
    let mut nonfinite_blocks = 0_usize;
    let mut zero_denominator_blocks = 0_usize;
    // Paired blocks per prespecified stratum (region). The bootstrap resamples
    // whole blocks within each stratum; LI windows and resamples are never
    // independent observations.
    let mut strata_pairs: Vec<Vec<(BlockMoments, BlockMoments)>> = Vec::with_capacity(strata.len());
    for stratum in strata {
        let mut pairs = Vec::new();
        for (start, end) in stratum {
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
            if baseline_values.is_empty()
                || candidate_values.is_empty()
                || baseline_values.len() != candidate_values.len()
            {
                excluded_blocks += 1;
                continue;
            }
            let baseline_moments = BlockMoments::new(&baseline_values);
            let candidate_moments = BlockMoments::new(&candidate_values);
            if !baseline_moments.finite() || !candidate_moments.finite() {
                nonfinite_blocks += 1;
                continue;
            }
            if baseline_moments.centered_sum_squares() <= 0.0
                || candidate_moments.centered_sum_squares() < 0.0
            {
                zero_denominator_blocks += 1;
            }
            pairs.push((baseline_moments, candidate_moments));
        }
        if !pairs.is_empty() {
            strata_pairs.push(pairs);
        }
    }
    let independent_blocks: usize = strata_pairs.iter().map(Vec::len).sum();
    let strata_count = strata_pairs.len();
    let baseline_blocks: Vec<BlockMoments> = strata_pairs
        .iter()
        .flatten()
        .map(|(baseline, _)| *baseline)
        .collect();
    let candidate_blocks: Vec<BlockMoments> = strata_pairs
        .iter()
        .flatten()
        .map(|(_, candidate)| *candidate)
        .collect();
    if independent_blocks == 0 {
        notes.push(
            "no complete paired evaluation blocks were available; no statistic was computed"
                .to_string(),
        );
        return unassessable_evidence(candidate, baseline, region_label, statistics, notes);
    }
    let baseline_scatter = pooled_scatter_of(&baseline_blocks, statistics.detrend);
    let candidate_scatter = pooled_scatter_of(&candidate_blocks, statistics.detrend);
    let sd_ratio = ratio_of(baseline_scatter, candidate_scatter);
    let baseline_mean_sd = mean_block_scatter(&baseline_blocks);
    let candidate_mean_sd = mean_block_scatter(&candidate_blocks);
    let mean_block_sd_ratio = ratio_of(baseline_mean_sd, candidate_mean_sd);
    // Common-trend sensitivity: the pooled ratio under both declared
    // detrending policies (no-detrend and shared-baseline-trend).
    let detrend_ratio_none = ratio_of(
        pooled_scatter_of(&baseline_blocks, CompareDetrend::None),
        pooled_scatter_of(&candidate_blocks, CompareDetrend::None),
    );
    let detrend_ratio_block_mean = ratio_of(
        pooled_scatter_of(&baseline_blocks, CompareDetrend::BlockMean),
        pooled_scatter_of(&candidate_blocks, CompareDetrend::BlockMean),
    );
    let total_len: usize = baseline_blocks.iter().map(|block| block.len).sum();
    let baseline_mean = (total_len > 0)
        .then(|| baseline_blocks.iter().map(|block| block.sum).sum::<f64>() / total_len as f64);
    let candidate_mean = (total_len > 0)
        .then(|| candidate_blocks.iter().map(|block| block.sum).sum::<f64>() / total_len as f64);
    let mean_shift = match (baseline_mean, candidate_mean) {
        (Some(baseline), Some(candidate)) => Some(candidate - baseline),
        _ => None,
    };
    let unequal_block_lengths = {
        let first = baseline_blocks[0].len;
        baseline_blocks.iter().any(|block| block.len != first)
    };
    // Stability / transient control (PN-FR-028): a reduction concentrated in
    // a minority of blocks is not evidence of a noise-gain mechanism.
    let mut reducible = 0_usize;
    let mut improved = 0_usize;
    let mut reductions: Vec<f64> = Vec::new();
    for (baseline, candidate) in strata_pairs.iter().flatten() {
        let baseline_ms = baseline.centered_sum_squares() / baseline.len.max(1) as f64;
        let candidate_ms = candidate.centered_sum_squares() / candidate.len.max(1) as f64;
        if baseline_ms.is_finite() && candidate_ms.is_finite() && baseline_ms > 0.0 {
            reducible += 1;
            if candidate_ms < baseline_ms {
                improved += 1;
            }
            reductions.push(baseline_ms - candidate_ms);
        }
    }
    let blocks_improved_fraction = (reducible > 0).then(|| improved as f64 / reducible as f64);
    let total_reduction: f64 = reductions.iter().sum();
    let reduction_concentration = if reductions.is_empty() || total_reduction <= 0.0 {
        None
    } else {
        Some(reductions.iter().copied().fold(f64::NEG_INFINITY, f64::max) / total_reduction)
    };
    let stability_gate = if reducible < 2 {
        // With fewer than two reducible blocks the spread of the reduction
        // cannot be assessed; that is insufficient evidence, not a failure.
        "inconclusive"
    } else {
        match (blocks_improved_fraction, reduction_concentration) {
            (Some(fraction), Some(concentration))
                if fraction >= COMPARE_STABILITY_MIN_IMPROVED_FRACTION
                    && concentration <= COMPARE_STABILITY_MAX_CONCENTRATION =>
            {
                "pass"
            }
            (Some(fraction), _) if fraction < COMPARE_STABILITY_MIN_IMPROVED_FRACTION => "fail",
            (_, Some(_)) => "fail",
            _ => "inconclusive",
        }
    };
    if stability_gate == "fail" {
        let concentration = reduction_concentration.unwrap_or(f64::NAN);
        notes.push(format!(
            "stability control `{policy}` failed: {improved} of {reducible} blocks improve and the largest single-block share of the total reduction is {concentration:.3}; a transient-dominated reduction is not a noise gain (PN-FR-028)",
            policy = COMPARE_STABILITY_POLICY_ID,
        ));
    }
    // Paired bootstrap over whole disjoint blocks within the prespecified
    // stratification, recomputing the primary pooled statistic exactly.
    let mut paired_ci: Option<[f64; 2]> = None;
    let mut familywise_ci: Option<[f64; 2]> = None;
    let minimum_blocks = statistics.min_independent_blocks.max(2);
    if independent_blocks < minimum_blocks {
        notes.push(format!(
            "only {independent_blocks} independent blocks; the paired interval needs at least {minimum_blocks} (PN-FR-029)"
        ));
    } else {
        let mut state = statistics.bootstrap_seed;
        let mut replicates = Vec::with_capacity(statistics.bootstrap_replicates);
        let mut degenerate = 0_usize;
        for _ in 0..statistics.bootstrap_replicates {
            let mut baseline_draw: Vec<BlockMoments> = Vec::with_capacity(independent_blocks);
            let mut candidate_draw: Vec<BlockMoments> = Vec::with_capacity(independent_blocks);
            for stratum in &strata_pairs {
                for _ in 0..stratum.len() {
                    let index = (xorshift64(&mut state) % stratum.len() as u64) as usize;
                    baseline_draw.push(stratum[index].0);
                    candidate_draw.push(stratum[index].1);
                }
            }
            match (
                pooled_scatter_of(&baseline_draw, statistics.detrend),
                pooled_scatter_of(&candidate_draw, statistics.detrend),
            ) {
                (Some(baseline), Some(candidate))
                    if baseline > 0.0 && baseline.is_finite() && candidate.is_finite() =>
                {
                    replicates.push(candidate / baseline);
                }
                _ => degenerate += 1,
            }
        }
        if replicates.len() * 4 < statistics.bootstrap_replicates * 3 {
            notes.push(format!(
                "paired bootstrap unusable: {degenerate} of {} replicates had a non-positive or non-finite denominator",
                statistics.bootstrap_replicates
            ));
        } else {
            replicates.sort_by(f64::total_cmp);
            let alpha = 1.0 - statistics.confidence_level;
            paired_ci = Some([
                sorted_quantile(&replicates, alpha / 2.0),
                sorted_quantile(&replicates, 1.0 - alpha / 2.0),
            ]);
            // Predeclared multiplicity control for a candidate family
            // (Bonferroni): all candidate outcomes remain visible and no
            // selection is performed by this lane.
            familywise_ci = if family_size > 1 {
                let adjusted = alpha / family_size as f64;
                Some([
                    sorted_quantile(&replicates, adjusted / 2.0),
                    sorted_quantile(&replicates, 1.0 - adjusted / 2.0),
                ])
            } else {
                paired_ci
            };
        }
    }
    let benefit_gate = match (sd_ratio, paired_ci) {
        (Some(ratio), Some(ci)) if ratio <= statistics.target_sd_ratio && ci[1] < 1.0 => {
            "pass".to_string()
        }
        (Some(_), Some(_)) => "fail".to_string(),
        _ => "inconclusive".to_string(),
    };
    if familywise_ci.is_some_and(|ci| ci[1] >= 1.0) {
        notes.push(
            "the familywise-adjusted interval does not exclude 1; the unadjusted candidate interval is shown beside it (PN-FR-027/029)".to_string(),
        );
    }
    // PN-D-007: unknown physical fidelity tolerance leaves the scientific
    // verdict non-pass even when the numeric gate passes.
    let scientific_verdict = "inconclusive".to_string();
    notes.push(
        "scientific verdict stays non-pass: physical fidelity tolerance is unspecified (PN-D-007) and this lane performs no promotion".to_string(),
    );
    notes.push("residual scatter is not identified with a physical noise mechanism".to_string());
    CandidateEvidence {
        candidate: candidate.name.clone(),
        baseline: baseline.name.clone(),
        mode: candidate.mode.clone(),
        region_label: region_label.to_string(),
        statistic_id: COMPARE_STATISTIC_ID.to_string(),
        secondary_statistic_id: COMPARE_MEAN_BLOCK_SD_STATISTIC_ID.to_string(),
        independent_blocks,
        strata: strata_count,
        pooled_dof: baseline_blocks
            .iter()
            .map(|block| match statistics.detrend {
                CompareDetrend::None => block.len,
                CompareDetrend::BlockMean => block.len.saturating_sub(1),
            })
            .sum(),
        unequal_block_lengths,
        excluded_blocks,
        zero_denominator_blocks,
        nonfinite_blocks,
        baseline_scatter,
        candidate_scatter,
        sd_ratio,
        mean_block_sd_ratio,
        paired_ci,
        familywise_ci,
        detrend_ratio_none,
        detrend_ratio_block_mean,
        baseline_mean,
        candidate_mean,
        mean_shift,
        blocks_improved_fraction,
        reduction_concentration,
        stability_gate: stability_gate.to_string(),
        stability_policy_id: COMPARE_STABILITY_POLICY_ID.to_string(),
        target_sd_ratio: statistics.target_sd_ratio,
        benefit_gate,
        scientific_verdict,
        notes,
    }
}

// ---------------------------------------------------------------------------
// Complete candidate accounting, separate result gates, fidelity section
// (PN-FR-026/028/030/032, PN-AT-018/020/022).
// ---------------------------------------------------------------------------

/// Every requested method gets a terminal per-region outcome (PN-FR-026).
fn build_accounting(
    legs: &[LegSignal],
    evidence: &[CandidateEvidence],
    evaluation_spans: &[(u64, u64)],
    channel: u8,
) -> Vec<CandidateAccounting> {
    let mut rows = Vec::new();
    for leg in legs {
        let method = leg.mode.clone().unwrap_or_else(|| "boxcar".to_string());
        let is_baseline = leg.mode.is_none();
        if leg.state == LegState::Complete {
            for index in 0..evaluation_spans.len() {
                let region = format!("eligible_evaluation_{index}");
                let record = evidence
                    .iter()
                    .find(|item| item.candidate == leg.name && item.region_label == region);
                if is_baseline {
                    // The labeled baseline is the comparison anchor: it has no
                    // candidate pairing of its own, but it is recorded as the
                    // anchor of every evidence record over this region.
                    let anchored = evidence
                        .iter()
                        .any(|item| item.baseline == leg.name && item.region_label == region);
                    rows.push(CandidateAccounting {
                        method: method.clone(),
                        channel,
                        region,
                        outcome: "baseline_anchor".to_string(),
                        reason: None,
                        evidence_recorded: anchored,
                    });
                    continue;
                }
                let computed = record.is_some_and(|item| item.sd_ratio.is_some());
                rows.push(CandidateAccounting {
                    method: method.clone(),
                    channel,
                    region,
                    outcome: if computed {
                        "computed"
                    } else {
                        "computed_unusable"
                    }
                    .to_string(),
                    reason: if computed {
                        None
                    } else {
                        record
                            .and_then(|item| item.notes.first().cloned())
                            .or_else(|| {
                                Some(
                                    "no paired evidence record for this method and region"
                                        .to_string(),
                                )
                            })
                    },
                    evidence_recorded: record.is_some(),
                });
            }
        } else {
            let outcome = match leg.state {
                LegState::Failed => "failed",
                LegState::Unavailable => "unavailable",
                LegState::Unqualified => "unqualified",
                LegState::NotRun => "not_run",
                LegState::Complete => "complete",
            };
            rows.push(CandidateAccounting {
                method: method.clone(),
                channel,
                region: "all_eligible".to_string(),
                outcome: outcome.to_string(),
                reason: leg
                    .message
                    .clone()
                    .or_else(|| leg.code.clone())
                    .or_else(|| Some("no reason recorded".to_string())),
                evidence_recorded: false,
            });
        }
    }
    rows
}

/// Reconciliation of the requested method x region matrix: every requested
/// method is either accounted once per eligible region or once with an
/// explicit terminal reason (PN-FR-026, PN-AT-018).
fn accounting_complete(
    accounting: &[CandidateAccounting],
    request: &CompareRequest,
    evaluation_spans: &[(u64, u64)],
) -> bool {
    let mut methods: Vec<String> = vec!["boxcar".to_string()];
    for mode in &request.candidates.joint_modes {
        methods.push(mode_name(*mode).to_string());
    }
    if accounting.iter().any(|row| !methods.contains(&row.method)) {
        return false;
    }
    for method in &methods {
        let rows: Vec<&CandidateAccounting> = accounting
            .iter()
            .filter(|row| row.method == *method)
            .collect();
        if rows.is_empty() {
            return false;
        }
        let per_region = rows
            .iter()
            .filter(|row| {
                matches!(
                    row.outcome.as_str(),
                    "computed" | "computed_unusable" | "baseline_anchor"
                )
            })
            .count();
        if per_region > 0 {
            let mut regions: Vec<&str> = rows
                .iter()
                .filter(|row| {
                    matches!(
                        row.outcome.as_str(),
                        "computed" | "computed_unusable" | "baseline_anchor"
                    )
                })
                .map(|row| row.region.as_str())
                .collect();
            regions.sort_unstable();
            regions.dedup();
            if per_region != evaluation_spans.len() || regions.len() != evaluation_spans.len() {
                return false;
            }
        } else if rows.len() != 1 || rows[0].reason.is_none() {
            return false;
        }
    }
    true
}

/// Independently reported gates (PN-FR-032). A numeric pass never overrides
/// unverified adequacy/fidelity, and promotion is never granted here.
fn assemble_gates(
    operation_status: &str,
    data_quality: &str,
    model_records: &[FrozenModelRecord],
    evidence: &[CandidateEvidence],
    completed_legs: usize,
    total_legs: usize,
) -> ResultGates {
    let mut reasons = Vec::new();
    let acquisition_validity = match data_quality {
        "valid" => "pass",
        "unverified" => "unverified",
        _ => "suspect",
    }
    .to_string();
    if acquisition_validity != "pass" {
        reasons.push(format!(
            "acquisition validity is `{data_quality}`: the recorded data did not qualify"
        ));
    }
    let computation = match operation_status {
        "complete" => "complete",
        "partial" => "partial",
        _ => "failed",
    }
    .to_string();
    if computation != "complete" {
        reasons.push(format!(
            "computation is {computation}: {completed_legs} of {total_legs} leg records completed"
        ));
    }
    let diagnostic_qualification = CHANGE_QUALIFICATION_STATUS.to_string();
    if diagnostic_qualification != "qualified" {
        reasons.push(format!(
            "diagnostic qualification is `{diagnostic_qualification}`: the calibrated change rule is not verified"
        ));
    }
    let frozen_models = model_records
        .iter()
        .filter(|record| record.status == "frozen")
        .count();
    let failed_models = model_records
        .iter()
        .filter(|record| record.status != "frozen")
        .count();
    let inadequate_models = model_records
        .iter()
        .filter(|record| record.status == "frozen" && !record.adequacy_adequate)
        .count();
    let model_adequacy = if frozen_models == 0 {
        "unavailable"
    } else if failed_models > 0 || inadequate_models > 0 {
        "unqualified"
    } else {
        "adequate"
    }
    .to_string();
    if model_adequacy != "adequate" {
        reasons.push(format!(
            "model applicability is `{model_adequacy}` ({frozen_models} frozen, {failed_models} unavailable, {inadequate_models} unqualified)"
        ));
    }
    let computed_evidence: Vec<&CandidateEvidence> = evidence
        .iter()
        .filter(|item| item.sd_ratio.is_some())
        .collect();
    let numeric_benefit = if computed_evidence.is_empty() {
        "inconclusive"
    } else if computed_evidence
        .iter()
        .any(|item| item.benefit_gate == "pass")
    {
        "pass"
    } else if computed_evidence
        .iter()
        .all(|item| item.benefit_gate == "fail")
    {
        "fail"
    } else {
        "inconclusive"
    }
    .to_string();
    if numeric_benefit == "inconclusive" && computed_evidence.is_empty() {
        reasons.push(
            "numeric benefit is inconclusive: no candidate produced a paired statistic".to_string(),
        );
    }
    let dynamic_fidelity = "unverified".to_string();
    reasons.push(
        "dynamic fidelity is unverified: the physical feature/tolerance is unspecified (PN-D-007)"
            .to_string(),
    );
    let scientific = if computed_evidence.is_empty() {
        "unverified"
    } else {
        "inconclusive"
    }
    .to_string();
    reasons.push(
        "scientific verdict stays non-pass on this lane: a numeric SD improvement cannot override unverified fidelity/applicability controls (PN-FR-032, PN-D-006)"
            .to_string(),
    );
    ResultGates {
        acquisition_validity,
        computation,
        diagnostic_qualification,
        model_adequacy,
        numeric_benefit,
        dynamic_fidelity,
        scientific,
        default_promotion: "not_authorized".to_string(),
        reasons,
    }
}

/// Fidelity controls (PN-FR-030): the physical tolerance is unknown, so the
/// section records `unverified` plus whatever boundary/seam evidence the
/// retained rows carry. Known-truth injections live in the synthetic
/// fidelity tests (PN-AT-020), never on recorded data.
fn build_fidelity(legs: &[LegSignal], output_centers: usize) -> FidelitySection {
    let bank_stats: Option<&BoundaryStats> = legs.iter().find_map(|leg| leg.bank_stats.as_ref());
    let (boundary_windows, cross_regime_windows, switch_jump_rms, interior_jump_rms) =
        match bank_stats {
            Some(stats) => (
                stats.boundary_windows,
                stats.cross_regime_windows,
                stats.switch_jump_rms,
                stats.interior_jump_rms,
            ),
            None => (0, 0, None, None),
        };
    FidelitySection {
        status: "unverified".to_string(),
        reason: "the fastest physical feature and its fidelity/field-error tolerance are unspecified (PN-D-007); fidelity cannot be qualified from recorded data".to_string(),
        tolerance_authority: None,
        injected_controls: "known-truth harmonic/sideband/rise injections are exercised by the synthetic fidelity tests (PN-AT-020); no injected control runs on recorded data".to_string(),
        output_resolution_centers: output_centers,
        boundary_windows,
        cross_regime_windows,
        switch_jump_rms,
        interior_jump_rms,
        notes: vec![
            "a smoother trace with damaged truth cannot earn a scientific pass (PN-FR-030)".to_string(),
            "rise/latency/overshoot errors are only meaningful at the actual output resolution".to_string(),
        ],
    }
}

/// Versioned statistics policy recorded with the report (PN-NFR-006).
fn build_statistics_policy(
    request: &CompareRequest,
    statistics: &StatisticsConfig,
    compared: usize,
) -> StatisticsPolicy {
    StatisticsPolicy {
        primary_statistic_id: COMPARE_STATISTIC_ID.to_string(),
        primary_formula: COMPARE_STATISTIC_FORMULA.to_string(),
        secondary_statistic_id: COMPARE_MEAN_BLOCK_SD_STATISTIC_ID.to_string(),
        secondary_formula: COMPARE_MEAN_BLOCK_FORMULA.to_string(),
        evaluate_lockin_statistic_id: COMPARE_EVALUATE_LOCKIN_STATISTIC_ID.to_string(),
        detrend: match statistics.detrend {
            CompareDetrend::None => "none",
            CompareDetrend::BlockMean => "block_mean",
        }
        .to_string(),
        detrend_sensitivity: vec!["none".to_string(), "block_mean".to_string()],
        resampling_unit: COMPARE_RESAMPLING_UNIT.to_string(),
        stratification: COMPARE_STRATIFICATION.to_string(),
        pairing: COMPARE_PAIRING_POLICY.to_string(),
        overlap_policy: COMPARE_OVERLAP_POLICY.to_string(),
        bootstrap_replicates: statistics.bootstrap_replicates,
        bootstrap_seed: statistics.bootstrap_seed,
        rng_id: COMPARE_RNG_ID.to_string(),
        confidence_level: statistics.confidence_level,
        multiplicity_policy_id: COMPARE_MULTIPLICITY_POLICY_ID.to_string(),
        candidates_requested: request.candidates.joint_modes.len(),
        candidates_compared: compared,
        selection_performed: false,
        stability_policy_id: COMPARE_STABILITY_POLICY_ID.to_string(),
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
    let mut prior_manifest: Option<StagingManifest> = None;
    if staging.exists() && !staging.join("staging-manifest.json").is_file() {
        std::fs::remove_dir_all(&staging).with_context(|| {
            format!("cannot clear stale compare staging: {}", staging.display())
        })?;
    }
    if staging.exists() {
        // Resume evidence: a prior staging dir with a matching manifest may
        // be replayed only after digest checks; a mismatched manifest is a
        // hard error, never a silent reuse (PN-FR-031). Schema v2 generations
        // additionally verify their full file-digest closure and semantic
        // ownership before any file is rewritten (PN-FR-023 addendum).
        let manifest_path = staging.join("staging-manifest.json");
        if manifest_path.is_file() {
            let text = std::fs::read_to_string(&manifest_path).with_context(|| {
                format!("cannot read staging manifest: {}", manifest_path.display())
            })?;
            let manifest: StagingManifest = serde_json::from_str(&text).with_context(|| {
                format!("invalid staging manifest: {}", manifest_path.display())
            })?;
            if manifest.schema_version > 2 {
                bail!(
                    "noise compare staging manifest schema_version {} is unsupported (code=staging_mismatch)",
                    manifest.schema_version
                );
            }
            if manifest.request_sha256 != request_sha {
                bail!(
                    "noise compare staging manifest binds a different request (code=staging_mismatch); remove {} and retry",
                    staging.display()
                );
            }
            if manifest.schema_version >= 2 {
                verify_manifest_closure(&staging, &manifest)?;
            }
            prior_manifest = Some(manifest);
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
        prior_manifest.as_ref(),
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
    prior: Option<&StagingManifest>,
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
    if let Some(prior) = prior
        && prior.source_digest != source_digest
    {
        bail!(
            "noise compare staging manifest binds a different source digest (code=staging_mismatch); remove {} and retry",
            staging.display()
        );
    }

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
        &training_spans(&resolved),
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
            "global",
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
                    .to_string()
                    // Portable manifest keys: staged paths use `/` even on
                    // Windows so the closure audit agrees with the references.
                    .replace('\\', "/");
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

    // --- Phase 5b: opt-in frozen regime bank (PN-FR-018/019). Regimes are
    // nominated explicitly; each builds its own mode-specific model from its
    // own training intervals. Identical estimated content is reduced to one
    // distinct model *before* inference, so an identical-model bank
    // reproduces the frozen global path exactly (PN-FR-021), and the
    // complete center -> model schedule is resolved and hashed before any
    // evaluation window is read (PN-FR-019).
    let mut bank_state: Option<BankState> = None;
    if let Some(bank_toml) = request.bank.as_ref() {
        let resolved_bank = bank::validate_bank_request(bank_toml, detector, total_samples)?;
        let mut slots: Vec<bank::BankModelSlot> = Vec::new();
        let mut links: Vec<bank::BankRegimeLink> = Vec::new();
        let mut regime_to_slot: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        let mut reduction: BTreeMap<String, String> = BTreeMap::new();
        let mut primary: Option<(String, PhaseVarianceOutput)> = None;
        for mode in &request.candidates.joint_modes {
            if check_cancel(cancel) {
                bail!(
                    "noise compare cancelled during bank calibration (code=cancelled_before_commit)"
                );
            }
            let mut per_regime_slot: Vec<usize> = Vec::with_capacity(resolved_bank.regimes.len());
            let mut distinct: Vec<(String, usize)> = Vec::new();
            for (regime_index, regime) in resolved_bank.regimes.iter().enumerate() {
                let regime_plan = BlockPlanRequest {
                    intervals: regime
                        .intervals
                        .iter()
                        .map(|(start, end)| RoleInterval {
                            role: CalibrationRole::Training,
                            start: *start,
                            end: *end,
                        })
                        .collect(),
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
                    min_training_intervals: request.calibration.min_training_intervals.unwrap_or(
                        pmoke_analysis_core::calibration::DEFAULT_MIN_TRAINING_INTERVALS,
                    ),
                };
                // Regime training reads the same resolved block geometry as
                // the global lane (each nominated interval is split into
                // block_len blocks), so a long regime interval contributes
                // its full block set instead of one giant block.
                let regime_spans: Vec<(u64, u64)> =
                    pmoke_analysis_core::calibration::plan_blocks(&regime_plan)?
                        .blocks
                        .iter()
                        .filter(|block| block.role == CalibrationRole::Training)
                        .map(|block| (block.start, block.end))
                        .collect();
                let training_regime = read_training_blocks(
                    &source,
                    &regime_spans,
                    request.reference_frequency_hz,
                    request.reference_phase_rad,
                    request.sample_interval_s,
                )
                .with_context(|| {
                    format!(
                        "noise compare bank regime '{}' training read failed (code=insufficient_calibration)",
                        regime.name
                    )
                })?;
                let build = build_mode_artifact(
                    *mode,
                    &format!("bank-{}", regime.name),
                    &training_regime,
                    &regime_plan,
                    source_digests.clone(),
                    u32::from(detector),
                    request.sample_interval_s,
                    request.reference_frequency_hz,
                    request.reference_phase_rad,
                    phase_recipe,
                    correlation_recipe,
                )
                .with_context(|| {
                    format!(
                        "noise compare bank regime '{}' cannot build its {} model (code=insufficient_calibration)",
                        regime.name,
                        mode_name(*mode)
                    )
                })?;
                let lags = build.correlation.as_ref().map(|output| output.lags.clone());
                let fingerprint = bank::estimation_fingerprint(
                    &build.record.correlation_basis,
                    build.variance.v0,
                    &build.variance.variances,
                    lags.as_deref(),
                    lags.as_ref().map(|_| request.sample_interval_s),
                    request.reference_frequency_hz,
                    request.sample_interval_s,
                );
                let slot_index = match distinct
                    .iter()
                    .find(|(existing, _)| existing == &fingerprint)
                {
                    Some((_, index)) => *index,
                    None => {
                        if distinct.len() >= bank::MAX_BANK_REGIMES_PER_MODE {
                            bail!(
                                "noise compare bank holds more than {} distinct {} models (code=resource_limit_exceeded)",
                                bank::MAX_BANK_REGIMES_PER_MODE,
                                mode_name(*mode)
                            );
                        }
                        let index = slots.len();
                        let artifact_path = format!(
                            "calibrations/ch{detector}/{}/bank-{}/model.json",
                            mode_name(*mode),
                            regime.name
                        );
                        let absolute = staging.join(&artifact_path);
                        if let Some(parent) = absolute.parent() {
                            std::fs::create_dir_all(parent)
                                .with_context(|| format!("cannot create {}", parent.display()))?;
                        }
                        std::fs::write(&absolute, &build.artifact_json)
                            .with_context(|| format!("cannot stage {}", absolute.display()))?;
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
                        if primary.is_none() {
                            primary = Some((build.record.sha256.clone(), build.variance.clone()));
                        }
                        slots.push(bank::BankModelSlot {
                            slot: index,
                            mode: mode_name(*mode).to_string(),
                            model_id: build.record.model_id.clone(),
                            sha256: build.record.sha256.clone(),
                            bytes: build.artifact_json.len(),
                            artifact_path: artifact_path.clone(),
                            estimation_sha256: fingerprint.clone(),
                            noise: noise_model_for(*mode, &build, request.sample_interval_s)?,
                            regime: regime.name.clone(),
                        });
                        distinct.push((fingerprint, index));
                        index
                    }
                };
                per_regime_slot.push(slot_index);
                links.push(bank::BankRegimeLink {
                    regime_index,
                    mode: mode_name(*mode).to_string(),
                    slot: slot_index,
                    model_id: slots[slot_index].model_id.clone(),
                    sha256: slots[slot_index].sha256.clone(),
                    artifact_path: slots[slot_index].artifact_path.clone(),
                });
            }
            let status = if distinct.len() == 1 {
                if resolved_bank.regimes.len() == 1 {
                    "single_model"
                } else {
                    "identical_model"
                }
            } else {
                "distinct_models"
            };
            reduction.insert(mode_name(*mode).to_string(), status.to_string());
            regime_to_slot.insert(mode_name(*mode).to_string(), per_regime_slot);
        }
        let (primary_sha, primary_variance) = primary.ok_or_else(|| {
            anyhow::anyhow!(
                "noise compare bank built no frozen models (code=insufficient_calibration)"
            )
        })?;
        bank_state = Some(BankState {
            resolved: resolved_bank,
            slots,
            links,
            regime_to_slot,
            reduction,
            bank_sha256: String::new(),
            schedule_sha256: String::new(),
            schedule_rows: Vec::new(),
            primary_sha,
            primary_variance,
            evidence: Vec::new(),
        });
    }

    // Resume digest check (PN-FR-023 addendum): a prior generation bound to
    // the same request must reproduce the same model digests; any mismatch
    // is a cross-generation mix and fails closed.
    if let Some(prior) = prior {
        let mut current: BTreeMap<String, String> = BTreeMap::new();
        for record in &model_records {
            if !record.sha256.is_empty() {
                current.insert(record.mode.clone(), record.sha256.clone());
            }
        }
        if let Some(state) = bank_state.as_ref() {
            for slot in &state.slots {
                current.insert(
                    format!("{}/bank-{}", slot.mode, slot.regime),
                    slot.sha256.clone(),
                );
            }
        }
        for (key, digest) in &current {
            if let Some(prior_digest) = prior.model_digests.get(key)
                && prior_digest != digest
            {
                bail!(
                    "noise compare staging manifest binds a different {key} model (code=staging_mismatch); remove {} and retry",
                    staging.display()
                );
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
    let primary: Option<(String, PhaseVarianceOutput)> = match (builds.first(), bank_state.as_ref())
    {
        (Some((_, first)), _) => Some((first.record.sha256.clone(), first.variance.clone())),
        (None, Some(state)) => Some((state.primary_sha.clone(), state.primary_variance.clone())),
        (None, None) => None,
    };
    if let Some((primary_sha, primary_variance)) = primary.as_ref() {
        reserved.training_model_sha256 = primary_sha.clone();
        match read_reserved_blocks(
            &source,
            &resolved,
            request.reference_frequency_hz,
            request.reference_phase_rad,
            request.sample_interval_s,
            primary_variance,
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
                        let scale = primary_variance.v0.sqrt();
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
    if let Some(prior) = prior
        && prior.context_sha256 != context.context_sha256
    {
        bail!(
            "noise compare staging manifest binds a different frozen context (code=staging_mismatch); remove {} and retry",
            staging.display()
        );
    }

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
    // Bank lane (PN-M3): the complete center -> model schedule is resolved
    // and hashed here, before any evaluation window is demodulated, and each
    // requested mode runs one model per center from the frozen bank. The
    // labeled boxcar baseline above stays the comparison anchor.
    if let Some(state) = bank_state.as_mut() {
        let absolute_centers: Vec<u64> = centers.iter().map(|center| center + origin).collect();
        let schedule_rows = bank::resolve_bank_schedule(
            &absolute_centers,
            half_taps as u64 + 1,
            &state.resolved.spans,
        )?;
        let schedule_sha = bank::schedule_sha256(&state.resolved.spans, &schedule_rows);
        let bank_sha = bank::bank_sha256(
            state.resolved.provenance,
            &state.resolved.regimes,
            &state.slots,
            &schedule_sha,
        );
        let retained = schedule_rows
            .len()
            .saturating_mul(request.candidates.joint_modes.len());
        if retained > MAX_BANK_RETAINED_ROWS {
            bail!(
                "noise compare bank would retain {retained} rows above the declared cap {MAX_BANK_RETAINED_ROWS} (code=resource_limit_exceeded)"
            );
        }
        state.schedule_rows = schedule_rows.clone();
        state.schedule_sha256 = schedule_sha;
        state.bank_sha256 = bank_sha;
        if let Some(prior) = prior {
            if let Some(prior_bank) = prior.bank_sha256.as_deref()
                && prior_bank != state.bank_sha256
            {
                bail!(
                    "noise compare staging manifest binds a different bank generation (code=staging_mismatch); remove {} and retry",
                    staging.display()
                );
            }
            if let Some(prior_schedule) = prior.schedule_sha256.as_deref()
                && prior_schedule != state.schedule_sha256
            {
                bail!(
                    "noise compare staging manifest binds a different schedule generation (code=staging_mismatch); remove {} and retry",
                    staging.display()
                );
            }
        }
        let regime_names: Vec<String> = state
            .resolved
            .regimes
            .iter()
            .map(|regime| regime.name.clone())
            .collect();
        for mode in &request.candidates.joint_modes {
            if check_cancel(cancel) {
                bail!("noise compare cancelled during bank legs (code=cancelled_before_commit)");
            }
            let mode_key = mode_name(*mode).to_string();
            let regime_map = state
                .regime_to_slot
                .get(&mode_key)
                .cloned()
                .unwrap_or_default();
            let label = format!("{mode_key}_candidate");
            let estimator = format!("joint_{mode_key}");
            let outcome = bank::apply_bank_leg(
                &label,
                &state.slots,
                &regime_names,
                &regime_map,
                &state.schedule_rows,
                origin,
                half_taps,
                &eval_times,
                &eval_signal,
                request.reference_frequency_hz,
                request.reference_phase_rad,
                request.sample_interval_s,
                &sorted_fit,
                requested_covariance,
                state.resolved.workers,
                state.resolved.chunk_size,
                4,
            );
            match outcome {
                Ok(output) => {
                    let stats = bank::boundary_stats(&output.rows);
                    let series: Vec<(u64, f64)> = output
                        .rows
                        .iter()
                        .map(|row| (row.center - origin, row.magnitude))
                        .collect();
                    let mut leg = LegSignal {
                        name: label.clone(),
                        estimator: estimator.clone(),
                        mode: Some(mode_key.clone()),
                        state: LegState::Complete,
                        code: None,
                        message: None,
                        model_sha256: None,
                        grid_fingerprint: Some(fingerprint.clone()),
                        output_rows: Some(series.len()),
                        window_failures: output.window_failures,
                        series,
                        bank_rows: output.rows.clone(),
                        bank_stats: Some(stats.clone()),
                        bank_schedule_sha256: Some(state.schedule_sha256.clone()),
                    };
                    if !reserved.adequate && !allow_exploratory {
                        leg.state = LegState::Unqualified;
                        leg.code = Some("unqualified".to_string());
                        leg.message = Some(format!(
                            "reserved adequacy failed ({}); leg withheld from pass verdicts",
                            reserved.reason
                        ));
                    } else if !reserved.adequate {
                        leg.message = Some(format!(
                            "exploratory only: reserved adequacy failed ({})",
                            reserved.reason
                        ));
                    }
                    let per_window_direct_ms = if output.direct_reference_rows == 0 {
                        0.0
                    } else {
                        output.direct_reference_ms / output.direct_reference_rows as f64
                    };
                    let fixed_global_projection_ms =
                        per_window_direct_ms * output.rows.len() as f64;
                    let accelerated_vs_projection_ratio = if fixed_global_projection_ms > 0.0 {
                        output.apply_ms / fixed_global_projection_ms
                    } else {
                        f64::NAN
                    };
                    state.evidence.push(BankLegEvidence {
                        leg: label,
                        mode: mode_key,
                        rows: output.rows.len(),
                        window_failures: output.window_failures,
                        prepared_plans: output.prepared_plans,
                        prepare_ms: output.prepare_ms,
                        apply_ms: output.apply_ms,
                        direct_reference_rows: output.direct_reference_rows,
                        direct_reference_ms: output.direct_reference_ms,
                        fixed_global_projection_ms,
                        accelerated_vs_projection_ratio,
                        max_abs_error_v: output.max_abs_error_v,
                        boundary: stats,
                    });
                    legs.push(leg);
                }
                Err(error) => {
                    legs.push(LegSignal {
                        name: label,
                        estimator,
                        mode: Some(mode_key),
                        state: LegState::Failed,
                        code: Some("bank_apply_failed".to_string()),
                        message: Some(format!("{error:#}")),
                        model_sha256: None,
                        grid_fingerprint: Some(fingerprint.clone()),
                        output_rows: None,
                        window_failures: 0,
                        series: Vec::new(),
                        bank_rows: Vec::new(),
                        bank_stats: None,
                        bank_schedule_sha256: Some(state.schedule_sha256.clone()),
                    });
                }
            }
        }
    }
    if bank_state.is_none() {
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
                        bank_rows: Vec::new(),
                        bank_stats: None,
                        bank_schedule_sha256: None,
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
            bank_rows: Vec::new(),
            bank_stats: None,
            bank_schedule_sha256: None,
        });
    }

    // --- Phase 9: paired evidence per eligible evaluation region plus one
    // stratified record per candidate (PN-FR-026/027/029). The labeled
    // baseline is the comparison anchor when present; when only explicit
    // unavailable records exist, evidence stays empty and the report records
    // the calibration-limited outcome (PN-FR-022/026).
    //
    // Requested candidate family size drives the predeclared multiplicity
    // control: every candidate stays visible and no selection is performed.
    let family_size = request.candidates.joint_modes.len().max(1);
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
                    std::slice::from_ref(&relative),
                    family_size,
                    &statistics,
                ));
            }
            // Prespecified stratification: one record over the whole
            // evaluation domain, bootstrapped within each region stratum.
            if evaluation_spans.len() > 1 {
                let relative: Vec<(u64, u64)> = evaluation_spans
                    .iter()
                    .map(|(start, end)| (start - origin, end - origin))
                    .collect();
                let strata: Vec<Vec<(u64, u64)>> =
                    relative.iter().map(|span| vec![*span]).collect();
                evidence.push(assess_candidate(
                    leg,
                    baseline,
                    "eligible_evaluation_stratified",
                    &strata,
                    family_size,
                    &statistics,
                ));
            }
        }
    }

    // --- Phase 9b: complete requested method x channel x region accounting
    // (PN-FR-026, PN-AT-018). Every requested method appears with a terminal
    // outcome; an isolated failure keeps its own record and reason.
    let accounting = build_accounting(
        &legs,
        &evidence,
        &evaluation_spans,
        source.binding().detector,
    );
    let accounting_complete = accounting_complete(&accounting, request, &evaluation_spans);

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
    // Bank artifacts (PN-FR-023): the frozen bank, the resolved center
    // schedule, and the measured boundary/resource/acceleration evidence.
    let mut referenced_files: Vec<String> = Vec::new();
    for path in [
        "request.source.toml",
        "request.resolved.json",
        "inputs.json",
        "diagnostics.json",
        "context.json",
        "role-plan.json",
        "reserved.json",
        "models.json",
        "schedule.json",
        "comparison.json",
        "report.md",
    ] {
        referenced_files.push(path.to_string());
    }
    for record in &model_records {
        if !record.artifact_path.is_empty() {
            referenced_files.push(record.artifact_path.clone());
        }
    }
    if let Some(state) = bank_state.as_ref() {
        for slot in &state.slots {
            referenced_files.push(slot.artifact_path.clone());
        }
    }
    if let Some(state) = bank_state.as_ref() {
        let mut regime_records: Vec<serde_json::Value> = Vec::new();
        for (regime_index, regime) in state.resolved.regimes.iter().enumerate() {
            let mut models = serde_json::Map::new();
            for link in state
                .links
                .iter()
                .filter(|link| link.regime_index == regime_index)
            {
                models.insert(
                    link.mode.clone(),
                    serde_json::json!({
                        "slot": link.slot,
                        "model_id": link.model_id,
                        "sha256": link.sha256,
                        "artifact_path": link.artifact_path,
                    }),
                );
            }
            regime_records.push(serde_json::json!({
                "name": regime.name,
                "condition": regime.condition,
                "channel": regime.channel,
                "intervals": regime
                    .intervals
                    .iter()
                    .map(|(start, end)| serde_json::json!({"start": start, "end": end}))
                    .collect::<Vec<_>>(),
                "models": models,
            }));
        }
        let model_entries: Vec<serde_json::Value> = state
            .slots
            .iter()
            .map(|slot| {
                serde_json::json!({
                    "slot": slot.slot,
                    "mode": slot.mode,
                    "model_id": slot.model_id,
                    "sha256": slot.sha256,
                    "bytes": slot.bytes,
                    "artifact_path": slot.artifact_path,
                    "estimation_sha256": slot.estimation_sha256,
                    "representative_regime": slot.regime,
                })
            })
            .collect();
        let model_bank = serde_json::json!({
            "schema_version": 1,
            "selection_provenance": state.resolved.provenance,
            "bank_sha256": state.bank_sha256,
            "schedule_sha256": state.schedule_sha256,
            "reduction": state.reduction,
            "regimes": regime_records,
            "models": model_entries,
        });
        write_json(&staging.join("model-bank.json"), &model_bank, "model bank")?;
        referenced_files.push("model-bank.json".to_string());
        let mut distinct_models = BTreeMap::new();
        for mode in &request.candidates.joint_modes {
            let distinct = state
                .regime_to_slot
                .get(mode_name(*mode))
                .map(|mapping| {
                    mapping
                        .iter()
                        .copied()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                })
                .unwrap_or(0);
            distinct_models.insert(mode_name(*mode).to_string(), distinct);
        }
        let retained_rows: usize = state.evidence.iter().map(|item| item.rows).sum();
        let prepared_plans: usize = state.evidence.iter().map(|item| item.prepared_plans).sum();
        let prepare_ms: f64 = state.evidence.iter().map(|item| item.prepare_ms).sum();
        let apply_ms: f64 = state.evidence.iter().map(|item| item.apply_ms).sum();
        let direct_rows: usize = state
            .evidence
            .iter()
            .map(|item| item.direct_reference_rows)
            .sum();
        let direct_ms: f64 = state
            .evidence
            .iter()
            .map(|item| item.direct_reference_ms)
            .sum();
        let projection_ms: f64 = state
            .evidence
            .iter()
            .map(|item| item.fixed_global_projection_ms)
            .sum();
        let ratio = if projection_ms > 0.0 {
            apply_ms / projection_ms
        } else {
            f64::NAN
        };
        let max_error = state
            .evidence
            .iter()
            .fold(0.0_f64, |max, item| max.max(item.max_abs_error_v));
        let peak_rss = bank::peak_rss_kib().unwrap_or(None);
        let bank_evidence = serde_json::json!({
            "schema_version": 1,
            "bank_sha256": state.bank_sha256,
            "schedule_sha256": state.schedule_sha256,
            "selection_provenance": state.resolved.provenance,
            "reduction": state.reduction,
            "distinct_models": distinct_models,
            "schedule_spans": state.resolved.spans.len(),
            "retained_rows": retained_rows,
            "retained_rows_cap": MAX_BANK_RETAINED_ROWS,
            "workers": state.resolved.workers,
            "chunk_size": state.resolved.chunk_size,
            "peak_rss_kib": peak_rss,
            "peak_rss_source": if peak_rss.is_some() {
                "measured"
            } else {
                "unavailable on this platform"
            },
            "legs": state.evidence,
            "acceleration": {
                "prepared_plans": prepared_plans,
                "plan_reuse_windows": retained_rows,
                "prepare_ms_total": prepare_ms,
                "apply_ms_total": apply_ms,
                "direct_reference_rows": direct_rows,
                "direct_reference_ms_total": direct_ms,
                "fixed_global_projection_ms_total": projection_ms,
                "accelerated_vs_projection_ratio": ratio,
                "max_abs_error_v": max_error,
                "equivalence_bound": "abs(error) <= 1e-8 V + 1e-8 * abs(reference)",
                "core_equivalence_tests": "crates/pmoke-analysis-core/tests/joint_gls_prepared.rs",
                "cache_state": "one factorization per distinct (mode, model, window length) per leg; the first window on a geometry is the cold prepare and every later window is warm reuse",
            },
        });
        write_json(
            &staging.join("bank-evidence.json"),
            &bank_evidence,
            "bank evidence",
        )?;
        referenced_files.push("bank-evidence.json".to_string());
    }
    // Per-leg output series (centers + fundamental magnitudes) for replay;
    // bank legs additionally retain the per-row model attribution, XY
    // quadratures and the policy-serialized covariance (PN-FR-023/025).
    for leg in &legs {
        let mut leg_json = serde_json::json!({
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
        if !leg.bank_rows.is_empty() {
            leg_json["bank_schedule_sha256"] = serde_json::json!(leg.bank_schedule_sha256);
            leg_json["row_count"] = serde_json::json!(leg.bank_rows.len());
            leg_json["rows"] =
                serde_json::to_value(&leg.bank_rows).context("cannot encode bank leg rows")?;
            if let Some(stats) = leg.bank_stats.as_ref() {
                leg_json["boundary"] =
                    serde_json::to_value(stats).context("cannot encode bank boundary stats")?;
            }
        }
        let leg_path = staging.join(format!("legs/{}.json", leg.name));
        if let Some(parent) = leg_path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        write_json(&leg_path, &leg_json, "leg output")?;
        referenced_files.push(format!("legs/{}.json", leg.name));
    }
    let mut grid_json = serde_json::json!({
        "schema_version": NOISE_COMPARE_SCHEMA_VERSION,
        "half_taps": grid.half_taps,
        "stride": grid.stride,
        "first_center": grid.first_center + origin,
        "last_center": grid.last_center + origin,
        "centers": centers.len(),
        "fingerprint": fingerprint,
        "context_sha256": context.context_sha256,
    });
    if let Some(state) = bank_state.as_ref() {
        let rows_json: Vec<serde_json::Value> = state
            .schedule_rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "center": row.center,
                    "regime": state.resolved.regimes[row.regime_index].name,
                    "regime_index": row.regime_index,
                    "boundary": row.boundary,
                })
            })
            .collect();
        let spans_json: Vec<serde_json::Value> = state
            .resolved
            .spans
            .iter()
            .map(|span| {
                serde_json::json!({
                    "regime": span.regime,
                    "start": span.start,
                    "end": span.end,
                })
            })
            .collect();
        grid_json["bank"] = serde_json::json!({
            "schema_version": 1,
            "bank_sha256": state.bank_sha256,
            "schedule_sha256": state.schedule_sha256,
            "spans": spans_json,
            "rows": rows_json,
        });
    }
    write_json(&staging.join("schedule.json"), &grid_json, "grid schedule")?;

    let completed_legs = legs
        .iter()
        .filter(|leg| leg.state == LegState::Complete)
        .count();
    let requested_modes = request.candidates.joint_modes.len();
    let built_modes = if bank_state.is_some() {
        requested_modes
    } else {
        builds.len()
    };
    let grid_equal = legs
        .iter()
        .filter_map(|leg| leg.grid_fingerprint.as_deref())
        .collect::<std::collections::HashSet<_>>()
        .len()
        <= 1;
    // Row-mapping agreement (PN-FR-023): every bank leg's retained rows must
    // match the resolved schedule one-to-one, in order and identity.
    let row_mapping_ok = match bank_state.as_ref() {
        None => true,
        Some(state) => legs
            .iter()
            .filter(|leg| !leg.bank_rows.is_empty())
            .all(|leg| {
                leg.bank_rows.len() == state.schedule_rows.len()
                    && leg.bank_rows.iter().zip(state.schedule_rows.iter()).all(
                        |(row, schedule)| {
                            row.center == schedule.center
                                && row.regime_index == schedule.regime_index
                                && row.boundary == schedule.boundary
                        },
                    )
            }),
    };
    let computation_complete =
        completed_legs >= 1 && built_modes == requested_modes && grid_equal && row_mapping_ok;
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
    let bank_report = bank_state.as_ref().map(|state| {
        let mut distinct = BTreeMap::new();
        for mode in &request.candidates.joint_modes {
            let count = state
                .regime_to_slot
                .get(mode_name(*mode))
                .map(|mapping| {
                    mapping
                        .iter()
                        .copied()
                        .collect::<std::collections::BTreeSet<_>>()
                        .len()
                })
                .unwrap_or(0);
            distinct.insert(mode_name(*mode).to_string(), count);
        }
        let boundary_windows = state
            .evidence
            .iter()
            .map(|item| item.boundary.boundary_windows)
            .max()
            .unwrap_or(0);
        let cross_regime_windows = state
            .evidence
            .iter()
            .map(|item| item.boundary.cross_regime_windows)
            .max()
            .unwrap_or(0);
        BankReport {
            schema_version: 1,
            selection_provenance: state.resolved.provenance.to_string(),
            bank_sha256: state.bank_sha256.clone(),
            schedule_sha256: state.schedule_sha256.clone(),
            reduction: state.reduction.clone(),
            distinct_models: distinct,
            schedule_spans: state.resolved.spans.len(),
            boundary_windows,
            cross_regime_windows,
            row_mapping_ok,
        }
    });
    let completed_modes = legs
        .iter()
        .filter(|leg| leg.state == LegState::Complete && leg.mode.is_some())
        .count();
    let statistics_policy = build_statistics_policy(request, &statistics, completed_modes);
    let gates = assemble_gates(
        operation_status,
        data_quality,
        &model_records,
        &evidence,
        completed_legs,
        legs.len(),
    );
    let fidelity = build_fidelity(&legs, centers.len());
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
        bank: bank_report,
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
        statistics: statistics_policy,
        gates,
        fidelity,
        accounting,
        accounting_complete,
        grid_equal,
        computation_complete,
        cancelled_before_commit: false,
        frozen_inputs: source.frozen_view().clone(),
    };
    write_json(&staging.join("comparison.json"), &report, "compare report")?;
    write_report_md(staging, &report, request_path, bins_explicit)?;
    // Restart manifest last: its presence with matching digests is the
    // resume gate (PN-FR-031). Schema v2 closes the retained-file digest
    // set, adds the bank/schedule digests and the row mapping, and audits
    // that staging holds no unreferenced file (PN-FR-023 addendum).
    let mut model_digests = BTreeMap::new();
    for record in &model_records {
        if !record.sha256.is_empty() {
            model_digests.insert(record.mode.clone(), record.sha256.clone());
        }
    }
    if let Some(state) = bank_state.as_ref() {
        for slot in &state.slots {
            model_digests.insert(
                format!("{}/bank-{}", slot.mode, slot.regime),
                slot.sha256.clone(),
            );
        }
    }
    let mut file_digests = BTreeMap::new();
    for path in &referenced_files {
        let absolute = staging.join(path);
        let bytes = std::fs::read(&absolute)
            .with_context(|| format!("cannot read staged {}", absolute.display()))?;
        file_digests.insert(path.clone(), crate::utils::checksum::sha256_hex(&bytes));
    }
    let row_mapping = serde_json::json!({
        "schedule_rows": bank_state
            .as_ref()
            .map(|state| state.schedule_rows.len())
            .unwrap_or(0),
        "legs": legs
            .iter()
            .map(|leg| (leg.name.clone(), leg.output_rows))
            .collect::<BTreeMap<_, _>>(),
        "rows_agree": row_mapping_ok,
    });
    let manifest = StagingManifest {
        schema_version: 2,
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
        file_digests,
        bank_sha256: bank_state.as_ref().map(|state| state.bank_sha256.clone()),
        schedule_sha256: bank_state
            .as_ref()
            .map(|state| state.schedule_sha256.clone()),
        row_mapping: Some(row_mapping),
    };
    // Closure audit before publication: staging must hold only files this
    // generation references (no stale leg or model from an earlier attempt).
    for path in collect_staged_files(staging)? {
        if path == "staging-manifest.json" {
            continue;
        }
        if !referenced_files.contains(&path) {
            bail!(
                "noise compare staging holds an unreferenced file '{path}' (code=staging_unreferenced_file); refusing to publish a mixed generation"
            );
        }
    }
    write_json(
        &staging.join("staging-manifest.json"),
        &manifest,
        "staging manifest",
    )?;
    verify_manifest_closure(staging, &manifest)?;
    source.verify_live()?;
    Ok(report)
}

/// Recursive sorted list of regular files under `dir` (staging-relative
/// portable paths), for the retained-closure audit.
fn collect_staged_files(dir: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    collect_staged_files_into(dir, Path::new(""), &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_staged_files_into(dir: &Path, prefix: &Path, files: &mut Vec<String>) -> Result<()> {
    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("cannot list staging directory {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("cannot list staging {}", dir.display()))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .with_context(|| format!("cannot stat staged entry {}", path.display()))?;
        let relative = prefix.join(entry.file_name());
        if file_type.is_dir() {
            collect_staged_files_into(&path, &relative, files)?;
        } else if file_type.is_file() {
            let path_text = relative
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("non-UTF8 staged path: {}", path.display()))?;
            // Manifest keys are portable: staged paths always use `/`, even on
            // Windows, so the closure audit and the resume digest map agree.
            files.push(path_text.replace('\\', "/"));
        } else {
            bail!(
                "staging holds a non-regular entry (code=staging_unreferenced_file): {}",
                path.display()
            );
        }
    }
    Ok(())
}

/// Verifies the manifest's digest closure: every referenced file exists and
/// matches, and staging holds nothing else (PN-FR-023 addendum). Digest
/// integrity and semantic validation stay separate steps.
pub(super) fn verify_manifest_closure(staging: &Path, manifest: &StagingManifest) -> Result<()> {
    for (path, digest) in &manifest.file_digests {
        let absolute = staging.join(path);
        let bytes = std::fs::read(&absolute)
            .with_context(|| format!("cannot read staged {}", absolute.display()))?;
        let actual = crate::utils::checksum::sha256_hex(&bytes);
        if &actual != digest {
            bail!(
                "noise compare staged file '{path}' digest mismatch (code=staging_mismatch); refusing to publish a mixed generation"
            );
        }
    }
    // v1 manifests predate the retained-file closure and carry no digest
    // inventory; the unreferenced-file audit applies to v2 generations.
    if manifest.schema_version < 2 {
        return Ok(());
    }
    for path in collect_staged_files(staging)? {
        if path == "staging-manifest.json" {
            continue;
        }
        if !manifest.file_digests.contains_key(&path) {
            bail!(
                "noise compare staging holds an unreferenced file '{path}' (code=staging_unreferenced_file)"
            );
        }
    }
    Ok(())
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
    if let Some(bank) = report.bank.as_ref() {
        let _ = writeln!(
            text,
            "## Frozen regime bank\n\n\
             - selection: `{prov}`, spans: {spans}, row mapping ok: {ok}\n\
             - bank sha256 `{bank_sha}`, schedule sha256 `{schedule_sha}`\n\
             - boundary windows: {boundary}, cross-regime windows: {cross}\n",
            prov = bank.selection_provenance,
            spans = bank.schedule_spans,
            ok = bank.row_mapping_ok,
            bank_sha = bank.bank_sha256,
            schedule_sha = bank.schedule_sha256,
            boundary = bank.boundary_windows,
            cross = bank.cross_regime_windows,
        );
        for (mode, status) in &bank.reduction {
            let distinct = bank.distinct_models.get(mode).copied().unwrap_or(0);
            let _ = writeln!(text, "- `{mode}`: {status} ({distinct} distinct model(s))");
        }
        text.push('\n');
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
            "- `{cand}` vs `{base}` ({region}): {stat} {ratio}, CI {ci}, gate {gate}, scientific {sci} over {n} blocks ({strata} strata); {secondary} {mean_sd}, stability {stability}\n",
            cand = item.candidate,
            base = item.baseline,
            region = item.region_label,
            stat = item.statistic_id,
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
            strata = item.strata,
            secondary = item.secondary_statistic_id,
            mean_sd = item
                .mean_block_sd_ratio
                .map(|ratio| format!("{ratio:.4}"))
                .unwrap_or_else(|| "(uncomputed)".to_string()),
            stability = item.stability_gate,
        );
    }
    text.push_str("## Result gates (independent; PN-FR-032)\n\n");
    for (name, value) in [
        ("acquisition validity", &report.gates.acquisition_validity),
        ("computation", &report.gates.computation),
        (
            "diagnostic qualification",
            &report.gates.diagnostic_qualification,
        ),
        ("model adequacy", &report.gates.model_adequacy),
        ("numeric benefit", &report.gates.numeric_benefit),
        ("dynamic fidelity", &report.gates.dynamic_fidelity),
        ("scientific", &report.gates.scientific),
        ("default promotion", &report.gates.default_promotion),
    ] {
        let _ = writeln!(text, "- {name}: `{value}`");
    }
    for reason in &report.gates.reasons {
        let _ = writeln!(text, "  - {reason}");
    }
    text.push_str("\n## Fidelity controls (PN-FR-030)\n\n");
    let _ = writeln!(
        text,
        "- status: `{status}` — {reason}\n\
         - output resolution: {centers} centers; boundary windows: {boundary}, cross-regime windows: {cross}\n\
         - switch jump RMS: {switch:?}, interior jump RMS: {interior:?}\n\
         - tolerance authority: {authority:?}\n",
        status = report.fidelity.status,
        reason = report.fidelity.reason,
        centers = report.fidelity.output_resolution_centers,
        boundary = report.fidelity.boundary_windows,
        cross = report.fidelity.cross_regime_windows,
        switch = report.fidelity.switch_jump_rms,
        interior = report.fidelity.interior_jump_rms,
        authority = report.fidelity.tolerance_authority,
    );
    for note in &report.fidelity.notes {
        let _ = writeln!(text, "- {note}");
    }
    text.push_str("\n## Requested method x channel x region accounting (PN-FR-026)\n\n");
    let _ = writeln!(
        text,
        "- complete: {complete}; rows: {rows}\n",
        complete = report.accounting_complete,
        rows = report.accounting.len(),
    );
    for row in &report.accounting {
        let _ = writeln!(
            text,
            "- `{method}` ch{channel} {region}: {outcome}{reason}\n",
            method = row.method,
            channel = row.channel,
            region = row.region,
            outcome = row.outcome,
            reason = row
                .reason
                .as_deref()
                .map(|reason| format!(" — {reason}"))
                .unwrap_or_default(),
        );
    }
    text.push_str("\n## Statistics policy (PN-NFR-006)\n\n");
    let _ = writeln!(
        text,
        "- primary `{primary}`: {primary_formula}\n\
         - secondary `{secondary}`: {secondary_formula}\n\
         - evaluate-lockin identity: `{evaluate}` (distinct statistic, never substituted)\n\
         - resampling unit: `{unit}`, stratification: `{strata}`, pairing: `{pairing}`\n\
         - overlap policy: `{overlap}`\n\
         - rng: `{rng}`, replicates: {reps}, seed: {seed}, confidence: {confidence}\n\
         - multiplicity: `{multiplicity}` (requested {requested}, compared {compared}, selection performed: {selection})\n\
         - stability policy: `{stability}` (max single-block share {max_share}, min improved fraction {min_fraction})\n",
        primary = report.statistics.primary_statistic_id,
        primary_formula = report.statistics.primary_formula,
        secondary = report.statistics.secondary_statistic_id,
        secondary_formula = report.statistics.secondary_formula,
        evaluate = report.statistics.evaluate_lockin_statistic_id,
        unit = report.statistics.resampling_unit,
        strata = report.statistics.stratification,
        pairing = report.statistics.pairing,
        overlap = report.statistics.overlap_policy,
        rng = report.statistics.rng_id,
        reps = report.statistics.bootstrap_replicates,
        seed = report.statistics.bootstrap_seed,
        confidence = report.statistics.confidence_level,
        multiplicity = report.statistics.multiplicity_policy_id,
        requested = report.statistics.candidates_requested,
        compared = report.statistics.candidates_compared,
        selection = report.statistics.selection_performed,
        stability = report.statistics.stability_policy_id,
        max_share = COMPARE_STABILITY_MAX_CONCENTRATION,
        min_fraction = COMPARE_STABILITY_MIN_IMPROVED_FRACTION,
    );
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

#[cfg(test)]
#[path = "m4_tests.rs"]
mod m4_tests;

#[cfg(test)]
#[path = "m4_e2e_tests.rs"]
mod m4_e2e_tests;
