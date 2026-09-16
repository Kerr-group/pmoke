//! Mechanism-agnostic signal diagnostics (PN-M1b, Issue #246).
//!
//! `run_signal_diagnostics` consumes the resolved role plan (already frozen
//! by [`RecordedSource`](crate::utils::recorded_source::RecordedSource)),
//! reads the training blocks with bounded block reads, fits the fixed
//! nuisance recipe per block, and reports phase-binned residual variance,
//! residual autocorrelation, and coverage evidence. Every verdict is
//! mechanism-agnostic (PN-FR-011/012): `supported_structure`,
//! `no_detectable_change`, `unknown`, or `insufficient_evidence` — never a
//! physical noise cause.
//!
//! Recorded-only: no acquisition, no instrument access, no config mutation.

use anyhow::{Context, Result, bail};
use pmoke_analysis_core::calibration::{
    CorrelationOutput, CorrelationRecipe, DEFAULT_CORRELATION_ETA, DEFAULT_FLOOR_RATIO,
    DEFAULT_MAX_LAG, DEFAULT_MIN_CYCLES_PER_BIN, DEFAULT_MIN_SAMPLES_PER_BIN,
    DEFAULT_SHRINKAGE_ALPHA, NUISANCE_PARAMETERS, PhaseVarianceOutput, PhaseVarianceRecipe,
    assemble_samples, estimate_correlation, estimate_phase_variance, fit_nuisance,
};
use pmoke_analysis_core::joint::JointSolverTolerances;
use serde::{Deserialize, Serialize};

use crate::utils::recorded_source::RecordedSource;

use super::plan::ResolvedPlan;

/// Measurement sections of the diagnostic report (schema v1, PN-A-004).
/// Absent measurements stay `None`: `unknown` is reported with reasons,
/// never filled silently (PN-FR-012/035).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalDiagnostics {
    pub acquisition_qc: AcquisitionQc,
    pub nuisance_observability: NuisanceObservability,
    pub phase_variance: PhaseVarianceSection,
    pub correlation: CorrelationSection,
    pub identifiability: IdentifiabilitySection,
    /// Recorded change-rule qualification (error/power limits, PN-FR-013).
    pub change_qualification: ChangeQualification,
}

/// Acquisition quality control over the analyzed blocks (PN-FR-009).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcquisitionQc {
    pub blocks_read: usize,
    pub samples_read: usize,
    pub samples_expected: usize,
    pub non_finite_rejected: bool,
    pub timebase_drift_max: f64,
    pub timebase_ok: bool,
    pub channel_parity_ok: bool,
}

/// Nuisance-fit observability per analyzed block (PN-FR-010).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NuisanceObservability {
    pub recipe_id: String,
    pub blocks_fitted: usize,
    pub ranks: Vec<usize>,
    pub conditions: Vec<f64>,
    pub residual_rms: Vec<f64>,
    pub rank_deficient_blocks: Vec<usize>,
    pub ill_conditioned_blocks: Vec<usize>,
}

/// Phase-binned residual variance (PN-FR-010/012), mechanism-agnostic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PhaseVarianceSection {
    pub recipe_id: String,
    pub bins: usize,
    pub v0: f64,
    pub variances: Vec<f64>,
    pub counts: Vec<usize>,
    pub distinct_cycles: Vec<usize>,
    pub contributing_blocks: usize,
    pub floor_activations: Vec<usize>,
    pub peak_to_median_ratio: f64,
}

/// Residual autocorrelation (PN-FR-010), bounded support with tail note.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CorrelationSection {
    pub recipe_id: String,
    pub lags: Vec<f64>,
    pub lag_step_s: f64,
    pub support_samples: usize,
    pub physical_duration_s: f64,
    pub taper_id: String,
    pub eta: f64,
    pub spd_validated: bool,
    pub tail_energy: Option<f64>,
    pub max_tail_lag: usize,
    pub max_abs_offpeak: f64,
}

/// Identifiability verdict (PN-FR-012): the workflow distinguishes
/// `supported_structure` from `no_detectable_change`, `unknown`, and
/// `insufficient_evidence`. `non_identifiable` is first-class, never a
/// silent pass.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentifiabilitySection {
    pub verdict: String,
    pub reasons: Vec<String>,
    pub non_identifiable: bool,
}

