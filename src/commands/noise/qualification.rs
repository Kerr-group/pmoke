//! PN-M4 registered synthetic benchmark for the calibrated change rule
//! (PN-FR-013, PN-AT-008, PN-A-006).
//!
//! The benchmark generates independent synthetic records with prespecified
//! residual covariances ("families"), runs the real estimator kernels
//! (`fit_nuisance` -> `estimate_phase_variance` / `estimate_correlation`) and
//! the product decision rule (`diagnostics::change_decision_with`), and
//! reports false-selection error and detection power with Wilson binomial
//! intervals over fresh seeds.
//!
//! Two disjoint seed stages:
//!
//! 1. *calibration*: candidate thresholds are selected from a prespecified
//!    grid so that the no-change control families keep a false-selection rate
//!    at or below [`CALIBRATION_FP_MARGIN`];
//! 2. *evaluation*: the frozen product thresholds are verified on fresh seeds
//!    against the PN-A-006 targets ([`QUALIFICATION_TARGET_NULL_FP_UPPER`]
//!    and [`QUALIFICATION_TARGET_POWER_LOWER`]).
//!
//! Reported scope: the decision rule over the accepted 64-bin phase-variance
//! recipe and a bounded 64-lag correlation support. The benchmark injects
//! known residual covariance; recorded-source decode and the product's
//! default coverage minimums are not part of this registration.
//!
//! No private data is used: every dataset is generated inline from the
//! documented seed scheme and RNG below.

use pmoke_analysis_core::calibration::{
    CorrelationRecipe, DEFAULT_CORRELATION_ETA, DEFAULT_FLOOR_RATIO, DEFAULT_SHRINKAGE_ALPHA,
    NUISANCE_PARAMETERS, PhaseVarianceRecipe, assemble_samples, estimate_correlation,
    estimate_phase_variance, fit_nuisance,
};
use pmoke_analysis_core::joint::JointSolverTolerances;
use serde::Serialize;

use super::diagnostics::{
    CHANGE_BENCHMARK_DATASETS_PER_FAMILY, CHANGE_BENCHMARK_ID, CHANGE_MEASURED_NULL_FP_UPPER_BOUND,
    CHANGE_MEASURED_POWER_LOWER_BOUND, CHANGE_NO_CHANGE_SPECIFICITY_TARGET,
    CHANGE_NULL_FP_TARGET_UPPER, CHANGE_POWER_TARGET_LOWER, ChangeDecision, ChangeThresholds,
    change_decision_with,
};

/// RNG identity recorded with every qualification report (PN-NFR-006).
pub(crate) const QUALIFICATION_RNG_ID: &str = "xorshift64/v1";
/// Confidence level for every reported binomial interval.
pub(crate) const QUALIFICATION_CONFIDENCE: f64 = 0.95;
/// Blocks per synthetic dataset.
pub(crate) const QUALIFICATION_BLOCKS: usize = 8;
/// Samples per synthetic block (40 cycles at 128 samples per reference cycle).
pub(crate) const QUALIFICATION_BLOCK_SAMPLES: usize = 5_120;
pub(crate) const QUALIFICATION_SAMPLE_INTERVAL_S: f64 = 1.0e-5;
/// Reference frequency chosen so each 64-bin phase bin holds exactly two
/// samples per cycle (781.25 Hz at 1e-5 s is 128 samples per cycle).
pub(crate) const QUALIFICATION_REFERENCE_HZ: f64 = 781.25;
pub(crate) const QUALIFICATION_HARMONICS: [f64; 3] = [1.0, 0.2, 0.05];
pub(crate) const QUALIFICATION_MAX_LAG: usize = 64;
pub(crate) const QUALIFICATION_BINS: usize = 64;
/// Calibration-stage size (threshold selection only; not the published gate).
pub(crate) const QUALIFICATION_CALIBRATION_DATASETS: usize = 120;
/// Published evaluation size (PN-A-006: >= 200 datasets per family).
pub(crate) const QUALIFICATION_EVALUATION_DATASETS: usize = CHANGE_BENCHMARK_DATASETS_PER_FAMILY;
pub(crate) const QUALIFICATION_TARGET_NULL_FP_UPPER: f64 = CHANGE_NULL_FP_TARGET_UPPER;
pub(crate) const QUALIFICATION_TARGET_POWER_LOWER: f64 = CHANGE_POWER_TARGET_LOWER;
/// A candidate threshold is calibrated only when the worst no-change family
/// keeps its calibration-stage false-selection rate at or below this margin.
pub(crate) const CALIBRATION_FP_MARGIN: f64 = 0.025;
/// Calibrating quantile of the no-change control families' statistics that
/// defines the no-change bands (worst case across the control families). The
/// 97.5% quantile leaves headroom above the [`CHANGE_NO_CHANGE_SPECIFICITY_TARGET`]
/// 95% lower bound measured on fresh evaluation seeds.
pub(crate) const NO_CHANGE_BAND_PERCENTILE: f64 = 0.975;
/// Prespecified candidate threshold grids (frozen before measurement).
pub(crate) const PHASE_THRESHOLD_GRID: [f64; 11] = [
    1.10, 1.125, 1.15, 1.175, 1.20, 1.225, 1.25, 1.275, 1.30, 1.325, 1.35,
];
pub(crate) const CORRELATION_THRESHOLD_GRID: [f64; 12] = [
    0.06, 0.08, 0.10, 0.12, 0.14, 0.16, 0.18, 0.20, 0.22, 0.24, 0.26, 0.30,
];

