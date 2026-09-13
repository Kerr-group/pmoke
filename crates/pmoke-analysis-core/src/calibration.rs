//! WP-3 recorded-data calibration builder: pure numerical components.
//!
//! Implements the NUMERICS section 6 reference recipe (fixed v1) and the
//! INTERFACES section 4 immutable artifact: explicit role/block planning,
//! per-block nuisance regression, phase-binned variance estimation,
//! tapered/shrunk correlation estimation, artifact construction with byte
//! identification, pure applicability inspection, and the SCS adequacy
//! diagnostic. Native I/O, CLI, and byte-level model loading belong to
//! WP-5; public tests use synthetic calibration only (never private
//! measurement-derived coefficients).
//!
//! V1 implements the fixed recipe only. Any hyperparameter search request is
//! rejected explicitly with `unsupported_autotuning` (NUMERICS 6.5).

use crate::error::{AnalysisError, Result};
use crate::joint::{JointSolverTolerances, solve_direct};
use nalgebra::DMatrix;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::f64::consts::TAU;

// ---------------------------------------------------------------------------
// Recipe identity and limits.
// ---------------------------------------------------------------------------

/// Immutable calibration artifact schema version (INTERFACES section 4).
pub const CALIBRATION_ARTIFACT_SCHEMA_VERSION: u32 = 1;
/// Versioned calibration recipe identifier; any recipe change is a new id.
pub const CALIBRATION_ALGORITHM_VERSION: &str = "pmoke-calibration-v1";
/// Fixed phase convention string shared with the joint solver.
pub const CALIBRATION_PHASE_CONVENTION: &str = "phi=2*pi*f*t-reference_phase_rad";
/// Periodic-linear variance interpolation identifier.
pub const VARIANCE_INTERP_ID: &str = "periodic_linear_variance_v1";
/// Bartlett lag taper identifier (NUMERICS 6.4).
pub const CORRELATION_TAPER_ID: &str = "bartlett_v1";
/// Per-channel artifact byte cap (INTERFACES section 4 proposal).
pub const MAX_CALIBRATION_ARTIFACT_BYTES: usize = 1024 * 1024;
/// Default sample-interval applicability tolerance (relative).
pub const DEFAULT_DT_REL_TOL: f64 = 1e-9;
/// Default reference-frequency applicability tolerance (relative).
pub const DEFAULT_FREQ_REL_TOL: f64 = 1e-9;
/// Default recipe block length in original samples (NUMERICS 6.1 proposal).
pub const DEFAULT_BLOCK_LEN: usize = 262_144;
/// Default minimum reference cycles per block.
pub const DEFAULT_MIN_CYCLES_PER_BLOCK: f64 = 128.0;
/// Default minimum usable training blocks.
pub const DEFAULT_MIN_TRAINING_BLOCKS: usize = 8;
/// Default minimum nominated training intervals contributing blocks.
pub const DEFAULT_MIN_TRAINING_INTERVALS: usize = 2;
/// Default minimum raw samples per phase bin.
pub const DEFAULT_MIN_SAMPLES_PER_BIN: usize = 256;
/// Default minimum distinct cycles per phase bin.
pub const DEFAULT_MIN_CYCLES_PER_BIN: usize = 128;
/// Default phase-variance shrinkage toward the coverage-weighted mean.
pub const DEFAULT_SHRINKAGE_ALPHA: f64 = 0.1;
/// Default variance floor as a ratio of v0.
pub const DEFAULT_FLOOR_RATIO: f64 = 0.05;
/// Default maximum correlation lag proposal (capped by runtime geometry).
pub const DEFAULT_MAX_LAG: usize = 256;
/// Default identity shrinkage on the tapered correlation.
pub const DEFAULT_CORRELATION_ETA: f64 = 0.05;
/// Nuisance harmonics 1..=12 plus DC and a scaled linear trend.
pub const NUISANCE_HARMONICS: usize = 12;
/// Nuisance parameter count: DC + trend + 2 coefficients per harmonic.
pub const NUISANCE_PARAMETERS: usize = 2 + 2 * NUISANCE_HARMONICS;

// ---------------------------------------------------------------------------
// Roles and block planning (NUMERICS 6.1).
// ---------------------------------------------------------------------------

/// Calibration dataset role. Previously inspected shot intervals stay
/// exploratory: they must not be nominated as confirmatory unseen data.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationRole {
    Training,
    TuningValidation,
    Evaluation,
}

/// Half-open original-index interval `[start, end)` nominated for one role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleInterval {
    pub role: CalibrationRole,
    pub start: u64,
    pub end: u64,
}

/// Realized calibration block, wholly inside its nominated role interval.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PlannedBlock {
    pub role: CalibrationRole,
    pub index_in_role: usize,
    pub start: u64,
    pub end: u64,
    pub reference_cycles: f64,
}

/// Realized exclusion: samples or blocks dropped with an explicit reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedExclusion {
    pub start: u64,
    pub end: u64,
    pub reason: String,
}

/// Block planning request over recorded original indices.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockPlanRequest {
    pub intervals: Vec<RoleInterval>,
    pub block_len: usize,
    pub total_samples: u64,
    pub reference_frequency_hz: f64,
    pub sample_interval_s: f64,
    pub min_reference_cycles_per_block: f64,
    pub min_training_blocks: usize,
    pub min_training_intervals: usize,
}

impl Default for BlockPlanRequest {
    fn default() -> Self {
        Self {
            intervals: Vec::new(),
            block_len: DEFAULT_BLOCK_LEN,
            total_samples: 0,
            reference_frequency_hz: 0.0,
            sample_interval_s: 0.0,
            min_reference_cycles_per_block: DEFAULT_MIN_CYCLES_PER_BLOCK,
            min_training_blocks: DEFAULT_MIN_TRAINING_BLOCKS,
            min_training_intervals: DEFAULT_MIN_TRAINING_INTERVALS,
        }
    }
}

/// Planned blocks plus every exclusion. No hidden boundary padding: a block
/// is emitted only when fully inside its nominated interval.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlockPlan {
    pub intervals: Vec<RoleInterval>,
    pub blocks: Vec<PlannedBlock>,
    pub exclusions: Vec<PlannedExclusion>,
}

fn invalid_request(message: impl Into<String>) -> AnalysisError {
    AnalysisError::new("invalid_calibration_request", message.into())
}

