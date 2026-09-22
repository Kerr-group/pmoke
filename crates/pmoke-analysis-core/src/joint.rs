//! Fixed-support joint harmonic estimator core (WP-2, milestone M2;
//! correlated modes WP-4, milestone M3).
//!
//! Direct finite-window reference: Householder QR solution of the whitened
//! design with explicit rank/condition checks, plus design-model covariance.
//! All four noise modes are executable: identity, phase-diagonal,
//! stationary-correlated (`v0 * C_N`), and phase-correlated (`S_N C_N S_N`).
//! Coordinate conventions follow NUMERICS sections 1-2: original-sample
//! support, `phi = 2 pi f t - phase`, column order `[DC, cos, sin, ...]`,
//! and the peak-amplitude map `Xk = b`, `Yk = a`.
//!
//! Backend: nalgebra 0.35.0, no-default plus `std` (A-010 spike). The
//! rectangular solve uses only public APIs: thin `Q`, `Q^T y`, and manual
//! back-substitution on the upper-trapezoidal `R`. Rank and conditioning
//! come from an SVD diagnostic on the thin `R` factor of the same whitened
//! design (P3: reuses the QR factorization instead of cloning the `N x P`
//! design a second time); the independent
//! cross-check is the SciPy oracle, not a second nalgebra path.
//!
//! Scaling policy: the solver factors the supplied whitened design as-is,
//! with no internal column equilibration. Callers provide already-scaled
//! (whitened) input; reported conditions refer to that supplied matrix.

use crate::{AnalysisError, Result};
use nalgebra::{DMatrix, DVector};
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

/// Maximum solver parameters per runtime model (NFR-008).
pub const MAX_MODEL_PARAMETERS: usize = 25;
/// Default scaled-design relative singular-value floor (TOL-06).
pub const DEFAULT_RANK_TOL: f64 = 1e-10;
/// Default scaled-design condition rejection threshold (TOL-06).
pub const DEFAULT_MAX_CONDITION: f64 = 1e8;
/// Default realized-noise-covariance condition cap (TOL-07).
pub const DEFAULT_MAX_NOISE_CONDITION: f64 = 1e10;
/// Relative step tolerance for timebase uniformity, mirroring the recorded
/// waveform preflight convention.
pub const TIMEBASE_RELATIVE_TOLERANCE: f64 = 1e-6;
/// Maximum window samples for correlated whitening (NFR-008 native support).
/// The Toeplitz factor is quadratic in N; larger requests fail before any
/// allocation rather than paginating silently.
pub const MAX_WINDOW_SAMPLES: usize = 4095;
/// Correlation lag-step agreement with the effective sample interval
/// (relative). Applying a model to another geometry without re-tapering is
/// rejected, never silent.
pub const CORRELATION_DT_REL_TOL: f64 = 1e-9;
/// Normalized lag-zero acceptance window around exactly one.
pub const LAG_ZERO_TOL: f64 = 1e-9;
/// Default bounded-jitter ceiling in normalized-lag units (FR-017). Zero
/// disables jitter: a non-SPD factor fails instead of escalating silently.
pub const DEFAULT_MAX_JITTER: f64 = 0.0;

/// Executable noise-model selector (FR-013). Every mode whitens through its
/// stated covariance; a GLS label never hides an OLS implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NoiseMode {
    Identity,
    PhaseDiagonal,
    StationaryCorrelated,
    PhaseCorrelated,
}

/// Immutable validated signal-model description (NUMERICS section 2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarmonicSignalModel {
    /// Ascending unique positive harmonics; contains every output harmonic.
    pub fit_harmonics: Vec<usize>,
    /// Requested outputs; exactly 1..6 in v1.
    pub output_harmonics: Vec<usize>,
    /// Envelope degree; exactly 0 in v1.
    pub envelope_degree: u8,
}

/// Immutable validated noise-model description (NUMERICS section 4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NoiseModel {
    pub mode: NoiseMode,
    /// Positive reference variance v0 in V^2.
    pub reference_variance_v2: f64,
    /// Absolute per-bin variances at uniform centers `2 pi (b + 0.5) / B`.
    /// Required for phase-diagonal and phase-correlated modes; `None` else.
    pub variance_bins: Option<Vec<f64>>,
    /// Normalized correlation kernel. Required for stationary-correlated and
    /// phase-correlated modes; `None` else.
    pub correlation: Option<CorrelationKernel>,
}

/// Normalized finite-lag correlation kernel (NUMERICS 4.2): explicit lag
/// sequence with units, dt, and lag-zero normalization. The runtime Toeplitz
/// factor pads zeros beyond the recorded support and is SPD-validated for
/// the requested geometry; naive truncation without taper/shrinkage evidence
/// is rejected upstream, not repaired here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrelationKernel {
    /// Normalized lags `c[0..=J]` with `c[0] == 1`.
    pub lags: Vec<f64>,
    /// Lag step in seconds; must agree with the effective sample interval.
    pub lag_step_s: f64,
}

/// Solver tolerances (TOL-06/07 policy carriers).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct JointSolverTolerances {
    /// Relative singular-value floor for rank decisions.
    pub rank_tol: f64,
    /// Scaled-design condition rejection threshold.
    pub max_condition: f64,
    /// Realized noise-covariance condition cap.
    pub max_noise_condition: f64,
    /// Bounded-jitter ceiling in normalized-lag units (FR-017). Zero means
    /// a non-SPD Toeplitz factor fails with `covariance_not_spd`.
    pub max_jitter_v2: f64,
}

/// Bundled estimator settings (INTERFACES section 1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JointHarmonicSettings {
    pub model: HarmonicSignalModel,
    pub noise: NoiseModel,
    pub tolerances: JointSolverTolerances,
}

impl Default for JointSolverTolerances {
    fn default() -> Self {
        Self {
            rank_tol: DEFAULT_RANK_TOL,
            max_condition: DEFAULT_MAX_CONDITION,
            max_noise_condition: DEFAULT_MAX_NOISE_CONDITION,
            max_jitter_v2: DEFAULT_MAX_JITTER,
        }
    }
}

/// Validate a coherent monotonic uniform timebase and return the effective
/// interval, mirroring the recorded waveform preflight: strict increase,
/// per-step uniformity within relative tolerance plus serialization
/// roundoff, and consistency with the declared sample rate
/// (INTERFACES `invalid_timebase`). The effective interval is the first
/// sample interval, by definition shared with correlated inference: a
/// kernel built from the returned value always satisfies the correlation
/// lag-step binding on the validated grid.
pub fn validate_timebase(times: &[f64], sample_rate_hz: f64) -> Result<f64> {
    if times.len() < 2 {
        return Err(AnalysisError::new(
            "empty_input",
            "timebase needs at least two samples",
        ));
    }
    for (index, time) in times.iter().enumerate() {
        if !time.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                format!("time axis is non-finite at sample {index}"),
            ));
        }
    }
    if !(sample_rate_hz.is_finite() && sample_rate_hz > 0.0) {
        return Err(AnalysisError::new(
            "non_finite_input",
            "sample_rate_hz must be finite and positive",
        ));
    }
    let interval = times[1] - times[0];
    if !interval.is_finite() || interval <= 0.0 {
        return Err(AnalysisError::new(
            "invalid_timebase",
            format!("timebase must start with a positive step (got {interval})"),
        ));
    }
    for index in 2..times.len() {
        let step = times[index] - times[index - 1];
        // Strict increase is independent of the uniformity allowance: at
        // large time origins the roundoff allowance can exceed a sample
        // step, so positivity is checked before the interval comparison.
        if !step.is_finite() || step <= 0.0 {
            return Err(AnalysisError::new(
                "invalid_timebase",
                format!("timebase step is not strictly increasing at sample {index}: {step}"),
            ));
        }
        let roundoff = times[index].abs().max(times[index - 1].abs()) * f64::EPSILON * 16.0;
        let tolerance = (interval.abs() * TIMEBASE_RELATIVE_TOLERANCE).max(roundoff);
        if (step - interval).abs() > tolerance {
            return Err(AnalysisError::new(
                "invalid_timebase",
                format!("timebase step changes at sample {index}: {step}, expected {interval}"),
            ));
        }
    }
    let declared = 1.0 / sample_rate_hz;
    let tolerance = declared.abs() * TIMEBASE_RELATIVE_TOLERANCE;
    if (interval - declared).abs() > tolerance {
        return Err(AnalysisError::new(
            "invalid_timebase",
            format!("timebase interval {interval} disagrees with sample rate {sample_rate_hz}"),
        ));
    }
    Ok(interval)
}

/// Direct-solver result on one finite window.
#[derive(Debug, Clone)]
pub struct JointEstimate {
    /// Raw peak coefficients in design order `[DC, a1, b1, ...]`.
    pub beta: Vec<f64>,
    /// Legacy-order quadratures `[X1, Y1, ..., X6, Y6]`.
    pub xy: Vec<f64>,
    /// Design-model covariance in beta space (same order as beta).
    pub covariance_beta: Vec<Vec<f64>>,
    /// Design-model covariance mapped to XY space (same order as xy).
    pub covariance_xy: Vec<Vec<f64>>,
    /// Numerical rank of the whitened design.
    pub rank: usize,
    /// Scaled-design condition number estimate.
    pub condition: f64,
    /// Standardized residual RMS per unit noise: the whitened RMS divided by
    /// the square root of the variance scale, so identity and diagonal modes
    /// report identical values for identical covariances. Dimensionless, not
    /// volts: it is numerically equal to the raw volts RMS for Identity
    /// noise when the reference variance is exactly 1 V^2 (more generally
    /// whenever the effective covariance is the unit identity), and stays
    /// dimensionless in that coincidence. Other modes normalize by their
    /// own realized bins and/or correlation factor, so the reference
    /// variance alone never sets the reading. Never the diagnostics-path
    /// volts RMS of a raw nuisance residual (different estimator,
    /// different normalization).
    pub residual_rms: f64,
    /// Bounded jitter actually applied to the normalized Toeplitz diagonal
    /// in V^2-normalized units (FR-017). Zero unless the factor needed it;
    /// every executed regularization value is recorded, never hidden.
    pub jitter_applied_v2: f64,
}

fn require_finite(name: &str, value: f64) -> Result<()> {
    if !value.is_finite() {
        return Err(AnalysisError::new(
            "non_finite_input",
            format!("{name} must be finite"),
        ));
    }
    Ok(())
}