/// Synthetic families with their prespecified residual covariance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ChangeFamily {
    /// Stationary iid residual: no-change control.
    NullStationaryIid,
    /// Scalar-only variance change (x4 after half the record): negative
    /// control. A pure scale change must not promise changed GLS weights.
    ScalarVarianceChange,
    /// Phase-dependent variance with max/min contrast 2: alternative.
    PhaseVarianceContrast2,
    /// Lag-1 autocorrelation 0.2 (truth level): alternative.
    Lag1Change,
    /// Anti-correlated shape at equal total RMS (lag-1 -0.25): alternative.
    CorrelationShapeEqualRms,
}

impl ChangeFamily {
    pub(crate) const ALL: [ChangeFamily; 5] = [
        ChangeFamily::NullStationaryIid,
        ChangeFamily::ScalarVarianceChange,
        ChangeFamily::PhaseVarianceContrast2,
        ChangeFamily::Lag1Change,
        ChangeFamily::CorrelationShapeEqualRms,
    ];

    pub(crate) fn label(self) -> &'static str {
        match self {
            ChangeFamily::NullStationaryIid => "null_stationary_iid",
            ChangeFamily::ScalarVarianceChange => "scalar_variance_change",
            ChangeFamily::PhaseVarianceContrast2 => "phase_variance_contrast_2",
            ChangeFamily::Lag1Change => "lag1_change_0p2",
            ChangeFamily::CorrelationShapeEqualRms => "correlation_shape_equal_rms",
        }
    }

    /// `true` for families where the rule is expected to select structure.
    pub(crate) fn expects_change(self) -> bool {
        matches!(
            self,
            ChangeFamily::PhaseVarianceContrast2
                | ChangeFamily::Lag1Change
                | ChangeFamily::CorrelationShapeEqualRms
        )
    }
}

/// Disjoint seed stages (PN-A-006: fresh seeds for the published evaluation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QualificationStage {
    Calibration,
    Evaluation,
}

const SEED_ROOT: u64 = 0x504E_4D34_0B3E_1F7A;

/// Deterministic, documented seed scheme: disjoint across stage, family and
/// dataset index, with a splitmix64 finalizer.
pub(crate) fn dataset_seed(stage: QualificationStage, family: ChangeFamily, index: usize) -> u64 {
    let stage_tag: u64 = match stage {
        QualificationStage::Calibration => 0x0C0F_FEE0_D000_0001,
        QualificationStage::Evaluation => 0x0E0A_1000_0000_0002,
    };
    let mut value = SEED_ROOT
        .wrapping_add(stage_tag)
        .wrapping_add((family as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add((index as u64).wrapping_mul(0xBF58_476D_1CE4_E5B9));
    value ^= value >> 30;
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^= value >> 31;
    value.max(1)
}

struct XorShift64(u64);

impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9E37_79B9_7F4A_7C15
        } else {
            seed
        })
    }

    fn next_u64(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn next_signed(&mut self) -> f64 {
        let unit = (self.next_u64() >> 11) as f64 / ((1_u64 << 53) as f64);
        2.0 * unit - 1.0
    }

    /// Zero-mean noise with unit variance (sum of three uniform draws).
    fn unit_variance(&mut self) -> f64 {
        self.next_signed() + self.next_signed() + self.next_signed()
    }
}