/// Positive-finite check that reads well under `clippy::neg_cmp_op_on_partial_ord`.
fn is_positive_finite(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

/// Enumerate whole blocks inside explicit half-open role intervals.
pub fn plan_blocks(request: &BlockPlanRequest) -> Result<BlockPlan> {
    if request.intervals.is_empty() {
        return Err(invalid_request("at least one role interval is required"));
    }
    if request.block_len == 0 {
        return Err(invalid_request("block_len must be positive"));
    }
    if !is_positive_finite(request.reference_frequency_hz) {
        return Err(invalid_request(
            "reference_frequency_hz must be positive finite",
        ));
    }
    if !is_positive_finite(request.sample_interval_s) {
        return Err(invalid_request("sample_interval_s must be positive finite"));
    }
    let mut ordered = request.intervals.clone();
    ordered.sort_by_key(|interval| (interval.start, interval.end));
    for interval in ordered.iter() {
        if interval.end <= interval.start {
            return Err(invalid_request(format!(
                "inverted or empty role interval [{}, {})",
                interval.start, interval.end
            )));
        }
        if interval.end > request.total_samples {
            return Err(invalid_request(format!(
                "role interval [{}, {}) exceeds recorded length {}",
                interval.start, interval.end, request.total_samples
            )));
        }
    }
    for pair in ordered.windows(2) {
        if pair[1].start < pair[0].end {
            return Err(invalid_request(format!(
                "overlapping role intervals [{}, {}) and [{}, {})",
                pair[0].start, pair[0].end, pair[1].start, pair[1].end
            )));
        }
    }
    let block_len = request.block_len as u64;
    let mut plan = BlockPlan {
        intervals: ordered.clone(),
        blocks: Vec::new(),
        exclusions: Vec::new(),
    };
    for interval in ordered.iter() {
        let mut role_index = 0;
        let mut cursor = interval.start;
        while cursor + block_len <= interval.end {
            let cycles =
                block_len as f64 * request.sample_interval_s * request.reference_frequency_hz;
            if cycles < request.min_reference_cycles_per_block {
                plan.exclusions.push(PlannedExclusion {
                    start: cursor,
                    end: cursor + block_len,
                    reason: format!(
                        "only {cycles:.3} reference cycles below minimum {}",
                        request.min_reference_cycles_per_block
                    ),
                });
            } else {
                plan.blocks.push(PlannedBlock {
                    role: interval.role,
                    index_in_role: role_index,
                    start: cursor,
                    end: cursor + block_len,
                    reference_cycles: cycles,
                });
                role_index += 1;
            }
            cursor += block_len;
        }
        if cursor < interval.end {
            plan.exclusions.push(PlannedExclusion {
                start: cursor,
                end: interval.end,
                reason: "trailing samples do not fill a whole block".to_string(),
            });
        }
    }
    let training_blocks: Vec<&PlannedBlock> = plan
        .blocks
        .iter()
        .filter(|block| block.role == CalibrationRole::Training)
        .collect();
    if training_blocks.len() < request.min_training_blocks {
        return Err(AnalysisError::new(
            "insufficient_calibration",
            format!(
                "only {} usable training blocks below minimum {}",
                training_blocks.len(),
                request.min_training_blocks
            ),
        ));
    }
    let mut contributing: Vec<u64> = training_blocks.iter().map(|block| block.start).collect();
    contributing.sort_unstable();
    contributing.dedup();
    // Distinct contributing training intervals, identified by block origin.
    let mut interval_hits = 0;
    for interval in ordered
        .iter()
        .filter(|interval| interval.role == CalibrationRole::Training)
    {
        if plan
            .blocks
            .iter()
            .any(|block| block.start >= interval.start && block.end <= interval.end)
        {
            interval_hits += 1;
        }
    }
    if interval_hits < request.min_training_intervals {
        return Err(AnalysisError::new(
            "insufficient_calibration",
            format!(
                "only {interval_hits} training intervals contribute blocks below minimum {}",
                request.min_training_intervals
            ),
        ));
    }
    Ok(plan)
}

// ---------------------------------------------------------------------------
// Per-block nuisance regression (NUMERICS 6.2).
// ---------------------------------------------------------------------------

/// Nuisance fit of one calibration block: DC, scaled linear trend, and
/// sin/cos harmonics 1..=12. The residual is a noise-estimation input, never
/// independent truth; regression-DOF effects are reported as approximate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NuisanceFit {
    pub coefficients: Vec<f64>,
    pub rank: usize,
    pub condition: f64,
    pub residual: Vec<f64>,
}

/// Nuisance design `[DC, trend, cos(k phi), sin(k phi) ...]` with the trend
/// scaled to [-1, 1] over the block and `phi = 2 pi f t - phase`.
pub fn nuisance_design_matrix(
    times: &[f64],
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
) -> Result<DMatrix<f64>> {
    if times.len() < NUISANCE_PARAMETERS {
        return Err(AnalysisError::new(
            "insufficient_calibration",
            format!(
                "nuisance fit needs at least {NUISANCE_PARAMETERS} samples, got {}",
                times.len()
            ),
        ));
    }
    for time in times.iter() {
        if !time.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                "nuisance fit times must be finite",
            ));
        }
    }
    if !is_positive_finite(reference_frequency_hz) {
        return Err(invalid_request(
            "nuisance fit needs a positive finite reference frequency",
        ));
    }
    if !reference_phase_rad.is_finite() {
        return Err(AnalysisError::new(
            "non_finite_input",
            "nuisance fit phase must be finite",
        ));
    }
    let first = times[0];
    let last = times[times.len() - 1];
    if last <= first {
        return Err(AnalysisError::new(
            "invalid_timebase",
            "nuisance fit needs a strictly increasing timebase",
        ));
    }
    let mid = 0.5 * (first + last);
    let half = 0.5 * (last - first);
    let mut design = DMatrix::zeros(times.len(), NUISANCE_PARAMETERS);
    for (row, time) in times.iter().enumerate() {
        design[(row, 0)] = 1.0;
        design[(row, 1)] = (time - mid) / half;
        let phase = TAU * reference_frequency_hz * time - reference_phase_rad;
        for harmonic in 1..=NUISANCE_HARMONICS {
            let angle = harmonic as f64 * phase;
            design[(row, 2 + 2 * (harmonic - 1))] = angle.cos();
            design[(row, 2 + 2 * (harmonic - 1) + 1)] = angle.sin();
        }
    }
    Ok(design)
}