/// Fixed diagnostic recipes (no hyperparameter search, PN-FR-010).
pub const NUISANCE_RECIPE_ID: &str = "nuisance-dc-trend-h12/v1";
pub const PHASE_VARIANCE_RECIPE_ID: &str = "phase-variance-64bin/v1";
pub const CORRELATION_RECIPE_ID: &str = "correlation-bartlett-j256/v1";

// ---------------------------------------------------------------------------
// Versioned calibrated change rule (PN-FR-013, PN-M4).
//
// The change rule separates "the residual structure justifies a
// regime-dependent calibration" (`supported_structure`) from
// "no detectable change" and an explicit abstention (`unknown`). Its
// thresholds are not descriptive guesses: they were selected on the disjoint
// calibration seeds of the registered synthetic benchmark
// (`super::qualification`, `change-structure-benchmark/v1`) and verified on
// fresh evaluation seeds (PN-A-006): >=200 independent null and >=200
// alternative datasets per family with binomial intervals; null
// false-selection 95% upper bound <= 0.10 and meaningful-alternative
// detection 95% lower bound >= 0.80.
//
// The measured limits below are the recorded output of that benchmark run.
// `qualification::tests::frozen_rule_matches_registered_benchmark` re-runs
// the registered benchmark and fails if these constants drift from it.
// ---------------------------------------------------------------------------

/// Versioned identity of the calibrated change rule.
pub const CHANGE_RULE_ID: &str = "phase-correlation-change/v1";

/// Calibrated detection thresholds: a residual process is reported as
/// `supported_structure` when either the phase-variance peak/median ratio or
/// the strongest off-peak autocorrelation reaches its threshold.
pub const CHANGE_PHASE_PEAK_MEDIAN_THRESHOLD: f64 = 1.15;
pub const CHANGE_CORRELATION_OFFPEAK_THRESHOLD: f64 = 0.06;

/// No-change band: below both bands the rule reports `no_detectable_change`;
/// between the band and the detection threshold it abstains (`unknown`). The
/// bands are the worst-case per-family 97.5th percentiles of the
/// calibration-stage no-change control families (pooled stationary-iid and
/// scalar-only-variance datasets), so a `no_detectable_change` report is
/// calibrated rather than assumed.
pub const CHANGE_NO_CHANGE_PHASE_BAND: f64 = 1.128_398_355_731_182_5;
pub const CHANGE_NO_CHANGE_CORRELATION_BAND: f64 = 0.019_699_427_486_708_58;

/// Registered benchmark identity and inherited engineering targets (PN-A-006).
pub const CHANGE_BENCHMARK_ID: &str = "change-structure-benchmark/v1";
pub const CHANGE_BENCHMARK_DATASETS_PER_FAMILY: usize = 200;
pub const CHANGE_NULL_FP_TARGET_UPPER: f64 = 0.10;
pub const CHANGE_POWER_TARGET_LOWER: f64 = 0.80;
/// Internal QA target (not part of PN-A-006): the calibrated no-change
/// families must be reported `no_detectable_change` at least this often.
pub const CHANGE_NO_CHANGE_SPECIFICITY_TARGET: f64 = 0.90;

/// Measured worst-case limits recorded by the registered benchmark evaluation
/// stage (fresh seeds): null-family false-selection 95% upper bound and
/// alternative-family detection 95% lower bound.
pub const CHANGE_MEASURED_NULL_FP_UPPER_BOUND: f64 = 0.027773704428405807;
pub const CHANGE_MEASURED_POWER_LOWER_BOUND: f64 = 0.9811546735929196;
/// `qualified` when every family met its PN-A-006 target on fresh seeds;
/// otherwise `unverified`/`failed` with the per-family report retained.
pub const CHANGE_QUALIFICATION_STATUS: &str = "qualified";

/// Calibrated change decision (see the frozen thresholds above).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeDecision {
    SupportedStructure,
    NoDetectableChange,
    Unknown,
}