/// Validate the signal model without touching data (FR-009).
pub fn validate_signal_model(model: &HarmonicSignalModel, sample_rate_hz: f64) -> Result<()> {
    if model.envelope_degree != 0 {
        return Err(AnalysisError::new(
            "unsupported_envelope_degree",
            "v1 supports envelope_degree 0 only",
        ));
    }
    if model.fit_harmonics.is_empty() {
        return Err(AnalysisError::new(
            "invalid_harmonics",
            "fit_harmonics must not be empty",
        ));
    }
    if model
        .fit_harmonics
        .windows(2)
        .any(|pair| pair[0] >= pair[1])
    {
        return Err(AnalysisError::new(
            "invalid_harmonics",
            "fit_harmonics must be ascending unique positive integers",
        ));
    }
    if model.fit_harmonics[0] == 0 {
        return Err(AnalysisError::new(
            "invalid_harmonics",
            "fit_harmonics must be ascending unique positive integers",
        ));
    }
    for output in &model.output_harmonics {
        if !model.fit_harmonics.contains(output) {
            return Err(AnalysisError::new(
                "invalid_harmonics",
                format!("output harmonic {output} is not in the fitting list"),
            ));
        }
    }
    if model.output_harmonics != [1, 2, 3, 4, 5, 6] {
        return Err(AnalysisError::new(
            "unsupported_output_set",
            "v1 outputs exactly harmonics 1..6",
        ));
    }
    if !(sample_rate_hz.is_finite() && sample_rate_hz > 0.0) {
        return Err(AnalysisError::new(
            "non_finite_input",
            "sample_rate_hz must be finite and positive",
        ));
    }
    // Per-harmonic Nyquist needs the reference frequency against the
    // recorded coordinates; design_matrix enforces it from actual times.
    let parameters = 1 + 2 * model.fit_harmonics.len();
    if parameters > MAX_MODEL_PARAMETERS {
        return Err(AnalysisError::new(
            "too_many_parameters",
            format!("{parameters} parameters exceed the cap {MAX_MODEL_PARAMETERS}"),
        ));
    }
    Ok(())
}

/// Validate the noise model without touching data (FR-013/014).
pub fn validate_noise_model(model: &NoiseModel) -> Result<()> {
    if !(model.reference_variance_v2.is_finite() && model.reference_variance_v2 > 0.0) {
        return Err(AnalysisError::new(
            "invalid_noise_model",
            "reference_variance_v2 must be positive finite",
        ));
    }
    match (&model.mode, &model.variance_bins) {
        (NoiseMode::PhaseDiagonal | NoiseMode::PhaseCorrelated, Some(bins)) => {
            if bins.len() < 2 {
                return Err(AnalysisError::new(
                    "invalid_noise_model",
                    "phase-diagonal modes need at least two variance bins",
                ));
            }
            if !bins.iter().all(|v| v.is_finite() && *v > 0.0) {
                return Err(AnalysisError::new(
                    "invalid_noise_model",
                    "variance bins must be positive finite",
                ));
            }
        }
        (NoiseMode::PhaseDiagonal | NoiseMode::PhaseCorrelated, None) => {
            return Err(AnalysisError::new(
                "missing_noise_component",
                "phase-diagonal modes need variance bins",
            ));
        }
        (_, Some(_)) => {
            return Err(AnalysisError::new(
                "invalid_noise_model",
                "variance bins are only valid for phase-diagonal modes",
            ));
        }
        _ => {}
    }
    match (&model.mode, &model.correlation) {
        (NoiseMode::StationaryCorrelated | NoiseMode::PhaseCorrelated, Some(kernel)) => {
            validate_correlation_kernel(kernel)?
        }
        (NoiseMode::StationaryCorrelated | NoiseMode::PhaseCorrelated, None) => {
            return Err(AnalysisError::new(
                "missing_noise_component",
                "correlated modes need a correlation kernel",
            ));
        }
        (_, Some(_)) => {
            return Err(AnalysisError::new(
                "invalid_noise_model",
                "correlation kernels are only valid for correlated modes",
            ));
        }
        _ => {}
    }
    Ok(())
}

/// Validate a normalized finite-lag kernel without touching data (FR-015).
pub fn validate_correlation_kernel(kernel: &CorrelationKernel) -> Result<()> {
    if kernel.lags.is_empty() {
        return Err(AnalysisError::new(
            "invalid_noise_model",
            "correlation kernel needs at least the lag-zero entry",
        ));
    }
    for (lag, value) in kernel.lags.iter().enumerate() {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                format!("correlation lag {lag} must be finite"),
            ));
        }
    }
    if (kernel.lags[0] - 1.0).abs() > LAG_ZERO_TOL {
        return Err(AnalysisError::new(
            "invalid_noise_model",
            "correlation lag zero must normalize to one",
        ));
    }
    if kernel.lag_step_s <= 0.0 || !kernel.lag_step_s.is_finite() {
        return Err(AnalysisError::new(
            "invalid_noise_model",
            "correlation lag_step_s must be positive finite",
        ));
    }
    Ok(())
}

/// Finite Toeplitz factor from recorded lags with zeros beyond the support
/// (NUMERICS 4.2).
pub fn toeplitz_from_lags(lags: &[f64], samples: usize) -> DMatrix<f64> {
    let mut factor = DMatrix::zeros(samples, samples);
    for row in 0..samples {
        for column in 0..samples {
            let lag = row.abs_diff(column);
            if lag < lags.len() {
                factor[(row, column)] = lags[lag];
            }
        }
    }
    factor
}

/// Forward substitution `L X = B` for lower-triangular `L` with any number
/// of right-hand sides. Intended for Cholesky factors of covariance models,
/// hence the `covariance_not_spd` code on zero diagonals: a zero diagonal
/// means the purported factor was not positive definite. Zero diagonals
/// fail instead of dividing.
///
/// Cost note (P1): the kernel below blocks across the right-hand sides
/// through a row-major work buffer (transpose once, solve with contiguous
/// row accesses, transpose back). Each `(row, column)` still accumulates
/// `inner = 0..row` subtractions in increasing order, exactly like the
/// naive column-outer triple loop, so results are bit-identical; only the
/// reuse changes. Each `L[row, inner]` is loaded once and fanned out
/// across every right-hand side instead of once per column. No threading
/// inside the window: the outer rayon pool owns parallelism.
pub fn forward_substitute(lower: &DMatrix<f64>, rhs: &DMatrix<f64>) -> Result<DMatrix<f64>> {
    let dimension = lower.nrows();
    if lower.ncols() != dimension || rhs.nrows() != dimension {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            "triangular solve needs a square factor matching every RHS row",
        ));
    }
    for value in lower.iter() {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                "triangular factor must be finite",
            ));
        }
    }
    for value in rhs.iter() {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                "triangular right-hand side must be finite",
            ));
        }
    }
    let mut solution = rhs.clone();
    blocked_forward_kernel(
        lower.as_slice(),
        solution.as_mut_slice(),
        dimension,
        rhs.ncols(),
    )?;
    for value in solution.iter() {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_output",
                "triangular solve produced a non-finite value",
            ));
        }
    }
    Ok(solution)
}

/// In-place blocked forward substitution `L X = B` over the caller's
/// buffer: the correlated whitener packs `[design | response]` once and
/// solves without the extra per-window right-hand-side copy. Validation,
/// diagonal-failure row, and output checks match [`forward_substitute`]
/// exactly; only the redundant allocation is hoisted.
fn forward_substitute_in_place(lower: &DMatrix<f64>, solution: &mut DMatrix<f64>) -> Result<()> {
    let dimension = lower.nrows();
    if lower.ncols() != dimension || solution.nrows() != dimension {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            "triangular solve needs a square factor matching every RHS row",
        ));
    }
    for value in lower.iter() {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                "triangular factor must be finite",
            ));
        }
    }
    for value in solution.iter() {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                "triangular right-hand side must be finite",
            ));
        }
    }
    let ncols = solution.ncols();
    blocked_forward_kernel(lower.as_slice(), solution.as_mut_slice(), dimension, ncols)?;
    for value in solution.iter() {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_output",
                "triangular solve produced a non-finite value",
            ));
        }
    }
    Ok(())
}

/// Blocked forward-substitution kernel over column-major slices: `lower`
/// holds the `dimension x dimension` factor and `solution` the
/// `dimension x ncols` right-hand sides, both column-major as stored by
/// [`DMatrix`].
///
/// Layout blocking (P1): the right-hand sides are transposed once into a
/// row-major work buffer, solved with contiguous row accesses, and
/// transposed back. The naive column-outer loop streams the 32MB factor
/// once per right-hand side (~26x at N=2003/P=25) and is host-memory
/// bound; the row-major solve loads each `L[row, inner]` once and fans it
/// out across every right-hand side while every work-buffer access stays
/// contiguous and vectorizable.
///
/// Bit-identity: per `(row, column)` the subtractions still run in
/// `inner = 0..row` order with the division landing last, exactly the
/// naive order -- only the storage layout changes, never the operation
/// sequence. The zero-diagonal failure reports the first such row,
/// matching the column-outer loop's discovery order. Empty right-hand
/// sides solve trivially with no diagonal checks, also matching.
fn blocked_forward_kernel(
    lower: &[f64],
    solution: &mut [f64],
    dimension: usize,
    ncols: usize,
) -> Result<()> {
    if ncols == 0 {
        return Ok(());
    }
    // Transpose to row-major work: work[row * ncols + column].
    let mut work = vec![0.0_f64; dimension * ncols];
    for column in 0..ncols {
        let source = &solution[column * dimension..(column + 1) * dimension];
        for row in 0..dimension {
            work[row * ncols + column] = source[row];
        }
    }
    for row in 0..dimension {
        let base = row * ncols;
        for inner in 0..row {
            let factor = lower[inner * dimension + row];
            let inner_base = inner * ncols;
            for column in 0..ncols {
                work[base + column] -= factor * work[inner_base + column];
            }
        }
        let diagonal = lower[row * dimension + row];
        if diagonal == 0.0 {
            return Err(AnalysisError::new(
                "covariance_not_spd",
                format!("zero triangular diagonal at row {row}"),
            ));
        }
        for column in 0..ncols {
            work[base + column] /= diagonal;
        }
    }
    // Transpose back to the column-major output.
    for column in 0..ncols {
        let target = &mut solution[column * dimension..(column + 1) * dimension];
        for row in 0..dimension {
            target[row] = work[row * ncols + column];
        }
    }
    Ok(())
}

/// Cholesky factor of the normalized Toeplitz matrix with the bounded
/// jitter policy (FR-017): try plain first; on failure add exactly
/// `max_jitter_v2` to the diagonal once and record it; otherwise fail with
/// `covariance_not_spd`. No search for smaller workable jitter and no
/// escalation beyond the stated ceiling.
pub fn cholesky_factor(toeplitz: &DMatrix<f64>, max_jitter_v2: f64) -> Result<(DMatrix<f64>, f64)> {
    if max_jitter_v2 < 0.0 || !max_jitter_v2.is_finite() {
        return Err(AnalysisError::new(
            "non_finite_input",
            "max_jitter_v2 must be finite non-negative",
        ));
    }
    if let Some(factor) = toeplitz.clone().cholesky() {
        return Ok((factor.l(), 0.0));
    }
    if max_jitter_v2 > 0.0 {
        let mut jittered = toeplitz.clone();
        for diagonal in 0..jittered.nrows() {
            jittered[(diagonal, diagonal)] += max_jitter_v2;
        }
        if let Some(factor) = jittered.cholesky() {
            return Ok((factor.l(), max_jitter_v2));
        }
    }
    Err(AnalysisError::new(
        "covariance_not_spd",
        "correlation Toeplitz factor is not positive definite within the jitter ceiling",
    ))
}

