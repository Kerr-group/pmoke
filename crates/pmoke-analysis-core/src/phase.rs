use crate::error::{AnalysisError, Result};

pub fn rotate_phase(lix: &[f64], liy: &[f64], delta: f64) -> Result<(Vec<f64>, Vec<f64>)> {
    if lix.len() != liy.len() {
        return Err(AnalysisError::new(
            "length_mismatch",
            format!(
                "phase rotation requires equal-length x and y arrays (got {} and {})",
                lix.len(),
                liy.len()
            ),
        ));
    }
    if !delta.is_finite() || lix.iter().chain(liy).any(|value| !value.is_finite()) {
        return Err(AnalysisError::new(
            "non_finite_phase",
            "phase rotation x, y, and delta must be finite",
        ));
    }

    let cos_delta = delta.cos();
    let sin_delta = delta.sin();
    Ok(lix
        .iter()
        .zip(liy)
        .map(|(&x, &y)| {
            (
                x * cos_delta + y * sin_delta,
                -x * sin_delta + y * cos_delta,
            )
        })
        .unzip())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_preserves_magnitude() {
        let (rotated_x, rotated_y) = rotate_phase(&[3.0, -1.0], &[4.0, 2.0], 0.73).unwrap();
        for ((x, y), expected) in rotated_x
            .iter()
            .zip(&rotated_y)
            .zip([5.0_f64, 5.0_f64.sqrt()])
        {
            assert!((x.hypot(*y) - expected).abs() < 1.0e-12);
        }
    }

    #[test]
    fn mismatched_lengths_are_rejected_before_rotation() {
        let error = rotate_phase(&[1.0, 2.0], &[3.0], 0.2).unwrap_err();

        assert_eq!(error.code(), "length_mismatch");
        assert_eq!(
            error.message(),
            "phase rotation requires equal-length x and y arrays (got 2 and 1)"
        );
    }

    #[test]
    fn non_finite_inputs_are_rejected() {
        for (x, y, delta) in [
            (vec![f64::NAN], vec![1.0], 0.2),
            (vec![1.0], vec![f64::INFINITY], 0.2),
            (vec![1.0], vec![2.0], f64::NEG_INFINITY),
        ] {
            let error = rotate_phase(&x, &y, delta).unwrap_err();
            assert_eq!(error.code(), "non_finite_phase");
            assert_eq!(
                error.message(),
                "phase rotation x, y, and delta must be finite"
            );
        }
    }
}