/// Regress the nuisance model on one block and return fit plus residual.
pub fn fit_nuisance(
    times: &[f64],
    signal: &[f64],
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    tolerances: JointSolverTolerances,
) -> Result<NuisanceFit> {
    if signal.len() != times.len() {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            "nuisance fit needs one response sample per time",
        ));
    }
    for value in signal.iter() {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                "nuisance fit response must be finite",
            ));
        }
    }
    let design = nuisance_design_matrix(times, reference_frequency_hz, reference_phase_rad)?;
    let response = nalgebra::DVector::from_column_slice(signal);
    let (beta, _, rank, condition) = solve_direct(&design, &response, tolerances)?;
    let fitted = &design * &beta;
    let residual: Vec<f64> = signal
        .iter()
        .zip(fitted.iter())
        .map(|(measured, model)| measured - model)
        .collect();
    Ok(NuisanceFit {
        coefficients: beta.iter().copied().collect(),
        rank,
        condition,
        residual,
    })
}

// ---------------------------------------------------------------------------
// Phase variance recipe (NUMERICS 6.3).
// ---------------------------------------------------------------------------

/// One residual sample with its measured phase, reference cycle, and block.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CalSample {
    pub phase_rad: f64,
    pub cycle: i64,
    pub residual: f64,
    pub block: usize,
}

/// Attach phases (`phi = 2 pi f t - phase`, wrapped to [0, 2 pi)) and integer
/// reference cycles (`floor(f t)`) to per-block residuals.
pub fn assemble_samples(
    block_times: &[Vec<f64>],
    block_residuals: &[Vec<f64>],
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
) -> Result<Vec<CalSample>> {
    if block_times.len() != block_residuals.len() {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            "block times and residuals must pair one to one",
        ));
    }
    if !is_positive_finite(reference_frequency_hz) {
        return Err(invalid_request(
            "sample assembly needs a positive finite reference frequency",
        ));
    }
    if !reference_phase_rad.is_finite() {
        return Err(AnalysisError::new(
            "non_finite_input",
            "sample assembly phase must be finite",
        ));
    }
    let mut samples = Vec::new();
    for (block, (times, residuals)) in block_times.iter().zip(block_residuals.iter()).enumerate() {
        if times.len() != residuals.len() {
            return Err(AnalysisError::new(
                "dimension_mismatch",
                format!("block {block} times and residuals differ in length"),
            ));
        }
        for (time, residual) in times.iter().zip(residuals.iter()) {
            if !time.is_finite() || !residual.is_finite() {
                return Err(AnalysisError::new(
                    "non_finite_input",
                    format!("block {block} time and residual must be finite"),
                ));
            }
            let phase = (TAU * reference_frequency_hz * time - reference_phase_rad).rem_euclid(TAU);
            let cycle = (reference_frequency_hz * time).floor() as i64;
            samples.push(CalSample {
                phase_rad: phase,
                cycle,
                residual: *residual,
                block,
            });
        }
    }
    Ok(samples)
}

/// Fixed phase-variance recipe parameters (NUMERICS 6.3).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PhaseVarianceRecipe {
    pub bins: usize,
    pub min_samples_per_bin: usize,
    pub min_cycles_per_bin: usize,
    pub min_contributing_blocks: usize,
    pub shrinkage_alpha: f64,
    pub floor_ratio: f64,
}

impl Default for PhaseVarianceRecipe {
    fn default() -> Self {
        Self {
            bins: 64,
            min_samples_per_bin: DEFAULT_MIN_SAMPLES_PER_BIN,
            min_cycles_per_bin: DEFAULT_MIN_CYCLES_PER_BIN,
            min_contributing_blocks: DEFAULT_MIN_TRAINING_BLOCKS,
            shrinkage_alpha: DEFAULT_SHRINKAGE_ALPHA,
            floor_ratio: DEFAULT_FLOOR_RATIO,
        }
    }
}

/// Phase variance estimate with coverage evidence and recipe diagnostics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseVarianceOutput {
    pub variances: Vec<f64>,
    pub v0: f64,
    pub counts: Vec<usize>,
    pub distinct_cycles: Vec<usize>,
    pub contributing_blocks: usize,
    pub raw: Vec<f64>,
    pub smoothed: Vec<f64>,
    pub floor_activations: Vec<usize>,
}

