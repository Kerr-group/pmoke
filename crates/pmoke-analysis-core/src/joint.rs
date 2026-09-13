//! Fixed-support joint harmonic estimator core (WP-2, milestone M2).
//!
//! Direct finite-window reference: Householder QR solution of the whitened
//! design with explicit rank/condition checks, plus design-model covariance.
//! Identity and phase-diagonal noise modes are executable; correlated modes
//! report explicit unsupported errors (staged delivery, A-001). Coordinate
//! conventions follow NUMERICS sections 1-2: original-sample support,
//! `phi = 2 pi f t - phase`, column order `[DC, cos, sin, ...]`, and the
//! legacy half-amplitude map `Xk = b/2`, `Yk = a/2`.
//!
//! Backend: nalgebra 0.35.0, no-default plus `std` (A-010 spike). The
//! rectangular solve uses only public APIs: thin `Q`, `Q^T y`, and manual
//! back-substitution on the upper-trapezoidal `R`. Rank and conditioning
//! come from an SVD diagnostic on the same whitened design; the independent
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

/// Executable noise-model selector. Correlated variants are accepted types
/// with explicit unsupported errors until the WP-3 calibration slice lands.
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
    /// Required for phase-diagonal mode; `None` otherwise.
    pub variance_bins: Option<Vec<f64>>,
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
        }
    }
}

/// Validate a coherent monotonic uniform timebase and return the effective
/// interval, mirroring the recorded waveform preflight: strict increase,
/// per-step uniformity within relative tolerance plus serialization
/// roundoff, and consistency with the declared sample rate
/// (INTERFACES `invalid_timebase`).
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
        let roundoff = times[index].abs().max(times[index - 1].abs()) * f64::EPSILON * 16.0;
        let tolerance = (interval.abs() * TIMEBASE_RELATIVE_TOLERANCE).max(roundoff);
        if !step.is_finite() || (step - interval).abs() > tolerance {
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
    /// Residual RMS in whitened units.
    pub residual_rms: f64,
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
        (NoiseMode::PhaseDiagonal, Some(bins)) => {
            if bins.len() < 2 {
                return Err(AnalysisError::new(
                    "invalid_noise_model",
                    "phase_diagonal needs at least two variance bins",
                ));
            }
            if !bins.iter().all(|v| v.is_finite() && *v > 0.0) {
                return Err(AnalysisError::new(
                    "invalid_noise_model",
                    "variance bins must be positive finite",
                ));
            }
        }
        (NoiseMode::PhaseDiagonal, None) => {
            return Err(AnalysisError::new(
                "missing_noise_component",
                "phase_diagonal mode needs variance bins",
            ));
        }
        (_, Some(_)) => {
            return Err(AnalysisError::new(
                "invalid_noise_model",
                "variance bins are only valid for phase_diagonal mode",
            ));
        }
        _ => {}
    }
    Ok(())
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
    let mut columns: Vec<Vec<f64>> = Vec::with_capacity(1 + 2 * model.fit_harmonics.len());
    columns.push(vec![1.0; times.len()]);
    for harmonic in &model.fit_harmonics {
        let frequency = *harmonic as f64 * reference_frequency_hz;
        if !frequency.is_finite() || frequency >= nyquist {
            return Err(AnalysisError::new(
                "aliased_harmonic",
                format!("harmonic {harmonic} reaches Nyquist at {nyquist:.6e} Hz"),
            ));
        }
        let mut cos_column = Vec::with_capacity(times.len());
        let mut sin_column = Vec::with_capacity(times.len());
        for time in times {
            let phi = TAU * reference_frequency_hz * time - reference_phase_rad;
            let (sin_phi, cos_phi) = (*harmonic as f64 * phi).sin_cos();
            cos_column.push(cos_phi);
            sin_column.push(sin_phi);
        }
        columns.push(cos_column);
        columns.push(sin_column);
    }
    Ok(DMatrix::from_columns(
        &columns
            .iter()
            .map(|column| DVector::from_vec(column.clone()))
            .collect::<Vec<_>>(),
    ))
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