/// TOL-07 gate on the executed noise covariance `R = S (T + j I) S`, where
/// `T` is the recorded-lag Toeplitz factor, `j` the actually applied
/// jitter, and `S = diag(sqrt_variance)` (uniform for stationary modes).
/// Cholesky success only proves positive definiteness, and the whitened
/// design condition only constrains the equation scaling: neither bounds
/// the noise-covariance condition, so it is measured here exactly with a
/// symmetric eigendecomposition of the executed matrix. An unmeasurable or
/// non-positive-definite realization fails closed; iterative probes are
/// deliberately avoided because clustered correlation spectra (for example
/// AR(1) factors) converge too slowly to certify a cap verdict.
fn gate_realized_noise_condition(
    toeplitz: &DMatrix<f64>,
    jitter_v2: f64,
    sqrt_variance: &[f64],
    max_noise_condition: f64,
) -> Result<()> {
    let rows = sqrt_variance.len();
    if toeplitz.nrows() != rows || toeplitz.ncols() != rows || rows < 2 {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            "noise-condition gate needs the executed factor matching every sample scale",
        ));
    }
    for scale in sqrt_variance.iter() {
        if !scale.is_finite() || *scale <= 0.0 {
            return Err(AnalysisError::new(
                "invalid_noise_model",
                "realized noise scales must be positive finite",
            ));
        }
    }
    if !jitter_v2.is_finite() || jitter_v2 < 0.0 {
        return Err(AnalysisError::new(
            "non_finite_input",
            "executed jitter must be finite non-negative",
        ));
    }
    // Executed covariance R = S (T + jI) S, formed explicitly.
    let mut realized = DMatrix::zeros(rows, rows);
    for row in 0..rows {
        for column in 0..rows {
            let mut value = toeplitz[(row, column)];
            if row == column {
                value += jitter_v2;
            }
            let entry = sqrt_variance[row] * value * sqrt_variance[column];
            if !entry.is_finite() {
                return Err(AnalysisError::new(
                    "ill_conditioned_noise_model",
                    "realized noise covariance overflows representation; \
                     refusing the condition verdict",
                ));
            }
            realized[(row, column)] = entry;
        }
    }
    let eigenvalues = realized.symmetric_eigen().eigenvalues;
    let mut smallest = f64::INFINITY;
    let mut largest = f64::NEG_INFINITY;
    for value in eigenvalues.iter() {
        smallest = smallest.min(*value);
        largest = largest.max(*value);
    }
    if !(smallest.is_finite() && largest.is_finite()) || smallest <= 0.0 {
        return Err(AnalysisError::new(
            "ill_conditioned_noise_model",
            "realized noise covariance is not numerically positive definite; \
             refusing the condition verdict",
        ));
    }
    let condition = largest / smallest;
    if !condition.is_finite() || condition > max_noise_condition {
        return Err(AnalysisError::new(
            "ill_conditioned_noise_model",
            format!(
                "realized noise covariance condition {condition:.6e} exceeds cap {max_noise_condition:.6e}",
            ),
        ));
    }
    Ok(())
}

/// Whitened linear system with its variance scale and executed jitter.
#[derive(Debug, Clone)]
pub struct WhitenedSystem {
    /// Whitened design `Dw`.
    pub design: DMatrix<f64>,
    /// Whitened response `yw`.
    pub response: DVector<f64>,
    /// Design-model covariance is `variance_scale * (Dw^T Dw)^-1`.
    pub variance_scale: f64,
    /// Bounded jitter applied to the normalized Toeplitz diagonal (0 except
    /// in correlated modes that needed it).
    pub jitter_applied_v2: f64,
}

/// Build the design matrix in `[DC, cos(k phi), sin(k phi), ...]` order with
/// `phi = 2 pi f t - phase` (NUMERICS section 2). The timebase is validated
/// for monotonic uniform spacing consistent with the declared sample rate.
pub fn design_matrix(
    times: &[f64],
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    model: &HarmonicSignalModel,
    sample_rate_hz: f64,
) -> Result<DMatrix<f64>> {
    let interval = validate_timebase(times, sample_rate_hz)?;
    require_finite("reference_frequency_hz", reference_frequency_hz)?;
    require_finite("reference_phase_rad", reference_phase_rad)?;
    if reference_frequency_hz <= 0.0 {
        return Err(AnalysisError::new(
            "non_finite_input",
            "reference_frequency_hz must be positive",
        ));
    }
    let nyquist = 0.5 / interval;
    let parameters = 1 + 2 * model.fit_harmonics.len();
    let mut design = DMatrix::zeros(times.len(), parameters);
    fill_design_columns(
        &mut design,
        times,
        reference_frequency_hz,
        reference_phase_rad,
        model,
        nyquist,
    )?;
    Ok(design)
}

/// Fill a preallocated design matrix in `[DC, cos(k phi), sin(k phi), ...]`
/// order with `phi = 2 pi f t - phase` (NUMERICS section 2), reusing the
/// caller's buffer across windows instead of allocating per-window column
/// vectors. Each entry is an independent closed-form evaluation, so the
/// filled values are bit-identical to [`design_matrix`]; only allocation
/// cost is hoisted, never arithmetic. The timebase checks run in the same
/// order with the same errors as [`design_matrix`].
pub fn design_matrix_into(
    design: &mut DMatrix<f64>,
    times: &[f64],
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    model: &HarmonicSignalModel,
    sample_rate_hz: f64,
) -> Result<()> {
    let interval = validate_timebase(times, sample_rate_hz)?;
    require_finite("reference_frequency_hz", reference_frequency_hz)?;
    require_finite("reference_phase_rad", reference_phase_rad)?;
    if reference_frequency_hz <= 0.0 {
        return Err(AnalysisError::new(
            "non_finite_input",
            "reference_frequency_hz must be positive",
        ));
    }
    let parameters = 1 + 2 * model.fit_harmonics.len();
    if design.nrows() != times.len() || design.ncols() != parameters {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            format!(
                "design scratch holds {}x{} but the window needs {}x{}",
                design.nrows(),
                design.ncols(),
                times.len(),
                parameters
            ),
        ));
    }
    let nyquist = 0.5 / interval;
    fill_design_columns(
        design,
        times,
        reference_frequency_hz,
        reference_phase_rad,
        model,
        nyquist,
    )
}

/// Shared design evaluation: per-harmonic Nyquist gate in fit order, then
/// the closed-form column fill. Both [`design_matrix`] and
/// [`design_matrix_into`] run this exact sequence, so acceptance and values
/// match between the allocating and the buffer-reusing paths.
fn fill_design_columns(
    design: &mut DMatrix<f64>,
    times: &[f64],
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    model: &HarmonicSignalModel,
    nyquist: f64,
) -> Result<()> {
    for (position, harmonic) in model.fit_harmonics.iter().enumerate() {
        let frequency = *harmonic as f64 * reference_frequency_hz;
        if !frequency.is_finite() || frequency >= nyquist {
            return Err(AnalysisError::new(
                "aliased_harmonic",
                format!("harmonic {harmonic} reaches Nyquist at {nyquist:.6e} Hz"),
            ));
        }
        let cos_column = 1 + 2 * position;
        let sin_column = 2 + 2 * position;
        for (row, time) in times.iter().enumerate() {
            let phi = TAU * reference_frequency_hz * time - reference_phase_rad;
            let (sin_phi, cos_phi) = (*harmonic as f64 * phi).sin_cos();
            design[(row, cos_column)] = cos_phi;
            design[(row, sin_column)] = sin_phi;
        }
    }
    for row in 0..times.len() {
        design[(row, 0)] = 1.0;
    }
    Ok(())
}

/// Periodic piecewise-linear interpolation at uniform bin centers
/// `2 pi (b + 0.5) / B` with last-to-first wrap (NUMERICS 4.1). Interpolates
/// variance, never standard deviation.
pub fn interpolate_variance(phases: &[f64], bins: &[f64]) -> Result<Vec<f64>> {
    if bins.len() < 2 {
        return Err(AnalysisError::new(
            "invalid_noise_model",
            "variance interpolation needs at least two bins",
        ));
    }
    for (index, bin) in bins.iter().enumerate() {
        if !bin.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                format!("variance bin {index} is non-finite"),
            ));
        }
        if *bin <= 0.0 {
            return Err(AnalysisError::new(
                "invalid_noise_model",
                format!("variance bin {index} must stay positive (no concealed zeros)"),
            ));
        }
    }
    let count = bins.len() as f64;
    let step = TAU / count;
    let mut out = Vec::with_capacity(phases.len());
    for phase in phases {
        require_finite("phase", *phase)?;
        let position = (phase.rem_euclid(TAU)) / step - 0.5;
        let below = position.floor() as isize;
        let frac = position - position.floor();
        let count_isize = bins.len() as isize;
        let first = bins[below.rem_euclid(count_isize) as usize];
        let second = bins[(below + 1).rem_euclid(count_isize) as usize];
        out.push((1.0 - frac) * first + frac * second);
    }
    Ok(out)
}

/// TOL-07 source-table variance guard shared by both phase modes: the
/// table max/min ratio is gated against the realized-noise-covariance cap
/// before interpolation. Interpolated sample variances are convex
/// combinations of table entries, so gating the table bounds the diagonal
/// part of the realized covariance in both modes.
fn table_condition_guard(bins: &[f64], max_noise_condition: f64) -> Result<()> {
    let mut smallest = f64::INFINITY;
    let mut largest = 0.0_f64;
    for bin in bins.iter() {
        if !bin.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                "variance bins must be finite",
            ));
        }
        if *bin <= 0.0 {
            return Err(AnalysisError::new(
                "invalid_noise_model",
                "variance bins must be positive",
            ));
        }
        smallest = smallest.min(*bin);
        largest = largest.max(*bin);
    }
    if largest > max_noise_condition * smallest {
        return Err(AnalysisError::new(
            "ill_conditioned_noise_model",
            format!(
                "noise variance table condition {largest:.6e}/{smallest:.6e} exceeds cap {max_noise_condition:.6e}",
            ),
        ));
    }
    Ok(())
}