/// Pooled within-bin variance after mean removal, then one cyclic smoothing
/// pass, shrinkage toward the coverage-weighted mean, and a ratio floor.
/// Missing coverage is an error; bins are never silently filled.
pub fn estimate_phase_variance(
    samples: &[CalSample],
    recipe: PhaseVarianceRecipe,
) -> Result<PhaseVarianceOutput> {
    if recipe.bins < 2 {
        return Err(invalid_request("phase variance needs at least two bins"));
    }
    if !(0.0..=1.0).contains(&recipe.shrinkage_alpha) {
        return Err(invalid_request("shrinkage_alpha must lie in [0, 1]"));
    }
    if !is_positive_finite(recipe.floor_ratio) {
        return Err(invalid_request("floor_ratio must be positive finite"));
    }
    if samples.is_empty() {
        return Err(AnalysisError::new(
            "insufficient_calibration",
            "phase variance needs residual samples",
        ));
    }
    let bins = recipe.bins;
    let mut counts = vec![0_usize; bins];
    let mut sums = vec![0.0_f64; bins];
    let mut square_sums = vec![0.0_f64; bins];
    let mut cycles: Vec<Vec<i64>> = vec![Vec::new(); bins];
    let mut blocks: Vec<usize> = Vec::new();
    for sample in samples.iter() {
        if !sample.phase_rad.is_finite() || !sample.residual.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                "calibration samples must carry finite phase and residual",
            ));
        }
        let mut bin = (sample.phase_rad / TAU * bins as f64).floor() as isize;
        bin = bin.rem_euclid(bins as isize);
        let bin = bin as usize;
        counts[bin] += 1;
        sums[bin] += sample.residual;
        square_sums[bin] += sample.residual * sample.residual;
        cycles[bin].push(sample.cycle);
        blocks.push(sample.block);
    }
    blocks.sort_unstable();
    blocks.dedup();
    if blocks.len() < recipe.min_contributing_blocks {
        return Err(AnalysisError::new(
            "insufficient_calibration",
            format!(
                "only {} contributing blocks below minimum {}",
                blocks.len(),
                recipe.min_contributing_blocks
            ),
        ));
    }
    let mut raw = vec![0.0_f64; bins];
    let mut distinct_cycles = vec![0_usize; bins];
    for bin in 0..bins {
        if counts[bin] < recipe.min_samples_per_bin {
            return Err(AnalysisError::new(
                "insufficient_calibration",
                format!(
                    "bin {bin} holds {} samples below minimum {}",
                    counts[bin], recipe.min_samples_per_bin
                ),
            ));
        }
        let mut sorted = cycles[bin].clone();
        sorted.sort_unstable();
        sorted.dedup();
        distinct_cycles[bin] = sorted.len();
        if sorted.len() < recipe.min_cycles_per_bin {
            return Err(AnalysisError::new(
                "insufficient_calibration",
                format!(
                    "bin {bin} spans {} cycles below minimum {}",
                    sorted.len(),
                    recipe.min_cycles_per_bin
                ),
            ));
        }
        // Pooled variance with denominator count-1 after mean removal.
        let count = counts[bin] as f64;
        let variance = (square_sums[bin] - sums[bin] * sums[bin] / count) / (count - 1.0);
        if !is_positive_finite(variance) {
            return Err(AnalysisError::new(
                "insufficient_calibration",
                format!("bin {bin} raw variance is nonpositive or non-finite"),
            ));
        }
        raw[bin] = variance;
    }
    let total: usize = counts.iter().sum();
    let v0: f64 = counts
        .iter()
        .zip(raw.iter())
        .map(|(count, variance)| *count as f64 * variance)
        .sum::<f64>()
        / total as f64;
    let mut smoothed = vec![0.0_f64; bins];
    for bin in 0..bins {
        let prev = raw[(bin + bins - 1) % bins];
        let next = raw[(bin + 1) % bins];
        smoothed[bin] = (prev + 2.0 * raw[bin] + next) / 4.0;
    }
    let floor = recipe.floor_ratio * v0;
    let mut variances = vec![0.0_f64; bins];
    let mut floor_activations = Vec::new();
    for bin in 0..bins {
        let shrunk = (1.0 - recipe.shrinkage_alpha) * smoothed[bin] + recipe.shrinkage_alpha * v0;
        if shrunk < floor {
            variances[bin] = floor;
            floor_activations.push(bin);
        } else {
            variances[bin] = shrunk;
        }
    }
    if floor_activations.len() * 4 > bins {
        return Err(AnalysisError::new(
            "insufficient_calibration",
            format!(
                "{} of {bins} bins hit the variance floor; inspect instead of smoothing harder",
                floor_activations.len()
            ),
        ));
    }
    Ok(PhaseVarianceOutput {
        variances,
        v0,
        counts,
        distinct_cycles,
        contributing_blocks: blocks.len(),
        raw,
        smoothed,
        floor_activations,
    })
}

// ---------------------------------------------------------------------------
// Correlation recipe (NUMERICS 6.4).
// ---------------------------------------------------------------------------

/// Fixed correlation recipe parameters: Bartlett taper plus identity
/// shrinkage, both declared in the artifact.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CorrelationRecipe {
    pub max_lag: usize,
    pub shrinkage_eta: f64,
}

impl Default for CorrelationRecipe {
    fn default() -> Self {
        Self {
            max_lag: DEFAULT_MAX_LAG,
            shrinkage_eta: DEFAULT_CORRELATION_ETA,
        }
    }
}

/// Estimated normalized correlation with taper/shrinkage/SPD evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrelationOutput {
    pub lags: Vec<f64>,
    pub lag_step_s: f64,
    pub support_samples: usize,
    pub physical_duration_s: f64,
    pub taper_id: String,
    pub eta: f64,
    pub spd_validated: bool,
    /// Strongest raw correlation just beyond the taper support, when the
    /// blocks are long enough to observe it; `None` records the tail as
    /// unrepresented rather than zero.
    pub tail_energy: Option<f64>,
    pub max_tail_lag: usize,
}