/// One frozen threshold set of the versioned change rule.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ChangeThresholds {
    pub phase_peak_median: f64,
    pub correlation_offpeak: f64,
    pub no_change_phase_band: f64,
    pub no_change_correlation_band: f64,
}

impl ChangeThresholds {
    /// The frozen product thresholds (calibrated; see the constants above).
    pub fn frozen() -> Self {
        Self {
            phase_peak_median: CHANGE_PHASE_PEAK_MEDIAN_THRESHOLD,
            correlation_offpeak: CHANGE_CORRELATION_OFFPEAK_THRESHOLD,
            no_change_phase_band: CHANGE_NO_CHANGE_PHASE_BAND,
            no_change_correlation_band: CHANGE_NO_CHANGE_CORRELATION_BAND,
        }
    }
}

/// Decision under explicit thresholds; the registered benchmark uses this
/// with candidate thresholds during calibration, and the workflow uses
/// [`change_decision`] with the frozen constants.
pub fn change_decision_with(
    thresholds: ChangeThresholds,
    phase_peak_median: f64,
    max_abs_offpeak: f64,
) -> ChangeDecision {
    if !phase_peak_median.is_finite() || !max_abs_offpeak.is_finite() {
        return ChangeDecision::Unknown;
    }
    if phase_peak_median >= thresholds.phase_peak_median
        || max_abs_offpeak >= thresholds.correlation_offpeak
    {
        return ChangeDecision::SupportedStructure;
    }
    if phase_peak_median <= thresholds.no_change_phase_band
        && max_abs_offpeak <= thresholds.no_change_correlation_band
    {
        return ChangeDecision::NoDetectableChange;
    }
    ChangeDecision::Unknown
}

/// The workflow decision under the frozen calibrated thresholds.
pub fn change_decision(phase_peak_median: f64, max_abs_offpeak: f64) -> ChangeDecision {
    change_decision_with(
        ChangeThresholds::frozen(),
        phase_peak_median,
        max_abs_offpeak,
    )
}

/// Recorded qualification of the change rule (PN-FR-013: report error/power
/// before any regime-dependent calibration is recommended).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeQualification {
    pub rule_id: String,
    pub benchmark_id: String,
    pub phase_peak_median_threshold: f64,
    pub correlation_offpeak_threshold: f64,
    pub datasets_per_family: usize,
    pub null_fp_target_upper: f64,
    pub power_target_lower: f64,
    pub no_change_specificity_target: f64,
    pub measured_null_fp_upper_bound: f64,
    pub measured_power_lower_bound: f64,
    pub status: String,
}

/// The frozen qualification record attached to every diagnostic measurement.
pub fn change_qualification() -> ChangeQualification {
    ChangeQualification {
        rule_id: CHANGE_RULE_ID.to_string(),
        benchmark_id: CHANGE_BENCHMARK_ID.to_string(),
        phase_peak_median_threshold: CHANGE_PHASE_PEAK_MEDIAN_THRESHOLD,
        correlation_offpeak_threshold: CHANGE_CORRELATION_OFFPEAK_THRESHOLD,
        datasets_per_family: CHANGE_BENCHMARK_DATASETS_PER_FAMILY,
        null_fp_target_upper: CHANGE_NULL_FP_TARGET_UPPER,
        power_target_lower: CHANGE_POWER_TARGET_LOWER,
        no_change_specificity_target: CHANGE_NO_CHANGE_SPECIFICITY_TARGET,
        measured_null_fp_upper_bound: CHANGE_MEASURED_NULL_FP_UPPER_BOUND,
        measured_power_lower_bound: CHANGE_MEASURED_POWER_LOWER_BOUND,
        status: CHANGE_QUALIFICATION_STATUS.to_string(),
    }
}