/// One module-generated dataset: a deterministic carrier plus the family's
/// residual process.
pub(crate) fn synthesize_record(family: ChangeFamily, seed: u64) -> Vec<f64> {
    let total = QUALIFICATION_BLOCKS * QUALIFICATION_BLOCK_SAMPLES;
    let dt = QUALIFICATION_SAMPLE_INTERVAL_S;
    let reference = QUALIFICATION_REFERENCE_HZ;
    let mut rng = XorShift64::new(seed);
    let mut previous = 0.0_f64;
    let mut record = Vec::with_capacity(total);
    for index in 0..total {
        let phase = std::f64::consts::TAU * reference * index as f64 * dt;
        let carrier: f64 = QUALIFICATION_HARMONICS
            .iter()
            .enumerate()
            .map(|(offset, amplitude)| {
                amplitude * (phase * (offset + 1) as f64 + 0.3 - 0.2 * offset as f64).sin()
            })
            .sum();
        let noise = match family {
            ChangeFamily::NullStationaryIid => rng.unit_variance(),
            ChangeFamily::ScalarVarianceChange => {
                let scale = if index * 2 < total { 1.0 } else { 4.0 };
                scale * rng.unit_variance()
            }
            ChangeFamily::PhaseVarianceContrast2 => {
                // v(phi) = 1 + cos(phi)/3 -> max/min contrast 2.
                (1.0 + phase.cos() / 3.0).max(0.0).sqrt() * rng.unit_variance()
            }
            ChangeFamily::Lag1Change => {
                let phi1 = 0.2_f64;
                let value = phi1 * previous + (1.0 - phi1 * phi1).sqrt() * rng.unit_variance();
                previous = value;
                value
            }
            ChangeFamily::CorrelationShapeEqualRms => {
                let phi1 = -0.25_f64;
                let value = phi1 * previous + (1.0 - phi1 * phi1).sqrt() * rng.unit_variance();
                previous = value;
                value
            }
        };
        record.push(carrier + noise);
    }
    record
}

/// Statistics observed on one dataset by the registered decision rule.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DatasetStats {
    pub phase_peak_median: f64,
    pub max_abs_offpeak: f64,
}

fn peak_to_median(variances: &[f64]) -> f64 {
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

/// Runs the real estimator chain on one synthetic dataset and returns the two
/// scalars the change rule consumes.
pub(crate) fn dataset_statistics(family: ChangeFamily, seed: u64) -> Result<DatasetStats, String> {
    let record = synthesize_record(family, seed);
    let dt = QUALIFICATION_SAMPLE_INTERVAL_S;
    let sample_rate_hz = 1.0 / dt;
    let reference = QUALIFICATION_REFERENCE_HZ;
    let block_samples = QUALIFICATION_BLOCK_SAMPLES;
    let mut block_times: Vec<Vec<f64>> = Vec::with_capacity(QUALIFICATION_BLOCKS);
    let mut residuals: Vec<Vec<f64>> = Vec::with_capacity(QUALIFICATION_BLOCKS);
    for block in 0..QUALIFICATION_BLOCKS {
        let start = block * block_samples;
        let end = start + block_samples;
        let times: Vec<f64> = (start..end).map(|index| index as f64 * dt).collect();
        let fit = fit_nuisance(
            &times,
            &record[start..end],
            reference,
            0.0,
            sample_rate_hz,
            JointSolverTolerances::default(),
        )
        .map_err(|error| format!("nuisance fit: {error}"))?;
        if fit.rank != NUISANCE_PARAMETERS {
            return Err(format!(
                "nuisance rank {} below the full-rank requirement {NUISANCE_PARAMETERS}",
                fit.rank
            ));
        }
        block_times.push(times);
        residuals.push(fit.residual);
    }
    let samples = assemble_samples(&block_times, &residuals, reference, 0.0)
        .map_err(|error| format!("sample assembly: {error}"))?;
    let recipe = PhaseVarianceRecipe {
        bins: QUALIFICATION_BINS,
        min_samples_per_bin: 8,
        min_cycles_per_bin: 2,
        min_contributing_blocks: QUALIFICATION_BLOCKS,
        shrinkage_alpha: DEFAULT_SHRINKAGE_ALPHA,
        floor_ratio: DEFAULT_FLOOR_RATIO,
    };
    let phase = estimate_phase_variance(&samples, recipe)
        .map_err(|error| format!("phase variance: {error}"))?;
    let correlation = estimate_correlation(
        &residuals,
        None,
        dt,
        CorrelationRecipe {
            max_lag: QUALIFICATION_MAX_LAG,
            shrinkage_eta: DEFAULT_CORRELATION_ETA,
        },
    )
    .map_err(|error| format!("correlation: {error}"))?;
    Ok(DatasetStats {
        phase_peak_median: peak_to_median(&phase.variances),
        max_abs_offpeak: max_abs_offpeak(&correlation.lags),
    })
}

/// Two-sided normal quantile for the confidence level (Acklam's inverse
/// normal CDF approximation; |error| < 1.15e-9).
fn two_sided_z(confidence: f64) -> f64 {
    let p = (1.0 + confidence) / 2.0;
    if !(0.0 < p && p < 1.0) {
        return 1.959_963_984_540_054;
    }
    const A: [f64; 6] = [
        -3.969_683_028_665_376e1,
        2.209_460_984_245_205e2,
        -2.759_285_104_469_687e2,
        1.383_577_518_672_69e2,
        -3.066_479_806_614_716e1,
        2.506_628_277_459_239,
    ];
    const B: [f64; 5] = [
        -5.447_609_879_822_406e1,
        1.615_858_368_580_409e2,
        -1.556_989_798_598_866e2,
        6.680_131_188_771_972e1,
        -1.328_068_155_288_572e1,
    ];
    const C: [f64; 6] = [
        -7.784_894_002_430_293e-3,
        -3.223_964_580_411_365e-1,
        -2.400_758_277_161_838,
        -2.549_732_539_343_734,
        4.374_664_141_464_968,
        2.938_163_982_698_783,
    ];
    const D: [f64; 4] = [
        7.784_695_709_041_462e-3,
        3.224_671_290_700_398e-1,
        2.445_134_137_142_996,
        3.754_408_661_907_416,
    ];
    const LOW: f64 = 0.02425;
    const HIGH: f64 = 1.0 - LOW;
    if p < LOW {
        let q = (-2.0 * p.ln()).sqrt();
        (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    } else if p <= HIGH {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q
            / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5])
            / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0)
    }
}