/// Whiten design and response together. Returns `(Dw, yw, variance_scale)`
/// where the design-model covariance is `variance_scale * (Dw^T Dw)^-1`.
/// For both phase modes the variance-table condition is gated against
/// the TOL-07 policy, independently of design conditioning. Interpolated
/// sample variances are convex combinations of table entries, so gating
/// the table bounds the diagonal part of the realized covariance. The
/// correlated arms additionally gate the full executed `R = S C S` against
/// the same cap (see `gate_realized_noise_condition`).
pub fn whiten(
    design: &DMatrix<f64>,
    signal: &[f64],
    reference_phase_rad: f64,
    reference_frequency_hz: f64,
    times: &[f64],
    noise: &NoiseModel,
    tolerances: JointSolverTolerances,
) -> Result<WhitenedSystem> {
    whiten_inner(
        design,
        signal,
        reference_phase_rad,
        reference_frequency_hz,
        times,
        noise,
        tolerances,
        None,
    )
}

/// Immutable prepared noise plan (PN-M3 numerical acceleration): the mode
/// preprocessing and, for the correlated modes, the Toeplitz Cholesky factor
/// of the normalized kernel are computed once for one (model, window length)
/// and shared safely across windows. The direct [`estimate_joint`] prepares
/// and consumes one plan per call, so both paths execute the same arithmetic
/// under the same gates: reuse changes cost, never values.
#[derive(Debug, Clone)]
pub struct PreparedNoisePlan {
    noise: NoiseModel,
    rows: usize,
    tolerances: JointSolverTolerances,
    correlation: Option<PreparedCorrelation>,
}

/// Factorized normalized Toeplitz factor for one window length, built once
/// and reused by every window that shares the geometry. The executed
/// factor condition `cond(C)` with `C = T + jI` is measured once here with
/// the same symmetric eigendecomposition the per-window TOL-07 gate uses,
/// so every window can certify acceptance with an O(N) scale-ratio bound
/// instead of its own O(N^3) eigendecomposition (LI 10s target).
#[derive(Debug, Clone)]
struct PreparedCorrelation {
    lower: DMatrix<f64>,
    toeplitz: DMatrix<f64>,
    jitter_applied_v2: f64,
    /// 2-norm condition of the executed factor `C = T + jI`.
    correlation_condition: f64,
    /// Maximum `|C|` entry, for the O(N) realized-finiteness guard that
    /// keeps the bound path fail-closed where the exact gate would refuse
    /// an overflowing `R = S C S` realization.
    max_abs_factor: f64,
}

impl PreparedCorrelation {
    /// Conservative TOL-07 bound `cond(S C S) <= cond(S)^2 * cond(C)`.
    /// Returns `None` when the bound cannot be formed (non-finite or
    /// non-positive scales, non-finite arithmetic, or an `R` realization
    /// the exact gate would refuse as overflowing): callers fall back to
    /// the exact per-window gate, preserving its failure surface exactly.
    fn condition_bound(&self, sqrt_variance: &[f64]) -> Option<f64> {
        if sqrt_variance.is_empty() {
            return None;
        }
        let mut smallest = f64::INFINITY;
        let mut largest = 0.0_f64;
        for scale in sqrt_variance.iter() {
            if !scale.is_finite() || *scale <= 0.0 {
                return None;
            }
            smallest = smallest.min(*scale);
            largest = largest.max(*scale);
        }
        if !(smallest > 0.0 && largest > 0.0) {
            return None;
        }
        let scale_condition = largest / smallest;
        if !scale_condition.is_finite() {
            return None;
        }
        let scale_condition_squared = scale_condition * scale_condition;
        if !scale_condition_squared.is_finite() {
            return None;
        }
        // Every `|R_ij| <= max_s^2 * max|C|`: a non-finite product means
        // the exact gate would refuse the overflowing realization, so the
        // bound must not certify acceptance here.
        let realized_magnitude = largest * largest * self.max_abs_factor;
        if !realized_magnitude.is_finite() {
            return None;
        }
        let bound = scale_condition_squared * self.correlation_condition;
        if !bound.is_finite() {
            return None;
        }
        Some(bound)
    }
}

impl PreparedNoisePlan {
    /// Prepares the immutable plan for one noise model and one window
    /// length. Component checks and the correlated factorization happen
    /// here, once, before any window data is touched; a correlated window
    /// above [`MAX_WINDOW_SAMPLES`] is refused before the quadratic factor
    /// is allocated (PN-FR-037).
    pub fn prepare(
        noise: &NoiseModel,
        rows: usize,
        tolerances: JointSolverTolerances,
    ) -> Result<Self> {
        if rows == 0 {
            return Err(AnalysisError::new(
                "dimension_mismatch",
                "prepared noise plan needs at least one sample",
            ));
        }
        if !(tolerances.max_noise_condition.is_finite() && tolerances.max_noise_condition > 0.0) {
            return Err(AnalysisError::new(
                "non_finite_input",
                "max_noise_condition must be positive finite",
            ));
        }
        let correlation = match noise.mode {
            NoiseMode::Identity => None,
            NoiseMode::PhaseDiagonal => {
                let bins = noise.variance_bins.as_ref().ok_or_else(|| {
                    AnalysisError::new(
                        "missing_noise_component",
                        "phase_diagonal mode needs variance bins",
                    )
                })?;
                table_condition_guard(bins, tolerances.max_noise_condition)?;
                None
            }
            NoiseMode::StationaryCorrelated => {
                let kernel = noise.correlation.as_ref().ok_or_else(|| {
                    AnalysisError::new(
                        "missing_noise_component",
                        "stationary_correlated mode needs a correlation kernel",
                    )
                })?;
                validate_correlation_kernel(kernel)?;
                Some(prepare_correlation_factor(
                    kernel,
                    rows,
                    tolerances.max_jitter_v2,
                )?)
            }
            NoiseMode::PhaseCorrelated => {
                let bins = noise.variance_bins.as_ref().ok_or_else(|| {
                    AnalysisError::new(
                        "missing_noise_component",
                        "phase_correlated mode needs variance bins",
                    )
                })?;
                let kernel = noise.correlation.as_ref().ok_or_else(|| {
                    AnalysisError::new(
                        "missing_noise_component",
                        "phase_correlated mode needs a correlation kernel",
                    )
                })?;
                validate_correlation_kernel(kernel)?;
                table_condition_guard(bins, tolerances.max_noise_condition)?;
                Some(prepare_correlation_factor(
                    kernel,
                    rows,
                    tolerances.max_jitter_v2,
                )?)
            }
        };
        Ok(Self {
            noise: noise.clone(),
            rows,
            tolerances,
            correlation,
        })
    }

    /// Window length this plan is prepared for.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Noise mode this plan is prepared for.
    pub fn mode(&self) -> NoiseMode {
        self.noise.mode
    }

    /// The exact noise model the plan was prepared from.
    pub fn noise_model(&self) -> &NoiseModel {
        &self.noise
    }

    /// Bounded jitter actually applied to the normalized Toeplitz diagonal
    /// when the plan was prepared (0 when no correlated factor ran or the
    /// exact factor was positive definite).
    pub fn jitter_applied_v2(&self) -> f64 {
        self.correlation
            .as_ref()
            .map(|prepared| prepared.jitter_applied_v2)
            .unwrap_or(0.0)
    }

    /// Hoisted 2-norm condition of the executed correlation factor
    /// `C = T + jI` (`None` for the uncorrelated modes). Measured once per
    /// plan with the same symmetric eigendecomposition the exact TOL-07
    /// gate uses; the per-window bound is `cond(S)^2` times this value.
    pub fn correlation_condition(&self) -> Option<f64> {
        self.correlation
            .as_ref()
            .map(|prepared| prepared.correlation_condition)
    }

    /// Whitens design and response with the prepared plan (the accelerated
    /// path). Every per-window validation, variance interpolation,
    /// realized-covariance verdict and triangular solve matches the direct
    /// path exactly; only the correlated factor construction and its
    /// TOL-07 condition measurement are reused (an O(N) conservative bound
    /// certifies acceptance, with exact-gate fallback otherwise).
    pub fn whiten(
        &self,
        design: &DMatrix<f64>,
        signal: &[f64],
        times: &[f64],
        reference_frequency_hz: f64,
        reference_phase_rad: f64,
    ) -> Result<WhitenedSystem> {
        if design.nrows() != self.rows {
            return Err(AnalysisError::new(
                "dimension_mismatch",
                format!(
                    "prepared noise plan covers {} rows but the design holds {}",
                    self.rows,
                    design.nrows()
                ),
            ));
        }
        whiten_inner(
            design,
            signal,
            reference_phase_rad,
            reference_frequency_hz,
            times,
            &self.noise,
            self.tolerances,
            self.correlation.as_ref(),
        )
    }
}

/// Prepared or freshly built factor for one correlated window.
enum FactorSource<'a> {
    Prepared(&'a PreparedCorrelation),
    Fresh(PreparedCorrelation),
}

impl FactorSource<'_> {
    fn parts(&self) -> (&DMatrix<f64>, &DMatrix<f64>, f64, &PreparedCorrelation) {
        match self {
            FactorSource::Prepared(prepared) => (
                &prepared.lower,
                &prepared.toeplitz,
                prepared.jitter_applied_v2,
                prepared,
            ),
            FactorSource::Fresh(fresh) => (
                &fresh.lower,
                &fresh.toeplitz,
                fresh.jitter_applied_v2,
                fresh,
            ),
        }
    }
}

/// Builds and factorizes the normalized Toeplitz factor for one window
/// length: recorded lags with zeros beyond the support, SPD-validated under
/// the bounded jitter policy.
fn prepare_correlation_factor(
    kernel: &CorrelationKernel,
    rows: usize,
    max_jitter_v2: f64,
) -> Result<PreparedCorrelation> {
    if rows < 2 {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            "correlated whitening needs at least two samples",
        ));
    }
    if rows > MAX_WINDOW_SAMPLES {
        return Err(AnalysisError::new(
            "resource_limit_exceeded",
            format!(
                "correlated window holds {rows} samples above cap {MAX_WINDOW_SAMPLES}; \
                 refusing the quadratic factor before allocating it"
            ),
        ));
    }
    let toeplitz = toeplitz_from_lags(&kernel.lags, rows);
    let (lower, jitter_applied_v2) = cholesky_factor(&toeplitz, max_jitter_v2)?;
    // Hoisted TOL-07 factor condition: measure cond(C) once with the same
    // symmetric eigendecomposition the per-window gate uses. Cholesky
    // success already proves positive definiteness; a non-finite or
    // non-positive spectrum here fails closed exactly like the gate would.
    let mut executed = toeplitz.clone();
    if jitter_applied_v2 != 0.0 {
        for diagonal in 0..executed.nrows() {
            executed[(diagonal, diagonal)] += jitter_applied_v2;
        }
    }
    let eigenvalues = executed.clone().symmetric_eigen().eigenvalues;
    let mut smallest = f64::INFINITY;
    let mut largest = f64::NEG_INFINITY;
    for value in eigenvalues.iter() {
        smallest = smallest.min(*value);
        largest = largest.max(*value);
    }
    if !(smallest.is_finite() && largest.is_finite()) || smallest <= 0.0 {
        return Err(AnalysisError::new(
            "ill_conditioned_noise_model",
            "hoisted correlation factor is not numerically positive definite; \
             refusing the condition verdict",
        ));
    }
    let correlation_condition = largest / smallest;
    if !correlation_condition.is_finite() {
        return Err(AnalysisError::new(
            "ill_conditioned_noise_model",
            "hoisted correlation factor condition is non-finite; \
             refusing the condition verdict",
        ));
    }
    let mut max_abs_factor = 0.0_f64;
    for value in executed.iter() {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "ill_conditioned_noise_model",
                "hoisted correlation factor is non-finite; \
                 refusing the condition verdict",
            ));
        }
        max_abs_factor = max_abs_factor.max(value.abs());
    }
    Ok(PreparedCorrelation {
        lower,
        toeplitz,
        jitter_applied_v2,
        correlation_condition,
        max_abs_factor,
    })
}