/// Runs the signal diagnostics over the resolved training blocks and
/// returns the measurement sections plus the mechanism-agnostic finding.
pub fn run_signal_diagnostics(
    source: &RecordedSource,
    resolved: &ResolvedPlan,
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    sample_interval_s: f64,
) -> Result<(SignalDiagnostics, String, String)> {
    if !(reference_frequency_hz.is_finite() && reference_frequency_hz > 0.0) {
        bail!("noise diagnostics need a positive finite reference_frequency_hz");
    }
    if !reference_phase_rad.is_finite() {
        bail!("noise diagnostics need a finite reference_phase_rad");
    }
    if !(sample_interval_s.is_finite() && sample_interval_s > 0.0) {
        bail!("noise diagnostics need a positive finite sample_interval_s");
    }
    let sample_rate_hz = 1.0 / sample_interval_s;
    let training = training_blocks(resolved)?;
    let mut block_times: Vec<Vec<f64>> = Vec::with_capacity(training.len());
    let mut detector_blocks: Vec<Vec<f64>> = Vec::with_capacity(training.len());
    let mut samples_expected = 0_usize;
    let mut drift_max = 0.0_f64;
    for (start, end) in &training {
        let block = source.read_block(*start, *end).with_context(|| {
            format!("noise diagnostics cannot read training block [{start}, {end})")
        })?;
        for value in block.detector.iter() {
            if !value.is_finite() {
                bail!("noise diagnostics rejects non-finite detector samples");
            }
        }
        for time in block.times.iter() {
            if !time.is_finite() {
                bail!("noise diagnostics rejects non-finite time samples");
            }
        }
        let drift = timebase_drift(&block.times, sample_interval_s);
        if drift > drift_max {
            drift_max = drift;
        }
        samples_expected += (end - start) as usize;
        block_times.push(block.times);
        detector_blocks.push(block.detector);
    }
    let samples_read: usize = detector_blocks.iter().map(Vec::len).sum();
    let tolerances = JointSolverTolerances::default();
    let mut residuals: Vec<Vec<f64>> = Vec::with_capacity(detector_blocks.len());
    let mut ranks = Vec::with_capacity(detector_blocks.len());
    let mut conditions = Vec::with_capacity(detector_blocks.len());
    let mut residual_rms = Vec::with_capacity(detector_blocks.len());
    let mut rank_deficient = Vec::new();
    let mut ill_conditioned = Vec::new();
    for (index, (times, signal)) in block_times.iter().zip(detector_blocks.iter()).enumerate() {
        let fit = fit_nuisance(
            times,
            signal,
            reference_frequency_hz,
            reference_phase_rad,
            sample_rate_hz,
            tolerances,
        )
        .with_context(|| format!("noise nuisance fit fails on training block {index}"))?;
        ranks.push(fit.rank);
        conditions.push(fit.condition);
        if fit.rank < NUISANCE_PARAMETERS {
            rank_deficient.push(index);
        }
        if fit.condition > tolerances.max_condition {
            ill_conditioned.push(index);
        }
        residual_rms.push(rms(&fit.residual));
        residuals.push(fit.residual);
    }
    let samples = assemble_samples(
        &block_times,
        &residuals,
        reference_frequency_hz,
        reference_phase_rad,
    )?;
    let phase_recipe = PhaseVarianceRecipe::default();
    let phase: PhaseVarianceOutput = estimate_phase_variance(&samples, phase_recipe)?;
    let correlation_recipe = CorrelationRecipe::default();
    let correlation: CorrelationOutput =
        estimate_correlation(&residuals, None, sample_interval_s, correlation_recipe)?;
    let peak_to_median = peak_to_median_ratio(&phase.variances);
    let max_abs_offpeak = max_abs_offpeak(&correlation.lags);
    let acquisition_qc = AcquisitionQc {
        blocks_read: training.len(),
        samples_read,
        samples_expected,
        non_finite_rejected: false,
        timebase_drift_max: drift_max,
        timebase_ok: drift_max <= 1e-9,
        channel_parity_ok: samples_read == samples_expected,
    };
    let nuisance_observability = NuisanceObservability {
        recipe_id: NUISANCE_RECIPE_ID.to_string(),
        blocks_fitted: training.len(),
        ranks,
        conditions,
        residual_rms,
        rank_deficient_blocks: rank_deficient,
        ill_conditioned_blocks: ill_conditioned,
    };
    let phase_section = PhaseVarianceSection {
        recipe_id: PHASE_VARIANCE_RECIPE_ID.to_string(),
        bins: phase.variances.len(),
        v0: phase.v0,
        variances: phase.variances,
        counts: phase.counts,
        distinct_cycles: phase.distinct_cycles,
        contributing_blocks: phase.contributing_blocks,
        floor_activations: phase.floor_activations,
        peak_to_median_ratio: peak_to_median,
    };
    let correlation_section = CorrelationSection {
        recipe_id: CORRELATION_RECIPE_ID.to_string(),
        lags: correlation.lags,
        lag_step_s: correlation.lag_step_s,
        support_samples: correlation.support_samples,
        physical_duration_s: correlation.physical_duration_s,
        taper_id: correlation.taper_id,
        eta: correlation.eta,
        spd_validated: correlation.spd_validated,
        tail_energy: correlation.tail_energy,
        max_tail_lag: correlation.max_tail_lag,
        max_abs_offpeak,
    };
    let (verdict, reasons, non_identifiable) = classify_identifiability(
        &phase_section,
        &correlation_section,
        &nuisance_observability,
        &acquisition_qc,
    );
    let diagnostics = SignalDiagnostics {
        acquisition_qc,
        nuisance_observability,
        phase_variance: phase_section,
        correlation: correlation_section,
        identifiability: IdentifiabilitySection {
            verdict: verdict.clone(),
            reasons: reasons.clone(),
            non_identifiable,
        },
        change_qualification: change_qualification(),
    };
    let reason = reasons.join("; ");
    Ok((diagnostics, verdict, reason))
}