/// Standardized-residual lag statistics within blocks (never across block
/// concatenations): per-block DC removal, biased autocovariance over n,
/// pooling by declared weights, lag-zero normalization, Bartlett taper,
/// identity shrinkage, and SPD validation of the lag Toeplitz factor.
#[allow(clippy::too_many_arguments)]
pub fn estimate_correlation(
    blocks: &[Vec<f64>],
    block_weights: Option<&[f64]>,
    lag_step_s: f64,
    recipe: CorrelationRecipe,
) -> Result<CorrelationOutput> {
    if blocks.is_empty() {
        return Err(AnalysisError::new(
            "insufficient_calibration",
            "correlation needs at least one block",
        ));
    }
    if recipe.max_lag == 0 {
        return Err(invalid_request("max_lag must be positive"));
    }
    if !(0.0..=1.0).contains(&recipe.shrinkage_eta) || !recipe.shrinkage_eta.is_finite() {
        return Err(invalid_request("shrinkage_eta must lie in [0, 1]"));
    }
    if !is_positive_finite(lag_step_s) {
        return Err(invalid_request("lag_step_s must be positive finite"));
    }
    let weights: Vec<f64> = match block_weights {
        Some(explicit) => {
            if explicit.len() != blocks.len() {
                return Err(AnalysisError::new(
                    "dimension_mismatch",
                    "block weights must pair one to one with blocks",
                ));
            }
            for weight in explicit.iter() {
                if !weight.is_finite() || *weight < 0.0 {
                    return Err(AnalysisError::new(
                        "non_finite_input",
                        "block weights must be finite non-negative",
                    ));
                }
            }
            let total: f64 = explicit.iter().sum();
            if !is_positive_finite(total) {
                return Err(invalid_request(
                    "block weights must sum to a positive value",
                ));
            }
            explicit.iter().map(|weight| weight / total).collect()
        }
        None => {
            let total: usize = blocks.iter().map(Vec::len).sum();
            if total == 0 {
                return Err(AnalysisError::new(
                    "insufficient_calibration",
                    "correlation blocks must hold samples",
                ));
            }
            blocks
                .iter()
                .map(|block| block.len() as f64 / total as f64)
                .collect()
        }
    };
    let shortest = blocks.iter().map(Vec::len).min().unwrap_or(0);
    if shortest <= recipe.max_lag {
        return Err(AnalysisError::new(
            "insufficient_calibration",
            format!(
                "shortest block holds {shortest} samples, cannot support lag {}",
                recipe.max_lag
            ),
        ));
    }
    let lags = recipe.max_lag + 1;
    let mut pooled = vec![0.0_f64; lags];
    for (block, weight) in blocks.iter().zip(weights.iter()) {
        for value in block.iter() {
            if !value.is_finite() {
                return Err(AnalysisError::new(
                    "non_finite_input",
                    "correlation blocks must be finite",
                ));
            }
        }
        let mean = block.iter().sum::<f64>() / block.len() as f64;
        // Biased autocovariance: numerator divided by n, not n-lag.
        let mut auto = vec![0.0_f64; lags];
        for lag in 0..lags {
            let mut numerator = 0.0;
            for i in 0..(block.len() - lag) {
                numerator += (block[i] - mean) * (block[i + lag] - mean);
            }
            auto[lag] = numerator / block.len() as f64;
        }
        if !is_positive_finite(auto[0]) {
            return Err(AnalysisError::new(
                "insufficient_calibration",
                "correlation block has no lag-zero power",
            ));
        }
        for lag in 0..lags {
            pooled[lag] += weight * auto[lag] / auto[0];
        }
    }
    let mut tapered = vec![0.0_f64; lags];
    for lag in 0..lags {
        let taper = 1.0 - lag as f64 / (recipe.max_lag + 1) as f64;
        let shrunk = (1.0 - recipe.shrinkage_eta) * pooled[lag] * taper
            + recipe.shrinkage_eta * f64::from(lag == 0);
        if !shrunk.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_output",
                "tapered correlation is non-finite",
            ));
        }
        tapered[lag] = shrunk;
    }
    // SPD validation on the finite lag Toeplitz factor.
    let mut toeplitz = DMatrix::zeros(lags, lags);
    for row in 0..lags {
        for column in 0..lags {
            let lag = row.abs_diff(column);
            toeplitz[(row, column)] = tapered[lag];
        }
    }
    if toeplitz.cholesky().is_none() {
        return Err(AnalysisError::new(
            "covariance_not_spd",
            "tapered correlation Toeplitz factor is not positive definite",
        ));
    }
    // Evidence of correlation beyond J stays a reported limitation.
    let tail_lags = (recipe.max_lag + 1)..=(2 * recipe.max_lag).min(shortest - 1);
    let mut tail_energy: Option<f64> = None;
    let mut max_tail_lag = recipe.max_lag;
    for lag in tail_lags {
        let mut pooled_tail = 0.0;
        for (block, weight) in blocks.iter().zip(weights.iter()) {
            let mean = block.iter().sum::<f64>() / block.len() as f64;
            let mut numerator = 0.0;
            let mut zero = 0.0;
            for i in 0..(block.len() - lag) {
                numerator += (block[i] - mean) * (block[i + lag] - mean);
            }
            for value in block.iter() {
                zero += (value - mean) * (value - mean);
            }
            if zero > 0.0 {
                pooled_tail +=
                    weight * (numerator / block.len() as f64) / (zero / block.len() as f64);
            }
        }
        tail_energy = Some(tail_energy.unwrap_or(0.0).max(pooled_tail.abs()));
        max_tail_lag = lag;
    }
    Ok(CorrelationOutput {
        lags: tapered,
        lag_step_s,
        support_samples: recipe.max_lag + 1,
        physical_duration_s: recipe.max_lag as f64 * lag_step_s,
        taper_id: CORRELATION_TAPER_ID.to_string(),
        eta: recipe.shrinkage_eta,
        spd_validated: true,
        tail_energy,
        max_tail_lag,
    })
}

// ---------------------------------------------------------------------------
// Immutable artifact v1 (INTERFACES section 4).
// ---------------------------------------------------------------------------

/// Builder provenance recorded in every artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BuilderInfo {
    pub pmoke_version: String,
    pub backend_versions: BTreeMap<String, String>,
    pub recipe_settings: BTreeMap<String, String>,
}

/// Channel/scale/reference binding checked before any reuse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelBinding {
    pub channel: u32,
    pub voltage_unit: String,
    pub adc_scale_provenance: Option<String>,
    pub sample_interval_s: f64,
    pub recorded_original_dt_s: Option<f64>,
    pub sample_interval_rel_tol: f64,
    pub reference_frequency_hz: f64,
    pub frequency_rel_tol: f64,
    pub phase_convention: String,
    pub acquisition: AcquisitionMeta,
}

/// Gain/bandwidth/scale metadata; `None` is explicitly unknown, never guessed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcquisitionMeta {
    pub device: Option<String>,
    pub gain: Option<f64>,
    pub bandwidth_hz: Option<f64>,
}

/// Phase variance table with its interpolation contract.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhaseTable {
    pub bins: usize,
    pub center_convention: String,
    pub variances_v2: Vec<f64>,
    pub interpolation: String,
}

/// Normalized correlation with taper/shrinkage/SPD evidence.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrelationTable {
    pub lags: Vec<f64>,
    pub lag_step_s: f64,
    pub support_samples: usize,
    pub taper: String,
    pub shrinkage_eta: f64,
    pub spd_validated: bool,
    pub tail_energy: Option<f64>,
    pub max_tail_lag: usize,
}

/// Actual regularization values in application order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegularizationRecord {
    pub smoothing: String,
    pub shrinkage_alpha_v: f64,
    pub variance_floor_ratio: f64,
    pub floor_activations: Vec<usize>,
    pub correlation_taper: String,
    pub correlation_eta: f64,
}

/// Training provenance: digests, realized intervals/blocks/roles, exclusions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TrainingRecord {
    pub source_digests: Vec<String>,
    pub intervals: Vec<RoleInterval>,
    pub blocks: Vec<PlannedBlock>,
    pub exclusions: Vec<PlannedExclusion>,
    pub seed: u64,
    pub per_bin_counts: Vec<usize>,
    pub per_bin_cycles: Vec<usize>,
    pub contributing_blocks: usize,
}

/// Held-out validation evidence; never an SNR-improvement certificate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationRecord {
    pub heldout_blocks: usize,
    pub profile_rmse_v2: Option<f64>,
    pub standardized_lag1: Option<f64>,
    pub dof_treatment: String,
    pub nuisance_params_per_block: usize,
    pub limits: Vec<String>,
}