/// Whitening body shared by the direct and prepared paths. `prepared` is
/// `None` for the direct path (every correlated window factorizes its own
/// Toeplitz) and `Some` for the accelerated path.
#[allow(clippy::too_many_arguments)]
fn whiten_inner(
    design: &DMatrix<f64>,
    signal: &[f64],
    reference_phase_rad: f64,
    reference_frequency_hz: f64,
    times: &[f64],
    noise: &NoiseModel,
    tolerances: JointSolverTolerances,
    prepared: Option<&PreparedCorrelation>,
) -> Result<WhitenedSystem> {
    if signal.len() != design.nrows() || times.len() != design.nrows() {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            "signal/time length must match design rows",
        ));
    }
    for value in design.iter() {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                "design matrix must be finite",
            ));
        }
    }
    for (index, value) in signal.iter().enumerate() {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                format!("signal is non-finite at index {index}"),
            ));
        }
    }
    for (index, time) in times.iter().enumerate() {
        if !time.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                format!("time axis is non-finite at sample {index}"),
            ));
        }
    }
    require_finite("reference_phase_rad", reference_phase_rad)?;
    require_finite("reference_frequency_hz", reference_frequency_hz)?;
    if !(tolerances.max_noise_condition.is_finite() && tolerances.max_noise_condition > 0.0) {
        return Err(AnalysisError::new(
            "non_finite_input",
            "max_noise_condition must be positive finite",
        ));
    }
    let response = DVector::from_vec(signal.to_vec());
    let identity_system =
        |design: DMatrix<f64>, response: DVector<f64>, scale: f64| WhitenedSystem {
            design,
            response,
            variance_scale: scale,
            jitter_applied_v2: 0.0,
        };
    match noise.mode {
        NoiseMode::Identity => Ok(identity_system(
            design.clone(),
            response,
            noise.reference_variance_v2,
        )),
        NoiseMode::PhaseDiagonal => {
            let bins = noise.variance_bins.as_ref().ok_or_else(|| {
                AnalysisError::new(
                    "missing_noise_component",
                    "phase_diagonal mode needs variance bins",
                )
            })?;
            table_condition_guard(bins, tolerances.max_noise_condition)?;
            let phases: Vec<f64> = times
                .iter()
                .map(|time| TAU * reference_frequency_hz * time - reference_phase_rad)
                .collect();
            let variances = interpolate_variance(&phases, bins)?;
            for variance in variances.iter() {
                if !variance.is_finite() || *variance <= 0.0 {
                    return Err(AnalysisError::new(
                        "invalid_noise_model",
                        "realized sample variances must be positive finite",
                    ));
                }
            }
            let mut whitened = design.clone();
            let mut whitened_response = response;
            for (row, variance) in variances.iter().enumerate() {
                let weight = 1.0 / variance.sqrt();
                for column in 0..whitened.ncols() {
                    whitened[(row, column)] *= weight;
                }
                whitened_response[row] *= weight;
            }
            Ok(identity_system(whitened, whitened_response, 1.0))
        }
        NoiseMode::StationaryCorrelated => {
            let kernel = noise.correlation.as_ref().ok_or_else(|| {
                AnalysisError::new(
                    "missing_noise_component",
                    "stationary_correlated mode needs a correlation kernel",
                )
            })?;
            // Re-validate contents for direct callers; estimate_joint
            // pre-validates, but this entry point stands on its own.
            validate_correlation_kernel(kernel)?;
            // R = v0 * C_N: variance scaling first, then the C factor. The
            // scale is absorbed into Dw, so the design-model covariance is
            // (Dw^T Dw)^-1 with unit scale (same convention as the
            // phase-diagonal path, where S scaling also precedes the solve).
            let scaled = scale_rows(design, &response, noise.reference_variance_v2.sqrt())?;
            let uniform = vec![noise.reference_variance_v2.sqrt(); scaled.0.nrows()];
            let system = whiten_correlated(
                &scaled.0, &scaled.1, times, kernel, tolerances, prepared, &uniform,
            )?;
            Ok(system)
        }
        NoiseMode::PhaseCorrelated => {
            let bins = noise.variance_bins.as_ref().ok_or_else(|| {
                AnalysisError::new(
                    "missing_noise_component",
                    "phase_correlated mode needs variance bins",
                )
            })?;
            let kernel = noise.correlation.as_ref().ok_or_else(|| {
                AnalysisError::new(
                    "missing_noise_component",
                    "phase_correlated mode needs a correlation kernel",
                )
            })?;
            // Re-validate contents for direct callers (see stationary arm).
            validate_correlation_kernel(kernel)?;
            // Same source-table TOL-07 guard as the diagonal path: an
            // identity kernel must accept exactly when diagonal accepts.
            table_condition_guard(bins, tolerances.max_noise_condition)?;
            // R = S_N C_N S_N with S_ii = sqrt(v(phi_i)): the variance
            // scaling precedes the C triangular solve; reversing the order
            // is a different (wrong) factorization.
            let phases: Vec<f64> = times
                .iter()
                .map(|time| TAU * reference_frequency_hz * time - reference_phase_rad)
                .collect();
            let variances = interpolate_variance(&phases, bins)?;
            let mut scaled_design = design.clone();
            let mut scaled_response = response;
            let mut sqrt_variance = Vec::with_capacity(variances.len());
            for (row, variance) in variances.iter().enumerate() {
                if !variance.is_finite() || *variance <= 0.0 {
                    return Err(AnalysisError::new(
                        "invalid_noise_model",
                        "realized sample variances must be positive finite",
                    ));
                }
                sqrt_variance.push(variance.sqrt());
                let weight = 1.0 / variance.sqrt();
                for column in 0..scaled_design.ncols() {
                    scaled_design[(row, column)] *= weight;
                }
                scaled_response[row] *= weight;
            }
            whiten_correlated(
                &scaled_design,
                &scaled_response,
                times,
                kernel,
                tolerances,
                prepared,
                &sqrt_variance,
            )
        }
    }
}

/// Divide every design row and the response by a positive scalar.
fn scale_rows(
    design: &DMatrix<f64>,
    response: &DVector<f64>,
    divisor: f64,
) -> Result<(DMatrix<f64>, DVector<f64>)> {
    if divisor <= 0.0 || !divisor.is_finite() {
        return Err(AnalysisError::new(
            "invalid_noise_model",
            "variance scale divisor must be positive finite",
        ));
    }
    Ok((design / divisor, response / divisor))
}

/// Correlated whitening shared by both correlated modes (NUMERICS 4.2):
/// form the finite Toeplitz factor from recorded lags (zeros beyond the
/// support), SPD-validate it for this exact window with the bounded jitter
/// policy, gate the executed `R = S C S` covariance against the TOL-07 cap,
/// and triangular-solve the already variance-scaled system. The solve stays
/// inside the current window; no state crosses windows.
fn whiten_correlated(
    scaled_design: &DMatrix<f64>,
    scaled_response: &DVector<f64>,
    times: &[f64],
    kernel: &CorrelationKernel,
    tolerances: JointSolverTolerances,
    prepared: Option<&PreparedCorrelation>,
    sqrt_variance: &[f64],
) -> Result<WhitenedSystem> {
    let rows = scaled_design.nrows();
    if rows < 2 {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            "correlated whitening needs at least two samples",
        ));
    }
    if rows > MAX_WINDOW_SAMPLES {
        return Err(AnalysisError::new(
            "resource_limit_exceeded",
            format!(
                "correlated window holds {rows} samples above cap {MAX_WINDOW_SAMPLES}; \
                 refusing the quadratic factor before allocating it"
            ),
        ));
    }
    // Effective interval is the validated first sample interval (the same
    // value validate_timebase returns): one convention shared by
    // validation, calibration binding, and inference, so a kernel built
    // from the validator output always binds on the validated grid.
    // An endpoint average would be a second, incompatible convention.
    let effective_dt = times[1] - times[0];
    if !effective_dt.is_finite() || effective_dt <= 0.0 {
        return Err(AnalysisError::new(
            "invalid_timebase",
            "correlated whitening needs a positive finite first sample interval",
        ));
    }
    let drift = (kernel.lag_step_s - effective_dt).abs() / effective_dt;
    if !drift.is_finite() || drift > CORRELATION_DT_REL_TOL {
        return Err(AnalysisError::new(
            "model_binding_mismatch",
            format!(
                "correlation lag step {dt:.6e} s disagrees with effective interval \
                 {effective_dt:.6e} s (rel {drift:.3e}); refusing silent re-taper",
                dt = kernel.lag_step_s,
            ),
        ));
    }
    let factor_source = match prepared {
        Some(prepared) => FactorSource::Prepared(prepared),
        None => FactorSource::Fresh(prepare_correlation_factor(
            kernel,
            rows,
            tolerances.max_jitter_v2,
        )?),
    };
    let (lower, toeplitz, jitter, prepared_factor) = factor_source.parts();
    // Hoisted TOL-07 bound (P0): cond(S C S) <= cond(S)^2 * cond(C) with
    // cond(C) measured once per plan. A bound at or under the cap implies
    // the exact gate passes, so acceptance skips the per-window O(N^3)
    // eigendecomposition with bit-identical outputs. Any unformable or
    // over-cap bound falls back to the exact gate, preserving its failure
    // surface exactly (no new rejections, no new acceptances).
    let bound_accepts = prepared_factor
        .condition_bound(sqrt_variance)
        .is_some_and(|bound| bound <= tolerances.max_noise_condition);
    if !bound_accepts {
        // TOL-07 on the executed covariance, not just its factors: Cholesky
        // success and whitened-design conditioning do not bound cond(R).
        gate_realized_noise_condition(
            toeplitz,
            jitter,
            sqrt_variance,
            tolerances.max_noise_condition,
        )?;
    }
    // One packed buffer holds `[design | response]` and is solved in place:
    // the old path built a separate right-hand-side copy plus the solver's
    // own output (two ~417KB allocs at N=2003/P=25). Packing once and
    // solving in place keeps every arithmetic step in the same per-element
    // order, so whitened values are bit-identical with one allocation.
    let columns = scaled_design.ncols();
    let mut solved = DMatrix::zeros(rows, columns + 1);
    for row in 0..rows {
        for column in 0..columns {
            solved[(row, column)] = scaled_design[(row, column)];
        }
        solved[(row, columns)] = scaled_response[row];
    }
    forward_substitute_in_place(lower, &mut solved)?;
    let design = solved.columns(0, columns).into_owned();
    let response = DVector::from_iterator(rows, (0..rows).map(|row| solved[(row, columns)]));
    for value in design.iter().chain(response.iter()) {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_output",
                "correlated whitening produced a non-finite value",
            ));
        }
    }
    Ok(WhitenedSystem {
        design,
        response,
        variance_scale: 1.0,
        jitter_applied_v2: jitter,
    })
}