/// Wilson score interval for a binomial rate.
pub(crate) fn wilson_interval(successes: usize, total: usize, confidence: f64) -> (f64, f64) {
    if total == 0 {
        return (0.0, 1.0);
    }
    let n = total as f64;
    let rate = successes as f64 / n;
    let z = two_sided_z(confidence);
    let denominator = 1.0 + z * z / n;
    let center = (rate + z * z / (2.0 * n)) / denominator;
    let spread = z * (rate * (1.0 - rate) / n + z * z / (4.0 * n * n)).sqrt() / denominator;
    ((center - spread).max(0.0), (center + spread).min(1.0))
}

/// Aggregated outcome for one family within a stage.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct FamilyOutcome {
    pub family: String,
    pub expectation: String,
    pub datasets: usize,
    pub uncomputable: usize,
    pub uncomputable_reason: Option<String>,
    pub selected: usize,
    pub no_detectable_change: usize,
    pub abstained: usize,
    pub rate: f64,
    pub interval_low: f64,
    pub interval_high: f64,
    pub no_change_rate: f64,
    pub no_change_interval_low: f64,
    pub no_change_interval_high: f64,
    pub specificity_target: f64,
    pub specificity_status: String,
    pub target: f64,
    pub target_kind: String,
    pub status: String,
    pub median_phase_peak_median: f64,
    pub median_max_abs_offpeak: f64,
}

fn median(values: &mut [f64]) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

