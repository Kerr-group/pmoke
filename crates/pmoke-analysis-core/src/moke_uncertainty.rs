//! Conditional MOKE uncertainty from joint GLS XY covariances (R2c).
//!
//! The MOKE standard angle is `theta = (1/2) atan2(jn(2,x) a1, jn(1,x) a2)`
//! with `x = 2*phim` frozen; harmonics inputs are named per the frozen
//! formula (`a1` = h1 in-phase, `a2` = h2 in-phase). The conditional
//! variance follows the delta method (first-order Taylor) through the same
//! formula the analyzer evaluates, with the gradient at the observed
//! `(a1, a2)` point — no linearization of the phase-delta estimation
//! itself, which stays outside the conditional claim.
//!
//! The estimator publishes the XY covariance in peak-amplitude units
//! (no rescaling by the core mapper); the (a1, a2) marginal is read off the rotated
//! `x1/x2` block. Degenerate Bessel denominators and non-finite inputs are
//! fail-closed (FR-041): unknown is never published as zero.

use crate::{AnalysisError, Result};

/// Local finite-input guard (mirrors `joint::require_finite`).
fn require_finite(name: &str, value: f64) -> Result<()> {
    if !value.is_finite() {
        return Err(AnalysisError::new(
            "non_finite_input",
            format!("{name} must be finite"),
        ));
    }
    Ok(())
}

/// Frozen Bessel denominator floor, mirroring BESSEL_DENOMINATOR_MIN in
/// `moke_standard_analysis.py` and `moke-analysis-core` FR-13.
pub const BESSEL_DENOMINATOR_MIN: f64 = 1e-12;

/// Evaluates the standard MOKE angle (radians) at one `(a1, a2)` point.
pub fn moke_standard_angle(a1: f64, a2: f64, phim: f64) -> Result<f64> {
    require_finite("a1", a1)?;
    require_finite("a2", a2)?;
    require_finite("phim", phim)?;
    let (top, bottom) = moke_bessel_coefficients(phim)?;
    Ok(0.5 * (top * a1).atan2(bottom * a2))
}

/// Delta-method conditional variance of the standard MOKE angle at one
/// `(a1, a2)` point with its 2x2 `(x1, x2)` marginal covariance
/// `[[v11, v12], [v12, v22]]` (peak-amplitude XY units).
pub fn moke_standard_angle_variance(
    a1: f64,
    a2: f64,
    phim: f64,
    v11: f64,
    v12: f64,
    v22: f64,
) -> Result<f64> {
    require_finite("a1", a1)?;
    require_finite("a2", a2)?;
    require_finite("phim", phim)?;
    for (name, value) in [("v11", v11), ("v12", v12), ("v22", v22)] {
        require_finite(name, value)?;
    }
    let (top, bottom) = moke_bessel_coefficients(phim)?;
    let numerator = top * a1;
    let denominator = bottom * a2;
    let radius_squared = numerator * numerator + denominator * denominator;
    if !radius_squared.is_finite() || radius_squared <= 0.0 {
        return Err(AnalysisError::new(
            "degenerate_moke_point",
            "MOKE angle gradient undefined at zero radius",
        ));
    }
    // d theta / d a1 = (1/2) top*denominator/r^2,
    // d theta / d a2 = -(1/2) numerator*bottom/r^2.
    let scale = 0.5 / radius_squared;
    let gradient_a1 = scale * top * denominator;
    let gradient_a2 = -scale * numerator * bottom;
    let variance = gradient_a1 * gradient_a1 * v11
        + 2.0 * gradient_a1 * gradient_a2 * v12
        + gradient_a2 * gradient_a2 * v22;
    if !variance.is_finite() {
        return Err(AnalysisError::new(
            "non_finite_input",
            "MOKE conditional variance is non-finite",
        ));
    }
    if variance < 0.0 {
        return Err(AnalysisError::new(
            "indefinite_marginal",
            "MOKE conditional variance is negative: input marginal is not PSD",
        ));
    }
    Ok(variance)
}

/// Frozen Bessel coefficients `(jn(2,2*phim), jn(1,2*phim))`, fail-closed
/// on non-finite or degenerate denominators.
fn moke_bessel_coefficients(phim: f64) -> Result<(f64, f64)> {
    let argument = 2.0 * phim;
    let top = libm::jn(2, argument);
    let bottom = libm::jn(1, argument);
    if !top.is_finite() || !bottom.is_finite() {
        return Err(AnalysisError::new(
            "non_finite_input",
            "Bessel denominators must be finite",
        ));
    }
    if top.abs() <= BESSEL_DENOMINATOR_MIN || bottom.abs() <= BESSEL_DENOMINATOR_MIN {
        return Err(AnalysisError::new(
            "degenerate_bessel",
            "Bessel denominators must be finite and nonzero",
        ));
    }
    Ok((top, bottom))
}
