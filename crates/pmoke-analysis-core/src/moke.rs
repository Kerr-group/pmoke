use crate::{AnalysisError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarmonicsMokeOutput {
    pub values_rad: Vec<f64>,
    pub representative_modulation_depth: f64,
}

pub fn calculate_harmonics_moke(
    a2: &[f64],
    a3: &[f64],
    a4: &[f64],
    a6: &[f64],
    factor: f64,
) -> Result<HarmonicsMokeOutput> {
    let length = a2.len();
    if length == 0 {
        return Err(AnalysisError::new(
            "empty_harmonics",
            "harmonic arrays must not be empty",
        ));
    }
    if [a3.len(), a4.len(), a6.len()]
        .into_iter()
        .any(|candidate| candidate != length)
    {
        return Err(AnalysisError::new(
            "length_mismatch",
            "harmonic arrays must have equal lengths",
        ));
    }
    if !factor.is_finite()
        || a2
            .iter()
            .chain(a3)
            .chain(a4)
            .chain(a6)
            .any(|value| !value.is_finite())
    {
        return Err(AnalysisError::new(
            "non_finite_harmonics",
            "harmonic arrays and factor must be finite",
        ));
    }

    let mut modulation_depths = a2
        .iter()
        .zip(a4)
        .zip(a6)
        .filter_map(|((&second, &fourth), &sixth)| {
            let denominator = 15.0 * second + 24.0 * fourth + 9.0 * sixth;
            let radicand = 20.0 * fourth / denominator;
            let value = 6.0 * radicand.sqrt();
            (value.is_finite() && value > 0.0).then_some(value)
        })
        .collect::<Vec<_>>();
    if modulation_depths.is_empty() {
        return Err(AnalysisError::new(
            "invalid_modulation_depth",
            "cannot determine a finite positive modulation depth",
        ));
    }
    modulation_depths.sort_by(f64::total_cmp);
    let midpoint = modulation_depths.len() / 2;
    let modulation_depth = if modulation_depths.len().is_multiple_of(2) {
        0.5 * (modulation_depths[midpoint - 1] + modulation_depths[midpoint])
    } else {
        modulation_depths[midpoint]
    };

    let values_rad = a2
        .iter()
        .zip(a3)
        .zip(a4)
        .map(|((&second, &third), &fourth)| {
            let denominator = (second + fourth) * modulation_depth / 6.0;
            0.5 * (third / denominator).atan() * factor
        })
        .collect::<Vec<_>>();
    if values_rad.iter().any(|value| !value.is_finite()) {
        return Err(AnalysisError::new(
            "non_finite_moke",
            "harmonics Moke calculation produced a non-finite result",
        ));
    }

    Ok(HarmonicsMokeOutput {
        values_rad,
        representative_modulation_depth: modulation_depth,
    })
}

/// Denominator guard: Bessel zeros are never hit exactly in floating point,
/// so values at or below this magnitude count as degenerate.
pub const BESSEL_DENOMINATOR_MIN: f64 = 1e-12;

/// Monitor voltage from the second and third lock-in harmonics.
///
/// `Vm = 0.5 * sqrt((third / jn(3, depth))^2 + (second / jn(2, depth))^2)`
/// with `depth` shared from the angle computation (D8).
pub fn calculate_harmonics_vm(
    second: &[f64],
    third: &[f64],
    modulation_depth: f64,
) -> Result<Vec<f64>> {
    let length = second.len();
    if length == 0 {
        return Err(AnalysisError::new(
            "empty_harmonics",
            "harmonic arrays must not be empty",
        ));
    }
    if third.len() != length {
        return Err(AnalysisError::new(
            "length_mismatch",
            "harmonic arrays must have equal lengths",
        ));
    }
    if !modulation_depth.is_finite() {
        return Err(AnalysisError::new(
            "non_finite_modulation_depth",
            "modulation depth must be finite",
        ));
    }
    if second.iter().chain(third).any(|value| !value.is_finite()) {
        return Err(AnalysisError::new(
            "non_finite_harmonics",
            "harmonic arrays must be finite",
        ));
    }
    let bessel_orders = [libm::jn(2, modulation_depth), libm::jn(3, modulation_depth)];
    if bessel_orders
        .iter()
        .any(|value| !value.is_finite() || value.abs() <= BESSEL_DENOMINATOR_MIN)
    {
        return Err(AnalysisError::new(
            "zero_bessel_denominator",
            "Bessel denominators must be finite and nonzero",
        ));
    }
    let values: Vec<f64> = second
        .iter()
        .zip(third)
        .map(|(&second_value, &third_value)| {
            0.5 * ((third_value / bessel_orders[1]).powi(2)
                + (second_value / bessel_orders[0]).powi(2))
            .sqrt()
        })
        .collect();
    if values.iter().any(|value| !value.is_finite()) {
        return Err(AnalysisError::new(
            "non_finite_vm",
            "harmonics Vm calculation produced a non-finite result",
        ));
    }
    Ok(values)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_folded_angle() {
        let theta = 0.01_f64;
        let coefficients = [
            0.315_745_306_087_972_3,
            0.104_537_902_479_595_42,
            0.025_139_158_519_404_087,
            0.000_745_551_998_014_054_3,
        ];
        let a2 = vec![(2.0 * theta).cos() * coefficients[0]; 8];
        let a3 = vec![(2.0 * theta).sin() * coefficients[1]; 8];
        let a4 = vec![(2.0 * theta).cos() * coefficients[2]; 8];
        let a6 = vec![(2.0 * theta).cos() * coefficients[3]; 8];
        let output = calculate_harmonics_moke(&a2, &a3, &a4, &a6, 1.0).unwrap();
        let expected = 0.5 * (2.0 * theta).tan().atan();
        assert!(
            output
                .values_rad
                .iter()
                .all(|value| (value - expected).abs() < 1.0e-12)
        );
    }

    #[test]
    fn vm_recovers_normalized_carrier_amplitude() {
        let depth = 1.84_f64;
        let amplitude = 2.5_f64;
        let second = vec![libm::jn(2, depth) * amplitude; 8];
        let third = vec![libm::jn(3, depth) * amplitude; 8];
        let values = calculate_harmonics_vm(&second, &third, depth).unwrap();
        let expected = 0.5 * amplitude * 2.0_f64.sqrt();
        assert!(
            values
                .iter()
                .all(|value| (value - expected).abs() < 1.0e-12)
        );
    }

    #[test]
    fn vm_rejects_empty_misaligned_and_degenerate_inputs() {
        assert_eq!(
            calculate_harmonics_vm(&[], &[], 1.84).unwrap_err().code(),
            "empty_harmonics"
        );
        assert_eq!(
            calculate_harmonics_vm(&[1.0], &[1.0, 2.0], 1.84)
                .unwrap_err()
                .code(),
            "length_mismatch"
        );
        assert_eq!(
            calculate_harmonics_vm(&[1.0], &[1.0], f64::NAN)
                .unwrap_err()
                .code(),
            "non_finite_modulation_depth"
        );
        assert_eq!(
            calculate_harmonics_vm(&[1.0], &[f64::INFINITY], 1.84)
                .unwrap_err()
                .code(),
            "non_finite_harmonics"
        );
        // jn(2, x) has a zero near x = 5.1356: Vm must refuse it explicitly.
        assert_eq!(
            calculate_harmonics_vm(&[1.0], &[1.0], 5.135_622_301_840_683)
                .unwrap_err()
                .code(),
            "zero_bessel_denominator"
        );
    }

    #[test]
    fn rejects_non_finite_and_misaligned_inputs() {
        assert_eq!(
            calculate_harmonics_moke(&[1.0], &[1.0, 2.0], &[1.0], &[1.0], 1.0)
                .unwrap_err()
                .code(),
            "length_mismatch"
        );
        assert_eq!(
            calculate_harmonics_moke(&[1.0], &[f64::INFINITY], &[1.0], &[1.0], 1.0)
                .unwrap_err()
                .code(),
            "non_finite_harmonics"
        );
    }
}