/// Training blocks in original-index order from the resolved plan.
fn training_blocks(resolved: &ResolvedPlan) -> Result<Vec<(u64, u64)>> {
    let mut blocks: Vec<(u64, u64)> = resolved
        .role_plan
        .blocks
        .iter()
        .filter(|block| block.role == pmoke_analysis_core::calibration::CalibrationRole::Training)
        .map(|block| (block.start, block.end))
        .collect();
    if blocks.is_empty() {
        bail!("noise diagnostics need at least one resolved training block");
    }
    blocks.sort_unstable();
    Ok(blocks)
}

/// Maximum relative step drift of a decoded block timebase against the
/// declared sample interval.
fn timebase_drift(times: &[f64], sample_interval_s: f64) -> f64 {
    let mut worst = 0.0;
    for pair in times.windows(2) {
        let step = pair[1] - pair[0];
        let drift = ((step - sample_interval_s) / sample_interval_s).abs();
        if drift.is_finite() && drift > worst {
            worst = drift;
        }
    }
    worst
}

fn rms(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    (values
        .iter()
        .map(|value| (value - mean) * (value - mean))
        .sum::<f64>()
        / values.len() as f64)
        .sqrt()
}

fn peak_to_median_ratio(variances: &[f64]) -> f64 {
    if variances.is_empty() {
        return 1.0;
    }
    let mut sorted = variances.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median = sorted[sorted.len() / 2];
    let peak = sorted[sorted.len() - 1];
    if !(median.is_finite() && median > 0.0) || !peak.is_finite() {
        return 1.0;
    }
    peak / median
}

fn max_abs_offpeak(lags: &[f64]) -> f64 {
    lags.iter()
        .skip(1)
        .map(|value| value.abs())
        .fold(0.0_f64, f64::max)
}