/// Thin-QR least squares with explicit rank/condition gates (NUMERICS 5.1).
/// Returns `(beta, whitened residual RMS, rank, condition)` with strictly
/// finite outputs; arithmetic overflow reports `non_finite_output`. The
/// returned RMS is pre-standardization: [`estimate_joint`] divides it by the
/// square root of the variance scale before publishing `residual_rms`.
pub fn solve_direct(
    whitened_design: &DMatrix<f64>,
    whitened_signal: &DVector<f64>,
    tolerances: JointSolverTolerances,
) -> Result<(DVector<f64>, f64, usize, f64)> {
    let (beta, residual_rms, rank, condition, _) =
        solve_direct_with_factor(whitened_design, whitened_signal, tolerances)?;
    Ok((beta, residual_rms, rank, condition))
}

/// Thin-QR least-squares solution with its design-model covariance, sharing
/// one R factor: `(beta, residual_rms, rank, condition, covariance_beta)`.
/// See [`solve_direct_and_covariance`].
pub type QrSolveWithCovariance = (DVector<f64>, f64, usize, f64, DMatrix<f64>);

/// Thin-QR least squares that also returns the thin R factor, so the
/// design-model covariance can reuse the exact same factorization instead
/// of running a second QR over the same matrix (see
/// [`solve_direct_and_covariance`]). The SVD rank/condition diagnostic
/// likewise reuses this factor (P3) instead of cloning the `N x P` design
/// a second time: for thin QR (`A = Q1 R1` with orthonormal `Q1`) the
/// singular values of `R1` coincide with the design's in exact arithmetic
/// (measured agreement ~1e-15 relative). Beta, residual, and covariance
/// are bit-identical to the sequential pair; rank is unchanged and the
/// reported condition agrees to ~1e-15 relative (see the P3 test). Gate
/// thresholds, error precedence, and every other value are unchanged
/// while the duplicate `N x P` traffic is gone; two compatibility guards
/// keep the observable admission identical to the design-SVD path without
/// touching thresholds: QR overflow to nonfinite `R` defers to the base
/// design diagnostic (typed `non_finite_output` downstream, never an SVD
/// panic), and acceptances whose gate distance lies within the
/// diagnostic-uncertainty bound below defer to the base diagnostic so
/// configured boundaries admit exactly as before.
/// Arithmetic, gates, and error
/// precedence match [`solve_direct`] step for step; only the duplicate
/// factorization cost is hoisted, never any value.
fn solve_direct_with_factor(
    whitened_design: &DMatrix<f64>,
    whitened_signal: &DVector<f64>,
    tolerances: JointSolverTolerances,
) -> Result<QrSolveWithCovariance> {
    let (rows, columns) = (whitened_design.nrows(), whitened_design.ncols());
    if whitened_signal.len() != rows {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            "whitened signal length must match design rows",
        ));
    }
    if rows < columns || columns == 0 {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            "whitened design needs at least as many rows as columns",
        ));
    }
    for value in whitened_design.iter().chain(whitened_signal.iter()) {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                "whitened design and signal must be finite before factorization",
            ));
        }
    }
    if tolerances.rank_tol <= 0.0 || !tolerances.rank_tol.is_finite() {
        return Err(AnalysisError::new(
            "non_finite_input",
            "rank_tol must be positive finite",
        ));
    }
    if tolerances.max_condition <= 0.0 || !tolerances.max_condition.is_finite() {
        return Err(AnalysisError::new(
            "non_finite_input",
            "max_condition must be positive finite",
        ));
    }
    // Solution through thin QR: c = Q^T y, then back-substitute R1 x = c.
    // The factorization below runs on the single N x P clone. The SVD
    // diagnostic reuses its thin R factor (P3) instead of cloning the
    // design a second time; the gate checks keep their original order, so
    // error precedence is unchanged.
    //
    // Compatibility guards (thresholds untouched):
    // - F1: QR arithmetic can overflow finite input to a nonfinite R, and
    //   nalgebra's SVD panics on nonfinite input. The finiteness gate above
    //   proves the design finite, not the factor, so check `upper` before
    //   any SVD and defer overflow cases to the base design diagnostic.
    //   The subsequent back-substitution/residual gates then report the
    //   same typed `non_finite_output` the base path reports.
    // - F2: thin-R singulars drift versus the design SVD, which can flip a
    //   configured `max_condition`/`rank_tol` boundary. Any rejection is
    //   re-decided with the exact base diagnostic (`svd(false, false)` on
    //   the whitened design). An acceptance is re-decided too whenever the
    //   gate distance lies within the diagnostic-uncertainty bound derived
    //   below, where the base verdict could differ. Clear acceptances keep
    //   the P3 saving; only rare borderline/error paths pay the extra N x P
    //   clone.
    let qr = whitened_design.clone().qr();
    let projected = qr.q().tr_mul(whitened_signal);
    let upper = qr.r();
    // Base design-SVD gates, bit-for-bit the pre-reuse diagnostic. Used for
    // F1 overflow and for F2 borderline/rejection verification.
    let base_gates = |singular: &nalgebra::DVector<f64>| -> Result<(usize, f64)> {
        let sigma_max = singular[0];
        if !(sigma_max.is_finite() && sigma_max > 0.0) {
            return Err(AnalysisError::new(
                "rank_deficient_design",
                "whitened design has no finite nonzero singular value",
            ));
        }
        let mut rank = 0;
        for value in singular.iter() {
            if *value > tolerances.rank_tol * sigma_max {
                rank += 1;
            }
        }
        if rank < columns {
            return Err(AnalysisError::new(
                "rank_deficient_design",
                format!("whitened design rank {rank} below {columns} columns"),
            ));
        }
        let sigma_min = singular[columns - 1];
        let condition = sigma_max / sigma_min;
        if !(condition.is_finite()) || condition > tolerances.max_condition {
            return Err(AnalysisError::new(
                "ill_conditioned_design",
                format!("scaled-design condition {condition:.6e} exceeds limit"),
            ));
        }
        Ok((rank, condition))
    };
    // Diagnostic-uncertainty bound (F2). Householder QR followed by the
    // bidiagonal-QR SVD each carry backward error `||dA|| <= c(m,n) eps
    // ||A||` with a small polynomial `c`, so by Weyl each computed
    // singular value lies within `c eps sigma_max` of truth -- for both
    // the thin-R chain and the design-SVD reference. The absolute
    // tolerance below, `64 eps max(m,n) sigma_max`, exceeds textbook
    // constants (order a few x m n on these shapes) by more than an order
    // of magnitude and additionally covers the computed-U versus
    // values-only few-ULP implementation drift (see NOTE #297 above).
    // Gate distances are measured against it, never against a fixed
    // relative band: the condition uncertainty scales with the condition
    // itself (the review's `~8.5e4` fixture needs `~2.4e-8` while its real
    // drift is `4.5e-12`), whereas a fixed band either misses such gates
    // or needlessly punts well-conditioned solves to the base path.
    let gate_dim = rows.max(columns) as f64;
    let (rank, condition) = if !upper.iter().all(|v| v.is_finite()) {
        // F1: overflowed factor; run base gates, then fall through to the
        // shared back-substitution/residual path which yields the base
        // typed `non_finite_output`.
        let svd_base = whitened_design.clone().svd(false, false);
        base_gates(&svd_base.singular_values)?
    } else {
        // Rank and conditioning from the SVD diagnostic on the thin R factor.
        // In exact arithmetic these singular values are the design's
        // (A = Q1 R1 with orthonormal Q1); in floating point they agree to
        // ~1e-15 relative. `upper` is still borrowed below by
        // the back-substitution, hence the P x P clone (negligible traffic).
        // NOTE (#297 convergence): the U factor stays computed (`svd(true,
        // false)`): pinned nalgebra 0.35.0 skips the 2x2 normalization for
        // values-only SVD, shifting singulars a few ULP and gate decisions, so
        // `svd(false, false)` is not adopted here. P3 already subsumes P2's
        // N x P U saving on this path (the diagnostic runs on P x P `upper`).
        let svd = upper.clone().svd(true, false);
        let singular = svd.singular_values;
        let sigma_max = singular[0];
        // Absolute singular tolerance from the uncertainty bound above.
        let sigma_abs_tol = 64.0 * f64::EPSILON * gate_dim * sigma_max;
        // Fast-path gate values; rejections and borderline acceptances are
        // verified against the base diagnostic below.
        let mut rank_r = 0;
        let sigma_max_ok = sigma_max.is_finite() && sigma_max > 0.0;
        if sigma_max_ok {
            for value in singular.iter() {
                if *value > tolerances.rank_tol * sigma_max {
                    rank_r += 1;
                }
            }
        }
        let sigma_min_r = singular[columns - 1];
        let condition_r = sigma_max / sigma_min_r;
        let rejects_r = !sigma_max_ok
            || rank_r < columns
            || !(condition_r.is_finite())
            || condition_r > tolerances.max_condition;
        // Borderline acceptance: within diagnostic uncertainty of either
        // gate, where the base verdict could differ. Rejections are exact
        // by construction (always re-decided); here `condition_r` is finite
        // with `condition_r <= limit` and every singular exceeds its
        // threshold, so the one-sided gaps below are non-negative.
        let mut borderline = false;
        if !rejects_r {
            // Condition: `(limit - cond_R) / limit` against the linearized
            // quotient uncertainty `4 sigma_abs_tol / sigma_min_R`
            // (sigma_min error dominates, plus sigma_max-proxy slack).
            let cond_gap = (tolerances.max_condition - condition_r) / tolerances.max_condition;
            if cond_gap <= 4.0 * sigma_abs_tol / sigma_min_r {
                borderline = true;
            } else {
                // Rank: any singular within `2 sigma_abs_tol` above the
                // `rank_tol sigma_max` threshold (factor 2 for the
                // threshold's own sigma_max-proxy error).
                let threshold_r = tolerances.rank_tol * sigma_max;
                for value in singular.iter() {
                    if *value - threshold_r <= 2.0 * sigma_abs_tol {
                        borderline = true;
                        break;
                    }
                }
            }
        }
        if rejects_r || borderline {
            // F2: re-decide with the exact base diagnostic so configured
            // boundaries admit exactly as before. Thresholds unchanged.
            let svd_base = whitened_design.clone().svd(false, false);
            base_gates(&svd_base.singular_values)?
        } else {
            // Clear acceptance far from any boundary: keep the reused
            // diagnostic values (agree with base to ~1e-15, well inside the
            // 1e-12 golden band).
            if !sigma_max_ok {
                return Err(AnalysisError::new(
                    "rank_deficient_design",
                    "whitened design has no finite nonzero singular value",
                ));
            }
            if rank_r < columns {
                return Err(AnalysisError::new(
                    "rank_deficient_design",
                    format!("whitened design rank {rank_r} below {columns} columns"),
                ));
            }
            if !(condition_r.is_finite()) || condition_r > tolerances.max_condition {
                return Err(AnalysisError::new(
                    "ill_conditioned_design",
                    format!("scaled-design condition {condition_r:.6e} exceeds limit"),
                ));
            }
            (rank_r, condition_r)
        }
    };
    // Back-substitution R1 x = c on the same factor the diagnostic above
    // already reused; `projected` and `upper` were computed before the
    // gates but only read here, after them.
    let mut beta = DVector::zeros(columns);
    for column in (0..columns).rev() {
        let mut accumulator = projected[column];
        for row in (column + 1)..columns {
            accumulator -= upper[(column, row)] * beta[row];
        }
        let diagonal = upper[(column, column)];
        if diagonal == 0.0 {
            return Err(AnalysisError::new(
                "rank_deficient_design",
                format!("zero QR diagonal at column {column}"),
            ));
        }
        beta[column] = accumulator / diagonal;
    }
    let residual = whitened_signal - whitened_design * &beta;
    // Scale-safe RMS: normalize by the largest magnitude first so a
    // representable RMS never overflows through norm_squared.
    let mut scale = 0.0_f64;
    for value in residual.iter() {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_output",
                "solver residual is non-finite",
            ));
        }
        scale = scale.max(value.abs());
    }
    for value in beta.iter() {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_output",
                "solver coefficients are non-finite",
            ));
        }
    }
    let residual_rms = if scale == 0.0 {
        0.0
    } else {
        let scaled: f64 = residual.iter().map(|value| (value / scale).powi(2)).sum();
        if !scaled.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_output",
                "residual sum of squares is non-finite",
            ));
        }
        scale * (scaled / rows as f64).sqrt()
    };
    if !residual_rms.is_finite() {
        return Err(AnalysisError::new(
            "non_finite_output",
            "residual RMS is non-finite",
        ));
    }
    Ok((beta, residual_rms, rank, condition, upper))
}

