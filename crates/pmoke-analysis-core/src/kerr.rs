use crate::{AnalysisError, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HarmonicsKerrOutput {
    pub values_rad: Vec<f64>,
    pub representative_modulation_depth: f64,
}

pub fn calculate_harmonics_kerr(
    a2: &[f64],
    a3: &[f64],
    a4: &[f64],
    a6: &[f64],
    factor: f64,
) -> Result<HarmonicsKerrOutput> {
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
            "non_finite_kerr",
            "harmonics Kerr calculation produced a non-finite result",
        ));
    }

    Ok(HarmonicsKerrOutput {
        values_rad,
        representative_modulation_depth: modulation_depth,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovers_folded_kerr_angle() {
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
        let output = calculate_harmonics_kerr(&a2, &a3, &a4, &a6, 1.0).unwrap();
        let expected = 0.5 * (2.0 * theta).tan().atan();
        assert!(
            output
                .values_rad
                .iter()
                .all(|value| (value - expected).abs() < 1.0e-12)
        );
    }

    #[test]
    fn rejects_non_finite_and_misaligned_inputs() {
        assert_eq!(
            calculate_harmonics_kerr(&[1.0], &[1.0, 2.0], &[1.0], &[1.0], 1.0)
                .unwrap_err()
                .code(),
            "length_mismatch"
        );
        assert_eq!(
            calculate_harmonics_kerr(&[1.0], &[f64::INFINITY], &[1.0], &[1.0], 1.0)
                .unwrap_err()
                .code(),
            "non_finite_harmonics"
        );
    }
}