/// Mechanism-agnostic classification (PN-FR-011/012/013/035). Thresholds are
/// the calibrated change-rule constants (PN-M4); the rule is
/// mechanism-agnostic — a peaked phase-variance profile or a strong off-peak
/// autocorrelation is `supported_structure`; a flat profile with weak
/// correlation is `no_detectable_change`; anything that fails
/// coverage/condition gates is `insufficient_evidence` (non-identifiable);
/// the remainder abstains as `unknown`. No physical noise cause is asserted.
fn classify_identifiability(
    phase: &PhaseVarianceSection,
    correlation: &CorrelationSection,
    nuisance: &NuisanceObservability,
    qc: &AcquisitionQc,
) -> (String, Vec<String>, bool) {
    let mut reasons = Vec::new();
    if !qc.channel_parity_ok {
        reasons.push(format!(
            "channel parity fails: read {} of {} expected samples",
            qc.samples_read, qc.samples_expected
        ));
        return ("insufficient_evidence".to_string(), reasons, true);
    }
    if !qc.timebase_ok {
        reasons.push(format!(
            "timebase drift {:.3e} exceeds 1e-9 relative tolerance",
            qc.timebase_drift_max
        ));
        return ("insufficient_evidence".to_string(), reasons, true);
    }
    if !nuisance.rank_deficient_blocks.is_empty() {
        reasons.push(format!(
            "rank-deficient nuisance fits on blocks {:?}; residuals under-constrained",
            nuisance.rank_deficient_blocks
        ));
        return ("insufficient_evidence".to_string(), reasons, true);
    }
    if !nuisance.ill_conditioned_blocks.is_empty() {
        reasons.push(format!(
            "ill-conditioned nuisance fits on blocks {:?}; phase variance not identifiable",
            nuisance.ill_conditioned_blocks
        ));
        return ("insufficient_evidence".to_string(), reasons, true);
    }
    if !phase.floor_activations.is_empty() {
        reasons.push(format!(
            "variance floor active on {} bins; shape evidence clipped",
            phase.floor_activations.len()
        ));
        return ("insufficient_evidence".to_string(), reasons, true);
    }
    let decision = change_decision(phase.peak_to_median_ratio, correlation.max_abs_offpeak);
    match decision {
        ChangeDecision::SupportedStructure => {
            let peaked = phase.peak_to_median_ratio >= CHANGE_PHASE_PEAK_MEDIAN_THRESHOLD;
            let correlated = correlation.max_abs_offpeak >= CHANGE_CORRELATION_OFFPEAK_THRESHOLD;
            if peaked {
                reasons.push(format!(
                    "phase variance peaks at {:.2}x median over {} bins (change rule `{rule}`; descriptive shape only)",
                    phase.peak_to_median_ratio,
                    phase.bins,
                    rule = CHANGE_RULE_ID
                ));
            }
            if correlated {
                reasons.push(format!(
                    "residual autocorrelation reaches {:.3} off-peak (change rule `{rule}`; descriptive shape only)",
                    correlation.max_abs_offpeak,
                    rule = CHANGE_RULE_ID
                ));
            }
            reasons.push("no physical noise cause is asserted".to_string());
            ("supported_structure".to_string(), reasons, false)
        }
        ChangeDecision::NoDetectableChange => {
            reasons.push(format!(
                "phase variance flat ({:.2}x median) with weak off-peak correlation ({:.3}) under change rule `{rule}`",
                phase.peak_to_median_ratio,
                correlation.max_abs_offpeak,
                rule = CHANGE_RULE_ID
            ));
            ("no_detectable_change".to_string(), reasons, false)
        }
        ChangeDecision::Unknown => {
            reasons.push(format!(
                "statistics sit between the change-rule `{rule}` bands ({:.2}x median, {:.3} off-peak); no structure can be claimed",
                phase.peak_to_median_ratio,
                correlation.max_abs_offpeak,
                rule = CHANGE_RULE_ID
            ));
            ("unknown".to_string(), reasons, true)
        }
    }
}

/// Recipe constants surfaced for report provenance.
#[allow(dead_code)]
pub fn recipe_provenance() -> Vec<(String, String)> {
    vec![
        (
            "nuisance".to_string(),
            format!("{NUISANCE_RECIPE_ID} bins=n/a harmonics=12 alpha=n/a floor=n/a eta=n/a"),
        ),
        (
            "phase_variance".to_string(),
            format!(
                "{PHASE_VARIANCE_RECIPE_ID} bins=64 min_samples={DEFAULT_MIN_SAMPLES_PER_BIN} min_cycles={DEFAULT_MIN_CYCLES_PER_BIN} alpha={DEFAULT_SHRINKAGE_ALPHA} floor={DEFAULT_FLOOR_RATIO}"
            ),
        ),
        (
            "correlation".to_string(),
            format!(
                "{CORRELATION_RECIPE_ID} max_lag={DEFAULT_MAX_LAG} eta={DEFAULT_CORRELATION_ETA}"
            ),
        ),
    ]
}