fn aggregate(
    family: ChangeFamily,
    stats: &[Result<DatasetStats, String>],
    thresholds: ChangeThresholds,
) -> FamilyOutcome {
    let mut selected = 0_usize;
    let mut no_change = 0_usize;
    let mut abstained = 0_usize;
    let mut uncomputable = 0_usize;
    let mut uncomputable_reason: Option<String> = None;
    let mut phase_values: Vec<f64> = Vec::with_capacity(stats.len());
    let mut correlation_values: Vec<f64> = Vec::with_capacity(stats.len());
    for entry in stats {
        match entry {
            Ok(value) => {
                phase_values.push(value.phase_peak_median);
                correlation_values.push(value.max_abs_offpeak);
                match change_decision_with(
                    thresholds,
                    value.phase_peak_median,
                    value.max_abs_offpeak,
                ) {
                    ChangeDecision::SupportedStructure => selected += 1,
                    ChangeDecision::NoDetectableChange => no_change += 1,
                    ChangeDecision::Unknown => abstained += 1,
                }
            }
            Err(reason) => {
                uncomputable += 1;
                if uncomputable_reason.is_none() {
                    uncomputable_reason = Some(reason.clone());
                }
            }
        }
    }
    let datasets = stats.len();
    let rate = if datasets == 0 {
        0.0
    } else {
        selected as f64 / datasets as f64
    };
    let (interval_low, interval_high) =
        wilson_interval(selected, datasets, QUALIFICATION_CONFIDENCE);
    let no_change_rate = if datasets == 0 {
        0.0
    } else {
        no_change as f64 / datasets as f64
    };
    let (no_change_interval_low, no_change_interval_high) =
        wilson_interval(no_change, datasets, QUALIFICATION_CONFIDENCE);
    let (target, target_kind) = if family.expects_change() {
        (QUALIFICATION_TARGET_POWER_LOWER, "power_lower_bound")
    } else {
        (
            QUALIFICATION_TARGET_NULL_FP_UPPER,
            "false_selection_upper_bound",
        )
    };
    let status = if family.expects_change() {
        if interval_low >= target {
            "met"
        } else {
            "failed"
        }
    } else if interval_high <= target {
        "met"
    } else {
        "failed"
    };
    // The specificity check keeps `no_detectable_change` from degenerating
    // into a rare verdict for the calibrated no-change families.
    let specificity_status = if family.expects_change() {
        "not_applicable".to_string()
    } else if no_change_interval_low >= CHANGE_NO_CHANGE_SPECIFICITY_TARGET {
        "met".to_string()
    } else {
        "failed".to_string()
    };
    FamilyOutcome {
        family: family.label().to_string(),
        expectation: if family.expects_change() {
            "change"
        } else {
            "no_change"
        }
        .to_string(),
        datasets,
        uncomputable,
        uncomputable_reason,
        selected,
        no_detectable_change: no_change,
        abstained,
        rate,
        interval_low,
        interval_high,
        no_change_rate,
        no_change_interval_low,
        no_change_interval_high,
        specificity_target: CHANGE_NO_CHANGE_SPECIFICITY_TARGET,
        specificity_status,
        target,
        target_kind: target_kind.to_string(),
        status: status.to_string(),
        median_phase_peak_median: median(&mut phase_values),
        median_max_abs_offpeak: median(&mut correlation_values),
    }
}

/// Nearest-rank percentile of calibration statistics; the deterministic rule
/// behind the frozen no-change bands.
fn percentile(values: &mut [f64], quantile: f64) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    values.sort_by(f64::total_cmp);
    let rank = (quantile * (values.len() - 1) as f64).round() as usize;
    values[rank]
}