/// Advertised capabilities and geometry restrictions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapabilityRecord {
    pub modes: Vec<String>,
    pub geometry_restrictions: Vec<String>,
}

/// Immutable calibration artifact v1: data only, unknown-field rejection,
/// identified by the SHA-256 of its exact serialized bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CalibrationArtifact {
    pub schema_version: u32,
    pub algorithm_version: String,
    pub model_id: String,
    pub builder: BuilderInfo,
    pub binding: ModelBinding,
    pub reference_variance_v2: f64,
    pub phase: PhaseTable,
    pub correlation: Option<CorrelationTable>,
    pub regularization: RegularizationRecord,
    pub training: TrainingRecord,
    pub validation: ValidationRecord,
    pub capabilities: CapabilityRecord,
}

/// Fixed-recipe tuning declaration. V1 supports the fixed recipe only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TuningMode {
    Fixed,
    Search(SearchSpace),
}

/// Predeclared hyperparameter search space (train/tune only, never evaluation).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchSpace {
    pub candidates: BTreeMap<String, Vec<f64>>,
    pub seed: u64,
    pub selection_rule: String,
}

/// V1 rejects hyperparameter search explicitly (NUMERICS 6.5).
pub fn ensure_fixed_tuning(mode: &TuningMode) -> Result<()> {
    match mode {
        TuningMode::Fixed => Ok(()),
        TuningMode::Search(_) => Err(AnalysisError::new(
            "unsupported_autotuning",
            "v1 implements the fixed recipe only; hyperparameter search is rejected",
        )),
    }
}

/// Artifact construction inputs: every field is explicit, nothing inferred.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArtifactRequest {
    pub model_id: String,
    pub pmoke_version: String,
    pub backend_versions: BTreeMap<String, String>,
    pub binding: ModelBinding,
    pub variance: PhaseVarianceOutput,
    pub correlation: Option<CorrelationOutput>,
    pub recipe: PhaseVarianceRecipe,
    pub correlation_recipe: Option<CorrelationRecipe>,
    pub plan: BlockPlan,
    pub source_digests: Vec<String>,
    pub seed: u64,
    pub tuning: TuningMode,
    pub heldout: HeldoutReport,
}

/// Held-out (tuning-validation) evidence attached to the artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeldoutReport {
    pub blocks: usize,
    pub profile_rmse_v2: Option<f64>,
    pub standardized_lag1: Option<f64>,
}

/// Artifact with its canonical bytes and SHA-256 identity.
#[derive(Debug, Clone, PartialEq)]
pub struct ArtifactWithHash {
    pub artifact: CalibrationArtifact,
    pub json_bytes: Vec<u8>,
    pub sha256: [u8; 32],
    pub sha256_hex: String,
}

fn validate_binding(binding: &ModelBinding) -> Result<()> {
    if binding.channel == 0 {
        return Err(invalid_request("binding channel must be nonzero"));
    }
    if binding.voltage_unit != "V" {
        return Err(invalid_request("binding voltage_unit must be \"V\""));
    }
    if !is_positive_finite(binding.sample_interval_s) {
        return Err(invalid_request(
            "binding sample_interval_s must be positive finite",
        ));
    }
    if !is_positive_finite(binding.reference_frequency_hz) {
        return Err(invalid_request(
            "binding reference_frequency_hz must be positive finite",
        ));
    }
    if !is_positive_finite(binding.sample_interval_rel_tol) {
        return Err(invalid_request(
            "binding sample_interval_rel_tol must be positive finite",
        ));
    }
    if !is_positive_finite(binding.frequency_rel_tol) {
        return Err(invalid_request(
            "binding frequency_rel_tol must be positive finite",
        ));
    }
    if binding.phase_convention != CALIBRATION_PHASE_CONVENTION {
        return Err(invalid_request(
            "binding phase_convention is not the accepted convention",
        ));
    }
    Ok(())
}