/// Design-model covariance `(Dw^T Dw)^-1` from the whitened thin R factor.
/// The triangular factor is applied through substitution against identity
/// (never a general inverse call); the result is still an inverse-derived
/// quantity and is labeled as such. All inputs and outputs are validated.
pub fn covariance_from_qr(
    whitened_design: &DMatrix<f64>,
    variance_scale: f64,
) -> Result<DMatrix<f64>> {
    let (rows, columns) = (whitened_design.nrows(), whitened_design.ncols());
    if rows < columns || columns == 0 {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            "covariance needs at least as many rows as columns",
        ));
    }
    for value in whitened_design.iter() {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                "whitened design must be finite",
            ));
        }
    }
    if !variance_scale.is_finite() || variance_scale <= 0.0 {
        return Err(AnalysisError::new(
            "invalid_noise_model",
            "covariance variance scale must be positive finite",
        ));
    }
    let upper = whitened_design.clone().qr().r();
    covariance_from_factor(&upper, columns, variance_scale)
}

/// Design-model covariance from an already-computed thin R factor: solves
/// `R1^T Z = I` for `Z`, then `C = Z^T Z`, scaled. The substitution
/// sequence matches [`covariance_from_qr`] exactly; callers must have run
/// the same dimension/finiteness/scale gates first (they do: both
/// [`covariance_from_qr`] above and [`solve_direct_and_covariance`] below
/// validate before substituting).
fn covariance_from_factor(
    upper: &DMatrix<f64>,
    columns: usize,
    variance_scale: f64,
) -> Result<DMatrix<f64>> {
    // Solve R1^T Z = I for Z, then C = Z^T Z, scaled.
    let mut inverse_transpose = DMatrix::zeros(columns, columns);
    for column in 0..columns {
        // Forward substitution on R1^T (lower triangular).
        for row in 0..columns {
            let mut accumulator = if row == column { 1.0 } else { 0.0 };
            for inner in 0..row {
                accumulator -= upper[(inner, row)] * inverse_transpose[(inner, column)];
            }
            let diagonal = upper[(row, row)];
            if diagonal == 0.0 {
                return Err(AnalysisError::new(
                    "rank_deficient_design",
                    format!("zero QR diagonal at column {column}"),
                ));
            }
            inverse_transpose[(row, column)] = accumulator / diagonal;
        }
    }
    let covariance = variance_scale * (&inverse_transpose.transpose() * &inverse_transpose);
    if covariance.iter().all(|value| value.is_finite()) {
        Ok(covariance)
    } else {
        Err(AnalysisError::new(
            "non_finite_output",
            "design-model covariance is non-finite",
        ))
    }
}

/// Combined thin-QR solve and design-model covariance sharing one R factor
/// (NUMERICS 5.1, per-window duplicate-factorization removal). Runs the
/// [`solve_direct`] gates and back-substitution, then the
/// [`covariance_from_qr`] scale gate and forward substitution on the exact
/// same factor object — so every output is bit-identical to running the two
/// entry points in sequence, while the second `O(N p^2)` QR factorization
/// and its matrix clone are gone (and the SVD diagnostic now also reuses
/// that factor instead of cloning the design again — see
/// [`solve_direct`]). Returns
/// `(beta, residual_rms, rank, condition, covariance_beta)`.
pub fn solve_direct_and_covariance(
    whitened_design: &DMatrix<f64>,
    whitened_signal: &DVector<f64>,
    tolerances: JointSolverTolerances,
    variance_scale: f64,
) -> Result<QrSolveWithCovariance> {
    let (beta, residual_rms, rank, condition, upper) =
        solve_direct_with_factor(whitened_design, whitened_signal, tolerances)?;
    // Same gate covariance_from_qr runs after its own factorization; the
    // solve above already ran the shared dimension/finiteness gates first,
    // preserving the sequential error precedence exactly.
    if !variance_scale.is_finite() || variance_scale <= 0.0 {
        return Err(AnalysisError::new(
            "invalid_noise_model",
            "covariance variance scale must be positive finite",
        ));
    }
    let covariance_beta = covariance_from_factor(&upper, whitened_design.ncols(), variance_scale)?;
    Ok((beta, residual_rms, rank, condition, covariance_beta))
}

/// Peak-amplitude map `Xk = b`, `Yk = a` in `[X1,Y1,...]` order.
pub fn map_to_xy(beta: &[f64], model: &HarmonicSignalModel) -> Result<Vec<f64>> {
    if beta.len() != 1 + 2 * model.fit_harmonics.len() {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            "beta length must match the design columns",
        ));
    }
    for (index, value) in beta.iter().enumerate() {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                format!("coefficients are non-finite at index {index}"),
            ));
        }
    }
    let mut xy = Vec::with_capacity(2 * model.output_harmonics.len());
    for output in &model.output_harmonics {
        let position = model
            .fit_harmonics
            .iter()
            .position(|h| h == output)
            .ok_or_else(|| {
                AnalysisError::new(
                    "invalid_harmonics",
                    format!("output harmonic {output} is not in the fitting list"),
                )
            })?;
        xy.push(beta[2 + 2 * position]);
        xy.push(beta[1 + 2 * position]);
    }
    Ok(xy)
}

/// Map beta-space covariance to XY space without rescaling (AT-022):
/// XY now carries peak amplitudes, so the mapper only reorders entries.
pub fn map_covariance_to_xy(
    covariance_beta: &DMatrix<f64>,
    model: &HarmonicSignalModel,
) -> Result<DMatrix<f64>> {
    let parameters = 1 + 2 * model.fit_harmonics.len();
    if covariance_beta.nrows() != parameters || covariance_beta.ncols() != parameters {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            "beta covariance shape must match the design columns",
        ));
    }
    for value in covariance_beta.iter() {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                "beta covariance must be finite",
            ));
        }
    }
    let mut rows = Vec::with_capacity(2 * model.output_harmonics.len());
    for output in &model.output_harmonics {
        let position = model
            .fit_harmonics
            .iter()
            .position(|h| h == output)
            .ok_or_else(|| {
                AnalysisError::new(
                    "invalid_harmonics",
                    format!("output harmonic {output} is not in the fitting list"),
                )
            })?;
        // XY order is [Xk, Yk] = [b, a]: select sin then cos rows/cols.
        rows.push(2 + 2 * position);
        rows.push(1 + 2 * position);
    }
    let dimension = rows.len();
    let mut xy = DMatrix::zeros(dimension, dimension);
    for (i, row) in rows.iter().enumerate() {
        for (j, column) in rows.iter().enumerate() {
            xy[(i, j)] = covariance_beta[(*row, *column)];
        }
    }
    Ok(xy)
}