/// Calibrates the whole threshold set: detection thresholds from the
/// prespecified grids under [`CALIBRATION_FP_MARGIN`], no-change bands as the
/// worst-case per-family [`NO_CHANGE_BAND_PERCENTILE`] of the calibration
/// no-change control families (so every control family keeps at least that
/// share of reportable `no_detectable_change` datasets).
pub(crate) fn calibrate_thresholds(
    calibration: &[(ChangeFamily, Result<DatasetStats, String>)],
) -> ChangeThresholds {
    let false_rate = |phase_threshold: f64, correlation_threshold: f64| -> f64 {
        let mut worst = 0.0_f64;
        for family in ChangeFamily::ALL {
            if family.expects_change() {
                continue;
            }
            let total = calibration
                .iter()
                .filter(|(entry_family, _)| *entry_family == family)
                .count();
            if total == 0 {
                continue;
            }
            let selected = calibration
                .iter()
                .filter(|(entry_family, _)| *entry_family == family)
                .filter(|(_, stats)| match stats {
                    Ok(value) => {
                        change_decision_with(
                            ChangeThresholds {
                                phase_peak_median: phase_threshold,
                                correlation_offpeak: correlation_threshold,
                                no_change_phase_band: 0.0,
                                no_change_correlation_band: 0.0,
                            },
                            value.phase_peak_median,
                            value.max_abs_offpeak,
                        ) == ChangeDecision::SupportedStructure
                    }
                    Err(_) => false,
                })
                .count();
            worst = worst.max(selected as f64 / total as f64);
        }
        worst
    };
    let phase = PHASE_THRESHOLD_GRID
        .iter()
        .copied()
        .find(|threshold| false_rate(*threshold, f64::INFINITY) <= CALIBRATION_FP_MARGIN)
        .unwrap_or(*PHASE_THRESHOLD_GRID.last().expect("grid"));
    let correlation = CORRELATION_THRESHOLD_GRID
        .iter()
        .copied()
        .find(|threshold| false_rate(f64::INFINITY, *threshold) <= CALIBRATION_FP_MARGIN)
        .unwrap_or(*CORRELATION_THRESHOLD_GRID.last().expect("grid"));
    let mut band_phase = 0.0_f64;
    let mut band_correlation = 0.0_f64;
    for family in ChangeFamily::ALL {
        if family.expects_change() {
            continue;
        }
        let mut phase_values: Vec<f64> = calibration
            .iter()
            .filter(|(entry_family, _)| *entry_family == family)
            .filter_map(|(_, stats)| stats.as_ref().ok().map(|stats| stats.phase_peak_median))
            .collect();
        let mut correlation_values: Vec<f64> = calibration
            .iter()
            .filter(|(entry_family, _)| *entry_family == family)
            .filter_map(|(_, stats)| stats.as_ref().ok().map(|stats| stats.max_abs_offpeak))
            .collect();
        band_phase = band_phase.max(percentile(&mut phase_values, NO_CHANGE_BAND_PERCENTILE));
        band_correlation = band_correlation.max(percentile(
            &mut correlation_values,
            NO_CHANGE_BAND_PERCENTILE,
        ));
    }
    ChangeThresholds {
        phase_peak_median: phase,
        correlation_offpeak: correlation,
        no_change_phase_band: band_phase.min(phase),
        no_change_correlation_band: band_correlation.min(correlation),
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct QualificationConfig {
    pub calibration_datasets: usize,
    pub evaluation_datasets: usize,
}

impl QualificationConfig {
    /// The registered benchmark: >=200 evaluation datasets per family.
    pub(crate) const fn registered() -> Self {
        Self {
            calibration_datasets: QUALIFICATION_CALIBRATION_DATASETS,
            evaluation_datasets: QUALIFICATION_EVALUATION_DATASETS,
        }
    }
}

/// Machine-readable qualification evidence.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct QualificationReport {
    pub schema_version: u32,
    pub benchmark_id: String,
    pub rng_id: String,
    pub confidence_level: f64,
    pub blocks: usize,
    pub block_samples: usize,
    pub samples_per_dataset: usize,
    pub reference_frequency_hz: f64,
    pub sample_interval_s: f64,
    pub max_lag: usize,
    pub phase_bins: usize,
    pub calibration_datasets: usize,
    pub evaluation_datasets: usize,
    pub calibration_fp_margin: f64,
    pub no_change_band_percentile: f64,
    pub selected_phase_threshold: f64,
    pub selected_correlation_threshold: f64,
    pub selected_no_change_phase_band: f64,
    pub selected_no_change_correlation_band: f64,
    pub frozen_phase_threshold: f64,
    pub frozen_correlation_threshold: f64,
    pub frozen_no_change_phase_band: f64,
    pub frozen_no_change_correlation_band: f64,
    pub thresholds_match_frozen: bool,
    pub target_null_fp_upper: f64,
    pub target_power_lower: f64,
    pub families: Vec<FamilyOutcome>,
    pub worst_null_fp_upper_bound: f64,
    pub worst_power_lower_bound: f64,
    pub status: String,
    pub scope: String,
}

/// Runs the two-stage registered benchmark.
pub(crate) fn run_qualification(config: &QualificationConfig) -> QualificationReport {
    let calibration_thresholds = {
        let mut calibration = Vec::new();
        for family in ChangeFamily::ALL {
            for index in 0..config.calibration_datasets {
                let seed = dataset_seed(QualificationStage::Calibration, family, index);
                calibration.push((family, dataset_statistics(family, seed)));
            }
        }
        calibrate_thresholds(&calibration)
    };
    let frozen_thresholds = ChangeThresholds::frozen();
    let (selected_phase, selected_correlation) = (
        calibration_thresholds.phase_peak_median,
        calibration_thresholds.correlation_offpeak,
    );
    // The evaluation stage verifies the frozen product thresholds on fresh
    // seeds; the selection is reported beside it so drift is visible.
    let thresholds = frozen_thresholds;
    let mut families = Vec::with_capacity(ChangeFamily::ALL.len());
    let mut worst_null = 0.0_f64;
    let mut worst_power = 1.0_f64;
    for family in ChangeFamily::ALL {
        let mut stats = Vec::with_capacity(config.evaluation_datasets);
        for index in 0..config.evaluation_datasets {
            let seed = dataset_seed(QualificationStage::Evaluation, family, index);
            stats.push(dataset_statistics(family, seed));
        }
        let outcome = aggregate(family, &stats, thresholds);
        if family.expects_change() {
            worst_power = worst_power.min(outcome.interval_low);
        } else {
            worst_null = worst_null.max(outcome.interval_high);
        }
        families.push(outcome);
    }
    let all_met = families.iter().all(|family| family.status == "met");
    let all_specific = families
        .iter()
        .all(|family| family.specificity_status != "failed");
    QualificationReport {
        schema_version: 1,
        benchmark_id: CHANGE_BENCHMARK_ID.to_string(),
        rng_id: QUALIFICATION_RNG_ID.to_string(),
        confidence_level: QUALIFICATION_CONFIDENCE,
        blocks: QUALIFICATION_BLOCKS,
        block_samples: QUALIFICATION_BLOCK_SAMPLES,
        samples_per_dataset: QUALIFICATION_BLOCKS * QUALIFICATION_BLOCK_SAMPLES,
        reference_frequency_hz: QUALIFICATION_REFERENCE_HZ,
        sample_interval_s: QUALIFICATION_SAMPLE_INTERVAL_S,
        max_lag: QUALIFICATION_MAX_LAG,
        phase_bins: QUALIFICATION_BINS,
        calibration_datasets: config.calibration_datasets,
        evaluation_datasets: config.evaluation_datasets,
        calibration_fp_margin: CALIBRATION_FP_MARGIN,
        no_change_band_percentile: NO_CHANGE_BAND_PERCENTILE,
        selected_phase_threshold: selected_phase,
        selected_correlation_threshold: selected_correlation,
        selected_no_change_phase_band: calibration_thresholds.no_change_phase_band,
        selected_no_change_correlation_band: calibration_thresholds.no_change_correlation_band,
        frozen_phase_threshold: thresholds.phase_peak_median,
        frozen_correlation_threshold: thresholds.correlation_offpeak,
        frozen_no_change_phase_band: thresholds.no_change_phase_band,
        frozen_no_change_correlation_band: thresholds.no_change_correlation_band,
        thresholds_match_frozen: calibration_thresholds == thresholds,
        target_null_fp_upper: QUALIFICATION_TARGET_NULL_FP_UPPER,
        target_power_lower: QUALIFICATION_TARGET_POWER_LOWER,
        families,
        worst_null_fp_upper_bound: worst_null,
        worst_power_lower_bound: worst_power,
        status: if all_met && all_specific {
            "qualified"
        } else {
            "failed"
        }
        .to_string(),
        scope: "decision rule over the accepted 64-bin phase-variance recipe and a 64-lag \
                correlation support on synthetic records with prespecified residual covariance"
            .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn smoke_config() -> QualificationConfig {
        QualificationConfig {
            calibration_datasets: 16,
            evaluation_datasets: 16,
        }
    }

    #[test]
    #[ignore = "reduced-size registered benchmark smoke: run with --release --ignored"]
    fn benchmark_smoke_is_deterministic_and_well_formed() {
        // Two evaluation datasets per family. The estimator chain is slow in
        // debug builds, so this reduced-size run and the full registered run
        // below are explicit `--ignored` evidence runs; the cheap logic tests
        // stay in the default suite.
        let config = QualificationConfig {
            calibration_datasets: 1,
            evaluation_datasets: 2,
        };
        let first = run_qualification(&config);
        let second = run_qualification(&config);
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap(),
            "the registered benchmark must be reproducible"
        );
        assert_eq!(first.families.len(), ChangeFamily::ALL.len());
        for outcome in &first.families {
            assert_eq!(outcome.datasets, 2);
            assert_eq!(outcome.uncomputable, 0);
            assert_eq!(
                outcome.selected
                    + outcome.no_detectable_change
                    + outcome.abstained
                    + outcome.uncomputable,
                outcome.datasets
            );
            if outcome.expectation == "no_change" {
                assert_eq!(
                    outcome.selected, 0,
                    "no-change family `{}` must not select structure",
                    outcome.family
                );
            } else {
                assert!(
                    outcome.median_phase_peak_median
                        >= super::super::diagnostics::CHANGE_PHASE_PEAK_MEDIAN_THRESHOLD
                        || outcome.median_max_abs_offpeak
                            >= super::super::diagnostics::CHANGE_CORRELATION_OFFPEAK_THRESHOLD,
                    "alternative family `{}` sits below the frozen thresholds",
                    outcome.family
                );
                assert_eq!(outcome.specificity_status, "not_applicable");
            }
        }
    }

    #[test]
    fn wilson_interval_and_threshold_selection_are_sane() {
        let (low, high) = wilson_interval(0, 200, 0.95);
        assert_eq!(low, 0.0);
        assert!(high > 0.0 && high < 0.03, "upper bound {high}");
        let (low, high) = wilson_interval(200, 200, 0.95);
        assert!(low > 0.97, "lower bound {low}");
        assert_eq!(high, 1.0);
        let mut calibration = Vec::new();
        for index in 0..60 {
            calibration.push((
                ChangeFamily::NullStationaryIid,
                Ok(DatasetStats {
                    phase_peak_median: 1.05 + 0.01 * (index % 5) as f64,
                    max_abs_offpeak: 0.005 + 0.001 * (index % 3) as f64,
                }),
            ));
            calibration.push((
                ChangeFamily::ScalarVarianceChange,
                Ok(DatasetStats {
                    phase_peak_median: 1.10,
                    max_abs_offpeak: 0.02,
                }),
            ));
            calibration.push((
                ChangeFamily::PhaseVarianceContrast2,
                Ok(DatasetStats {
                    phase_peak_median: 1.33,
                    max_abs_offpeak: 0.01,
                }),
            ));
        }
        let thresholds = calibrate_thresholds(&calibration);
        assert!(thresholds.no_change_phase_band < thresholds.phase_peak_median);
        assert!(thresholds.no_change_correlation_band < thresholds.correlation_offpeak);
        assert_eq!(
            change_decision_with(thresholds, 1.33, 0.01),
            ChangeDecision::SupportedStructure
        );
        assert_eq!(
            change_decision_with(thresholds, 1.05, 0.005),
            ChangeDecision::NoDetectableChange
        );
    }

    #[test]
    #[ignore = "reduced-size benchmark smoke: run with --release --ignored"]
    fn benchmark_smoke_reports_error_and_power() {
        let report = run_qualification(&smoke_config());
        assert_eq!(report.families.len(), ChangeFamily::ALL.len());
        for outcome in &report.families {
            assert_eq!(outcome.datasets, 16);
            if outcome.expectation == "no_change" {
                assert_eq!(outcome.selected, 0);
                assert!(outcome.no_change_rate >= 0.5);
            } else {
                assert!(outcome.selected * 2 >= outcome.datasets);
            }
        }
    }

    #[test]
    fn seed_scheme_is_disjoint_across_stage_family_and_index() {
        let mut seeds = std::collections::BTreeSet::new();
        for stage in [
            QualificationStage::Calibration,
            QualificationStage::Evaluation,
        ] {
            for family in ChangeFamily::ALL {
                for index in 0..200 {
                    assert!(
                        seeds.insert(dataset_seed(stage, family, index)),
                        "seed collision at {stage:?}/{family:?}/{index}"
                    );
                }
            }
        }
    }

    #[test]
    #[ignore = "registered PN-AT-008 benchmark: run with --release --ignored for the evidence run"]
    fn registered_benchmark_reports_error_and_power_on_fresh_seeds() {
        let report = run_qualification(&QualificationConfig::registered());
        println!(
            "PN-AT-008 qualification report:\n{}",
            serde_json::to_string_pretty(&report).unwrap()
        );
        assert_eq!(
            report.evaluation_datasets, QUALIFICATION_EVALUATION_DATASETS,
            "the registered evaluation stage must run >= 200 datasets per family"
        );
        assert!(
            report.thresholds_match_frozen,
            "calibration must reproduce the frozen thresholds (selected {} / {} bands {} / {}, frozen {} / {} bands {} / {})",
            report.selected_phase_threshold,
            report.selected_correlation_threshold,
            report.selected_no_change_phase_band,
            report.selected_no_change_correlation_band,
            report.frozen_phase_threshold,
            report.frozen_correlation_threshold,
            report.frozen_no_change_phase_band,
            report.frozen_no_change_correlation_band
        );
        assert_eq!(report.status, "qualified");
        assert!(report.worst_null_fp_upper_bound <= report.target_null_fp_upper);
        assert!(report.worst_power_lower_bound >= report.target_power_lower);
        for outcome in &report.families {
            if outcome.expectation == "no_change" {
                assert_eq!(
                    outcome.specificity_status, "met",
                    "no-change family `{}` must stay reportable as no_detectable_change",
                    outcome.family
                );
            }
        }
        assert_eq!(
            report.worst_null_fp_upper_bound,
            CHANGE_MEASURED_NULL_FP_UPPER_BOUND
        );
        assert_eq!(
            report.worst_power_lower_bound,
            CHANGE_MEASURED_POWER_LOWER_BOUND
        );
        assert_eq!(
            super::super::diagnostics::CHANGE_QUALIFICATION_STATUS,
            "qualified"
        );
    }
}