/// Build the immutable artifact: validate, serialize to canonical bytes
/// (struct field order, Ryu floats), enforce the byte cap, and identify by
/// SHA-256 of exactly those bytes.
pub fn build_artifact(request: ArtifactRequest) -> Result<ArtifactWithHash> {
    ensure_fixed_tuning(&request.tuning)?;
    validate_binding(&request.binding)?;
    if request.model_id.is_empty() {
        return Err(invalid_request("model_id must be nonempty"));
    }
    if !is_positive_finite(request.variance.v0) {
        return Err(AnalysisError::new(
            "invalid_noise_model",
            "artifact v0 must be positive finite",
        ));
    }
    for (bin, variance) in request.variance.variances.iter().enumerate() {
        if !is_positive_finite(*variance) {
            return Err(AnalysisError::new(
                "invalid_noise_model",
                format!("artifact phase bin {bin} variance must be positive finite"),
            ));
        }
    }
    if request.variance.counts.len() != request.variance.variances.len() {
        return Err(invalid_request("variance counts and values must align"));
    }
    let mut modes = vec!["identity".to_string(), "phase_diagonal".to_string()];
    let correlation = match (&request.correlation, &request.correlation_recipe) {
        (Some(output), Some(recipe)) => {
            if output.lags.len() != recipe.max_lag + 1 {
                return Err(invalid_request("correlation lags and recipe disagree"));
            }
            if !output.spd_validated {
                return Err(AnalysisError::new(
                    "covariance_not_spd",
                    "artifact correlation must carry SPD validation",
                ));
            }
            modes.push("phase_correlated".to_string());
            Some(CorrelationTable {
                lags: output.lags.clone(),
                lag_step_s: output.lag_step_s,
                support_samples: output.support_samples,
                taper: output.taper_id.clone(),
                shrinkage_eta: output.eta,
                spd_validated: true,
                tail_energy: output.tail_energy,
                max_tail_lag: output.max_tail_lag,
            })
        }
        (None, None) => None,
        _ => {
            return Err(invalid_request(
                "correlation output and recipe must be supplied together",
            ));
        }
    };
    let mut recipe_settings = BTreeMap::new();
    recipe_settings.insert("bins".to_string(), request.recipe.bins.to_string());
    recipe_settings.insert(
        "min_samples_per_bin".to_string(),
        request.recipe.min_samples_per_bin.to_string(),
    );
    recipe_settings.insert(
        "min_cycles_per_bin".to_string(),
        request.recipe.min_cycles_per_bin.to_string(),
    );
    recipe_settings.insert(
        "shrinkage_alpha".to_string(),
        request.recipe.shrinkage_alpha.to_string(),
    );
    recipe_settings.insert(
        "floor_ratio".to_string(),
        request.recipe.floor_ratio.to_string(),
    );
    let artifact = CalibrationArtifact {
        schema_version: CALIBRATION_ARTIFACT_SCHEMA_VERSION,
        algorithm_version: CALIBRATION_ALGORITHM_VERSION.to_string(),
        model_id: request.model_id,
        builder: BuilderInfo {
            pmoke_version: request.pmoke_version,
            backend_versions: request.backend_versions,
            recipe_settings,
        },
        binding: request.binding,
        reference_variance_v2: request.variance.v0,
        phase: PhaseTable {
            bins: request.variance.variances.len(),
            center_convention: "bin_centers_at_2pi*(i+0.5)/bins".to_string(),
            variances_v2: request.variance.variances,
            interpolation: VARIANCE_INTERP_ID.to_string(),
        },
        correlation,
        regularization: RegularizationRecord {
            smoothing: "single_cyclic_pass_1_2_1_over_4".to_string(),
            shrinkage_alpha_v: request.recipe.shrinkage_alpha,
            variance_floor_ratio: request.recipe.floor_ratio,
            floor_activations: request.variance.floor_activations,
            correlation_taper: CORRELATION_TAPER_ID.to_string(),
            correlation_eta: request
                .correlation_recipe
                .map(|recipe| recipe.shrinkage_eta)
                .unwrap_or(0.0),
        },
        training: TrainingRecord {
            source_digests: request.source_digests,
            intervals: request.plan.intervals.clone(),
            blocks: request.plan.blocks,
            exclusions: request.plan.exclusions,
            seed: request.seed,
            per_bin_counts: request.variance.counts,
            per_bin_cycles: request.variance.distinct_cycles,
            contributing_blocks: request.variance.contributing_blocks,
        },
        validation: ValidationRecord {
            heldout_blocks: request.heldout.blocks,
            profile_rmse_v2: request.heldout.profile_rmse_v2,
            standardized_lag1: request.heldout.standardized_lag1,
            dof_treatment: "pooled_count_minus_one_approximate".to_string(),
            nuisance_params_per_block: NUISANCE_PARAMETERS,
            limits: vec![
                "residual variance is a noise-estimation input, not a physical mechanism"
                    .to_string(),
                "long-block nuisance fit is not a validation of the short-window runtime estimator"
                    .to_string(),
            ],
        },
        capabilities: CapabilityRecord {
            modes,
            geometry_restrictions: vec![
                "single recorded geometry/reference/model digest per plan".to_string(),
            ],
        },
    };
    let json_bytes = serde_json::to_vec(&artifact).map_err(|error| {
        AnalysisError::new(
            "io_failure",
            format!("artifact serialization failed: {error}"),
        )
    })?;
    if json_bytes.len() > MAX_CALIBRATION_ARTIFACT_BYTES {
        return Err(AnalysisError::new(
            "resource_limit_exceeded",
            format!(
                "artifact holds {} bytes above cap {MAX_CALIBRATION_ARTIFACT_BYTES}",
                json_bytes.len()
            ),
        ));
    }
    let mut hasher = Sha256::new();
    hasher.update(&json_bytes);
    let digest: [u8; 32] = hasher.finalize().into();
    let sha256_hex = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(ArtifactWithHash {
        artifact,
        json_bytes,
        sha256: digest,
        sha256_hex,
    })
}

// ---------------------------------------------------------------------------
// Applicability inspection (pure; FR-031).
// ---------------------------------------------------------------------------

/// Inference-time conditions to check against a frozen model binding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApplicabilityRequest {
    pub channel: u32,
    pub sample_interval_s: f64,
    pub reference_frequency_hz: f64,
    pub voltage_unit: String,
    pub phase_convention: String,
    pub acquisition_known: bool,
}

/// One applicability failure with its contract code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicabilityError {
    pub code: String,
    pub message: String,
}

/// Applicability verdict: hard errors versus explicitly unverified warnings.
/// Unknown conditions stay unverified; production suitability then needs an
/// explicit operator decision, never an agent's guess.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApplicabilityReport {
    pub compatible: bool,
    pub errors: Vec<ApplicabilityError>,
    pub warnings: Vec<String>,
}

fn binding_error(message: impl Into<String>) -> ApplicabilityError {
    ApplicabilityError {
        code: "model_binding_mismatch".to_string(),
        message: message.into(),
    }
}

/// Check inference conditions against the frozen binding without touching
/// any source data. Cross-channel reuse is a hard error.
pub fn inspect_applicability(
    model: &CalibrationArtifact,
    request: &ApplicabilityRequest,
) -> ApplicabilityReport {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    if request.channel != model.binding.channel {
        errors.push(binding_error(format!(
            "channel {} does not match calibrated channel {}",
            request.channel, model.binding.channel
        )));
    }
    let dt_detail = (request.sample_interval_s - model.binding.sample_interval_s).abs()
        / model.binding.sample_interval_s;
    if !dt_detail.is_finite() || dt_detail > model.binding.sample_interval_rel_tol {
        errors.push(binding_error(format!(
            "sample interval relative deviation {dt_detail:.3e} exceeds {:.1e}",
            model.binding.sample_interval_rel_tol
        )));
    }
    let freq_detail = (request.reference_frequency_hz - model.binding.reference_frequency_hz).abs()
        / model.binding.reference_frequency_hz;
    if !freq_detail.is_finite() || freq_detail > model.binding.frequency_rel_tol {
        errors.push(binding_error(format!(
            "reference frequency relative deviation {freq_detail:.3e} exceeds {:.1e}",
            model.binding.frequency_rel_tol
        )));
    }
    if request.voltage_unit != model.binding.voltage_unit {
        errors.push(binding_error(format!(
            "voltage unit {} does not match calibrated {}",
            request.voltage_unit, model.binding.voltage_unit
        )));
    }
    if request.phase_convention != model.binding.phase_convention {
        errors.push(binding_error(
            "phase convention does not match the calibrated convention",
        ));
    }
    if model.binding.acquisition.device.is_none()
        || model.binding.acquisition.gain.is_none()
        || model.binding.acquisition.bandwidth_hz.is_none()
        || !request.acquisition_known
    {
        warnings.push("unverified_acquisition_conditions".to_string());
    }
    ApplicabilityReport {
        compatible: errors.is_empty(),
        errors,
        warnings,
    }
}