/// Rotate XY covariance by per-harmonic phase deltas using the same
/// block-diagonal rotation as the phase transform:
/// `in = X c + Y s`, `out = -X s + Y c` (AT-022).
pub fn rotate_xy_covariance(
    covariance_xy: &DMatrix<f64>,
    deltas_rad: &[f64; 6],
) -> Result<DMatrix<f64>> {
    if covariance_xy.nrows() != 12 || covariance_xy.ncols() != 12 {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            "XY covariance rotation needs the 12 quadrature entries",
        ));
    }
    for delta in deltas_rad.iter() {
        require_finite("rotation delta", *delta)?;
    }
    for value in covariance_xy.iter() {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_input",
                "XY covariance must be finite",
            ));
        }
    }
    let mut rotated = DMatrix::zeros(12, 12);
    for (ka, delta_a) in deltas_rad.iter().enumerate() {
        let (sa, ca) = delta_a.sin_cos();
        for (kb, delta_b) in deltas_rad.iter().enumerate() {
            let (sb, cb) = delta_b.sin_cos();
            // Block rotation R(ka) C R(kb)^T with R = [[c, s], [-s, c]].
            let rotation = [
                [ca * cb, ca * sb, sa * cb, sa * sb],
                [-ca * sb, ca * cb, -sa * sb, sa * cb],
                [-sa * cb, -sa * sb, ca * cb, ca * sb],
                [sa * sb, -sa * cb, -ca * sb, ca * cb],
            ];
            for (i, &row) in [2 * ka, 2 * ka + 1].iter().enumerate() {
                for (j, &column) in [2 * kb, 2 * kb + 1].iter().enumerate() {
                    // C'[2ka+i, 2kb+j] = sum_pq R[i,p] C[2ka+p, 2kb+q] R[j,q].
                    let mut accumulator = 0.0;
                    for (p, &source_row) in [2 * ka, 2 * ka + 1].iter().enumerate() {
                        for (q, &source_column) in [2 * kb, 2 * kb + 1].iter().enumerate() {
                            accumulator += rotation[i * 2 + j][p * 2 + q]
                                * covariance_xy[(source_row, source_column)];
                        }
                    }
                    rotated[(row, column)] = accumulator;
                }
            }
        }
    }
    Ok(rotated)
}

/// Upper-triangle packing in lexicographic `(row, column)`, `row <= column`.
pub fn pack_upper_triangle(covariance: &DMatrix<f64>) -> Result<Vec<f64>> {
    if covariance.nrows() != covariance.ncols() {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            "covariance packing needs a square matrix",
        ));
    }
    let dimension = covariance.nrows();
    let mut packed = Vec::with_capacity(dimension * (dimension + 1) / 2);
    for row in 0..dimension {
        for column in row..dimension {
            let value = covariance[(row, column)];
            if !value.is_finite() {
                return Err(AnalysisError::new(
                    "non_finite_input",
                    "covariance packing needs finite entries",
                ));
            }
            packed.push(value);
        }
    }
    Ok(packed)
}

/// Full direct estimate on one window: validate, build, whiten, solve, map.
///
/// `residual_rms` is standardized per-unit-noise: the whitened RMS divided
/// by the square root of the variance scale, so identity and diagonal modes
/// report identical values for identical covariances.
///
/// This entry point is the direct numerical reference used by tests and
/// comparisons. Production preflight (workspace and model-byte budgets)
/// belongs to the plan/application layer, not here; the correlated arms
/// additionally refuse windows above MAX_WINDOW_SAMPLES before allocating
/// the quadratic factor. All joint items are provisional WP-2/WP-4
/// interfaces; the backend-private typed-API boundary (NFR-014) is decided
/// before WP-3/WP-5 consumers depend on them.
pub fn estimate_joint(
    times: &[f64],
    signal: &[f64],
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    sample_rate_hz: f64,
    settings: &JointHarmonicSettings,
) -> Result<JointEstimate> {
    validate_signal_model(&settings.model, sample_rate_hz)?;
    validate_noise_model(&settings.noise)?;
    let plan = PreparedNoisePlan::prepare(&settings.noise, times.len(), settings.tolerances)?;
    estimate_joint_with_plan(
        times,
        signal,
        reference_frequency_hz,
        reference_phase_rad,
        sample_rate_hz,
        &settings.model,
        settings.tolerances,
        &plan,
    )
}

/// Direct-estimate entry point on an immutable prepared noise plan (the
/// accelerated path). `estimate_joint` prepares and consumes one plan per
/// call, so both paths share every arithmetic step; reuse across windows is
/// a cost optimization proven by equivalence tests, never a different
/// estimator. The plan's noise model is re-validated, so the acceptance
/// surface matches the direct entry point exactly.
#[allow(clippy::too_many_arguments)]
pub fn estimate_joint_with_plan(
    times: &[f64],
    signal: &[f64],
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    sample_rate_hz: f64,
    model: &HarmonicSignalModel,
    tolerances: JointSolverTolerances,
    plan: &PreparedNoisePlan,
) -> Result<JointEstimate> {
    let noise = plan.noise_model();
    validate_signal_model(model, sample_rate_hz)?;
    validate_noise_model(noise)?;
    if plan.rows() != times.len() {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            format!(
                "prepared noise plan covers {} samples but the window holds {}",
                plan.rows(),
                times.len()
            ),
        ));
    }
    let design = design_matrix(
        times,
        reference_frequency_hz,
        reference_phase_rad,
        model,
        sample_rate_hz,
    )?;
    let system = plan.whiten(
        &design,
        signal,
        times,
        reference_frequency_hz,
        reference_phase_rad,
    )?;
    let whitened_design = system.design;
    let whitened_signal = system.response;
    let variance_scale = system.variance_scale;
    // Shared-R solve+covariance: bit-identical to the sequential pair, one
    // factorization fewer. The variance-scale guard below is unreachable
    // through whitening (every arm emits a validated positive scale) and is
    // kept as defense-in-depth in its original position.
    let (beta, whitened_rms, rank, condition, covariance_beta) = solve_direct_and_covariance(
        &whitened_design,
        &whitened_signal,
        tolerances,
        variance_scale,
    )?;
    if variance_scale <= 0.0 {
        return Err(AnalysisError::new(
            "non_finite_output",
            "variance scale is non-positive",
        ));
    }
    let residual_rms = whitened_rms / variance_scale.sqrt();
    let beta_vec = beta.as_slice().to_vec();
    let covariance_xy = map_covariance_to_xy(&covariance_beta, model)?;
    let xy = map_to_xy(&beta_vec, model)?;
    if !residual_rms.is_finite() {
        return Err(AnalysisError::new(
            "non_finite_output",
            "standardized residual RMS is non-finite",
        ));
    }
    Ok(JointEstimate {
        beta: beta_vec,
        xy,
        covariance_beta: covariance_beta
            .row_iter()
            .map(|row| row.iter().copied().collect())
            .collect(),
        covariance_xy: covariance_xy
            .row_iter()
            .map(|row| row.iter().copied().collect())
            .collect(),
        rank,
        condition,
        residual_rms,
        jitter_applied_v2: system.jitter_applied_v2,
    })
}

/// Reusable per-window workspace for the buffer-reusing estimate path.
/// Holds the design-matrix buffer across windows so the hot loop allocates
/// it once per thread instead of once per window. A scratch must never be
/// shared across threads: give each worker its own (the native pipeline
/// keeps one per rayon thread via a thread-local).
#[derive(Debug, Clone)]
pub struct JointScratch {
    design: DMatrix<f64>,
}

impl JointScratch {
    /// Empty workspace; the buffer grows to the first window it serves.
    pub fn new() -> Self {
        Self {
            design: DMatrix::zeros(0, 0),
        }
    }

    fn design_for(&mut self, rows: usize, parameters: usize) -> &mut DMatrix<f64> {
        if self.design.nrows() != rows || self.design.ncols() != parameters {
            self.design = DMatrix::zeros(rows, parameters);
        }
        &mut self.design
    }
}

impl Default for JointScratch {
    fn default() -> Self {
        Self::new()
    }
}

/// Buffer-reusing estimate on an immutable prepared noise plan: the design
/// matrix fills the caller's [`JointScratch`] instead of allocating
/// per-window column vectors, and the solve shares one QR factor with the
/// covariance. Every validation, gate, and arithmetic step matches
/// [`estimate_joint_with_plan`] exactly, so outputs are bit-identical;
/// only allocation and duplicate-factorization cost is hoisted.
#[allow(clippy::too_many_arguments)]
pub fn estimate_joint_with_scratch(
    times: &[f64],
    signal: &[f64],
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    sample_rate_hz: f64,
    model: &HarmonicSignalModel,
    tolerances: JointSolverTolerances,
    plan: &PreparedNoisePlan,
    scratch: &mut JointScratch,
) -> Result<JointEstimate> {
    let noise = plan.noise_model();
    validate_signal_model(model, sample_rate_hz)?;
    validate_noise_model(noise)?;
    if plan.rows() != times.len() {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            format!(
                "prepared noise plan covers {} samples but the window holds {}",
                plan.rows(),
                times.len()
            ),
        ));
    }
    let parameters = 1 + 2 * model.fit_harmonics.len();
    let design = scratch.design_for(times.len(), parameters);
    design_matrix_into(
        design,
        times,
        reference_frequency_hz,
        reference_phase_rad,
        model,
        sample_rate_hz,
    )?;
    let system = plan.whiten(
        design,
        signal,
        times,
        reference_frequency_hz,
        reference_phase_rad,
    )?;
    let whitened_design = system.design;
    let whitened_signal = system.response;
    let variance_scale = system.variance_scale;
    // Shared-R solve+covariance (same notes as estimate_joint_with_plan).
    let (beta, whitened_rms, rank, condition, covariance_beta) = solve_direct_and_covariance(
        &whitened_design,
        &whitened_signal,
        tolerances,
        variance_scale,
    )?;
    if variance_scale <= 0.0 {
        return Err(AnalysisError::new(
            "non_finite_output",
            "variance scale is non-positive",
        ));
    }
    let residual_rms = whitened_rms / variance_scale.sqrt();
    let beta_vec = beta.as_slice().to_vec();
    let covariance_xy = map_covariance_to_xy(&covariance_beta, model)?;
    let xy = map_to_xy(&beta_vec, model)?;
    if !residual_rms.is_finite() {
        return Err(AnalysisError::new(
            "non_finite_output",
            "standardized residual RMS is non-finite",
        ));
    }
    Ok(JointEstimate {
        beta: beta_vec,
        xy,
        covariance_beta: covariance_beta
            .row_iter()
            .map(|row| row.iter().copied().collect())
            .collect(),
        covariance_xy: covariance_xy
            .row_iter()
            .map(|row| row.iter().copied().collect())
            .collect(),
        rank,
        condition,
        residual_rms,
        jitter_applied_v2: system.jitter_applied_v2,
    })
}

/// Dense-slice flavor of [`rotate_xy_covariance`] for nalgebra-free callers
/// (native pipeline, WASM boundary): accepts a row-major 12x12 slice and
/// returns the rotated row-major 12x12 entries. Fail-closed on shape and
/// finiteness exactly like the matrix entry point.
pub fn rotate_xy_covariance_dense(
    covariance_xy_row_major: &[f64],
    deltas_rad: &[f64; 6],
) -> Result<Vec<f64>> {
    if covariance_xy_row_major.len() != 144 {
        return Err(AnalysisError::new(
            "dimension_mismatch",
            "XY covariance rotation needs the 12 quadrature entries",
        ));
    }
    let matrix = DMatrix::from_row_slice(12, 12, covariance_xy_row_major);
    let rotated = rotate_xy_covariance(&matrix, deltas_rad)?;
    Ok(rotated.iter().copied().collect())
}