/// Whiten design and response together. Returns `(Dw, yw, variance_scale)`
/// where the design-model covariance is `variance_scale * (Dw^T Dw)^-1`.
/// For phase-diagonal mode the variance-table condition is gated against
/// the TOL-07 policy, independently of design conditioning. Interpolated
/// sample variances are convex combinations of table entries, so gating
/// the table bounds the realized covariance.
pub fn whiten(
    design: &DMatrix<f64>,
    signal: &[f64],
    reference_phase_rad: f64,
    reference_frequency_hz: f64,
    times: &[f64],
    noise: &NoiseModel,
    tolerances: JointSolverTolerances,
) -> Result<(DMatrix<f64>, DVector<f64>, f64)> {
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
    match noise.mode {
        NoiseMode::Identity => Ok((design.clone(), response, noise.reference_variance_v2)),
        NoiseMode::PhaseDiagonal => {
            let bins = noise.variance_bins.as_ref().ok_or_else(|| {
                AnalysisError::new(
                    "missing_noise_component",
                    "phase_diagonal mode needs variance bins",
                )
            })?;
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
            }
            let mut smallest = f64::INFINITY;
            let mut largest = 0.0_f64;
            for bin in bins.iter() {
                smallest = smallest.min(*bin);
                largest = largest.max(*bin);
            }
            if largest > tolerances.max_noise_condition * smallest {
                return Err(AnalysisError::new(
                    "ill_conditioned_noise_model",
                    format!(
                        "noise variance table condition {largest:.6e}/{smallest:.6e} exceeds cap {:.6e}",
                        tolerances.max_noise_condition
                    ),
                ));
            }
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
            Ok((whitened, whitened_response, 1.0))
        }
        NoiseMode::StationaryCorrelated | NoiseMode::PhaseCorrelated => Err(AnalysisError::new(
            "unsupported_noise_mode",
            "correlated modes arrive with the WP-3 calibration slice",
        )),
    }
}

/// Thin-QR least squares with explicit rank/condition gates (NUMERICS 5.1).
/// Returns `(beta, residual_rms, rank, condition)` with strictly finite
/// outputs; arithmetic overflow reports `non_finite_output`.
pub fn solve_direct(
    whitened_design: &DMatrix<f64>,
    whitened_signal: &DVector<f64>,
    tolerances: JointSolverTolerances,
) -> Result<(DVector<f64>, f64, usize, f64)> {
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
    // Rank and conditioning from the SVD diagnostic on the same matrix.
    let svd = whitened_design.clone().svd(true, false);
    let singular = svd.singular_values;
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
    // Solution through thin QR: c = Q^T y, then back-substitute R1 x = c.
    let qr = whitened_design.clone().qr();
    let projected = qr.q().tr_mul(whitened_signal);
    let upper = qr.r();
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
    Ok((beta, residual_rms, rank, condition))
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

/// Legacy half-amplitude map `Xk = b/2`, `Yk = a/2` in `[X1,Y1,...]` order.
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
        xy.push(beta[2 + 2 * position] / 2.0);
        xy.push(beta[1 + 2 * position] / 2.0);
    }
    Ok(xy)
}

/// Map beta-space covariance to XY space with 1/4 scaling (AT-022).
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
        // XY order is [Xk, Yk] = [b/2, a/2]: select sin then cos rows/cols.
        rows.push(2 + 2 * position);
        rows.push(1 + 2 * position);
    }
    let dimension = rows.len();
    let mut xy = DMatrix::zeros(dimension, dimension);
    for (i, row) in rows.iter().enumerate() {
        for (j, column) in rows.iter().enumerate() {
            xy[(i, j)] = covariance_beta[(*row, *column)] / 4.0;
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
/// This entry point is the uncapped numerical reference used by tests and
/// comparisons. Bounded production preflight (support caps, workspace and
/// model-byte budgets) belongs to the plan/application layer, not here.
/// All joint items are provisional WP-2 interfaces; the backend-private
/// typed-API boundary (NFR-014) is decided before WP-3/WP-5 consumers
/// depend on them.
pub fn estimate_joint(
    times: &[f64],
    signal: &[f64],
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    sample_rate_hz: f64,
    settings: &JointHarmonicSettings,
) -> Result<JointEstimate> {
    let model = &settings.model;
    let noise = &settings.noise;
    let tolerances = settings.tolerances;
    validate_signal_model(model, sample_rate_hz)?;
    validate_noise_model(noise)?;
    let design = design_matrix(
        times,
        reference_frequency_hz,
        reference_phase_rad,
        model,
        sample_rate_hz,
    )?;
    let (whitened_design, whitened_signal, variance_scale) = whiten(
        &design,
        signal,
        reference_phase_rad,
        reference_frequency_hz,
        times,
        noise,
        tolerances,
    )?;
    let (beta, whitened_rms, rank, condition) =
        solve_direct(&whitened_design, &whitened_signal, tolerances)?;
    if variance_scale <= 0.0 {
        return Err(AnalysisError::new(
            "non_finite_output",
            "variance scale is non-positive",
        ));
    }
    let residual_rms = whitened_rms / variance_scale.sqrt();
    let beta_vec = beta.as_slice().to_vec();
    let covariance_beta = covariance_from_qr(&whitened_design, variance_scale)?;
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
    })
}