// ---------------------------------------------------------------------------
// SCS adequacy diagnostic (AT-041).
// ---------------------------------------------------------------------------

/// Standardized residuals with phases for one adequacy group.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdequacyGroup {
    pub standardized: Vec<f64>,
    pub phases: Vec<f64>,
}

/// Prespecified adequacy thresholds (fixed before seeing the data).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AdequacyPolicy {
    pub lags: usize,
    pub min_pairs_per_cell: usize,
    pub max_phase_spread: f64,
    pub max_reserved_shift: f64,
}

/// SCS adequacy verdict. A nonseparable process yields a model-mismatch
/// diagnostic (`adequate: false` with a reason), never a false adequacy
/// claim. Missing power/coverage yields unverified applicability.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AdequacyReport {
    pub adequate: bool,
    pub phase_spread: f64,
    pub reserved_shift: f64,
    pub reason: String,
    pub warnings: Vec<String>,
}

fn lag_pairs(group: &AdequacyGroup, lag: usize) -> Result<Vec<(f64, f64, f64)>> {
    if group.standardized.len() != group.phases.len() {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            "adequacy group residuals and phases must align",
        ));
    }
    if group.standardized.len() <= lag {
        return Err(AnalysisError::new(
            "insufficient_calibration",
            "adequacy group is shorter than the diagnostic lag",
        ));
    }
    for (value, phase) in group.standardized.iter().zip(group.phases.iter()) {
        if !value.is_finite() || !phase.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                "adequacy groups must be finite",
            ));
        }
    }
    Ok((0..(group.standardized.len() - lag))
        .map(|i| {
            (
                group.standardized[i],
                group.standardized[i + lag],
                group.phases[i],
            )
        })
        .collect())
}

/// Compare lag covariance across phase octants (training) and against
/// reserved blocks. Time-varying structure that averages out within phase
/// pooling is caught by the reserved comparison.
pub fn scs_adequacy(
    training: &[AdequacyGroup],
    reserved: &[AdequacyGroup],
    policy: AdequacyPolicy,
) -> Result<AdequacyReport> {
    if policy.lags == 0 {
        return Err(invalid_request("adequacy policy needs positive lags"));
    }
    if training.is_empty() || reserved.is_empty() {
        return Err(AnalysisError::new(
            "insufficient_calibration",
            "adequacy needs training and reserved groups",
        ));
    }
    const OCTANTS: usize = 8;
    // Per-lag, per-octant correlations on training pairs grouped by the
    // first sample's phase, plus pooled lag vectors for both roles.
    let mut train_pooled = vec![0.0_f64; policy.lags];
    let mut reserved_pooled = vec![0.0_f64; policy.lags];
    let mut train_counts = vec![0_usize; policy.lags];
    let mut reserved_counts = vec![0_usize; policy.lags];
    let mut octant_num = vec![vec![0.0_f64; OCTANTS]; policy.lags];
    let mut octant_den = vec![vec![0_usize; OCTANTS]; policy.lags];
    for (role_index, groups) in [training, reserved].iter().enumerate() {
        for group in groups.iter() {
            for lag in 1..=policy.lags {
                for (first, second, phase) in lag_pairs(group, lag)?.into_iter() {
                    let octant = ((phase.rem_euclid(TAU) / TAU * OCTANTS as f64).floor() as usize)
                        .min(OCTANTS - 1);
                    if role_index == 0 {
                        train_pooled[lag - 1] += first * second;
                        train_counts[lag - 1] += 1;
                        octant_num[lag - 1][octant] += first * second;
                        octant_den[lag - 1][octant] += 1;
                    } else {
                        reserved_pooled[lag - 1] += first * second;
                        reserved_counts[lag - 1] += 1;
                    }
                }
            }
        }
    }
    for lag in 0..policy.lags {
        if train_counts[lag] == 0 || reserved_counts[lag] == 0 {
            return Err(AnalysisError::new(
                "insufficient_calibration",
                format!("adequacy lag {} has no pairs", lag + 1),
            ));
        }
        train_pooled[lag] /= train_counts[lag] as f64;
        reserved_pooled[lag] /= reserved_counts[lag] as f64;
    }
    let mut phase_spread: f64 = 0.0;
    for lag in 0..policy.lags {
        let mut low = f64::INFINITY;
        let mut high = f64::NEG_INFINITY;
        for octant in 0..OCTANTS {
            if octant_den[lag][octant] < policy.min_pairs_per_cell {
                return Err(AnalysisError::new(
                    "insufficient_calibration",
                    format!(
                        "octant {octant} lag {} holds {} pairs below minimum {}",
                        lag + 1,
                        octant_den[lag][octant],
                        policy.min_pairs_per_cell
                    ),
                ));
            }
            let value = octant_num[lag][octant] / octant_den[lag][octant] as f64;
            low = low.min(value);
            high = high.max(value);
        }
        phase_spread = phase_spread.max(high - low);
    }
    let reserved_shift: f64 = train_pooled
        .iter()
        .zip(reserved_pooled.iter())
        .map(|(train, reserved)| (train - reserved).abs())
        .fold(0.0, f64::max);
    let mut warnings = Vec::new();
    let (adequate, reason) = if phase_spread > policy.max_phase_spread {
        warnings.push("insufficient_scs_adequacy".to_string());
        (
            false,
            format!(
                "phase spread {phase_spread:.6} across octants exceeds {:.6}: \
                 lag structure depends on phase beyond an SCS description",
                policy.max_phase_spread
            ),
        )
    } else if reserved_shift > policy.max_reserved_shift {
        warnings.push("insufficient_scs_adequacy".to_string());
        (
            false,
            format!(
                "reserved shift {reserved_shift:.6} exceeds {:.6}: \
                 lag structure drifts between training and reserved blocks, \
                 inconsistent with one stationary SCS model",
                policy.max_reserved_shift
            ),
        )
    } else {
        (
            true,
            format!(
                "phase spread {phase_spread:.6} and reserved shift {reserved_shift:.6} \
                 within policy; SCS description adequate at this evidence level"
            ),
        )
    };
    Ok(AdequacyReport {
        adequate,
        phase_spread,
        reserved_shift,
        reason,
        warnings,
    })
}
