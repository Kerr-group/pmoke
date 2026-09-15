//! R2c: conditional MOKE uncertainty — delta-method variance through the
//! frozen standard angle formula, validated against a numerical oracle.
use pmoke_analysis_core::moke_uncertainty::{moke_standard_angle, moke_standard_angle_variance};

const PHIM: f64 = 0.92;

fn numerical_variance(a1: f64, a2: f64, v11: f64, v12: f64, v22: f64) -> f64 {
    let step = 1e-6;
    let center = moke_standard_angle(a1, a2, PHIM).unwrap();
    let d_a1 = (moke_standard_angle(a1 + step, a2, PHIM).unwrap() - center) / step;
    let d_a2 = (moke_standard_angle(a1, a2 + step, PHIM).unwrap() - center) / step;
    d_a1 * d_a1 * v11 + 2.0 * d_a1 * d_a2 * v12 + d_a2 * d_a2 * v22
}

#[test]
fn angle_matches_frozen_python_formula() {
    // Mirrors MokeStandardAnalyser.calculate with DEFAULT_PHIM: hand-evaluate
    // the ratio at a non-trivial point.
    let angle = moke_standard_angle(1.7, -0.9, PHIM).unwrap();
    let top = 0.3157453060879723_f64 * 1.7;
    let bottom = 0.5818649368420833_f64 * -0.9;
    assert!((angle - 0.5 * top.atan2(bottom)).abs() < 1e-15);
}

#[test]
fn variance_matches_numerical_gradient_oracle() {
    let (a1, a2) = (1.7, -0.9);
    let (v11, v12, v22) = (0.004, 0.0007, 0.009);
    let analytic = moke_standard_angle_variance(a1, a2, PHIM, v11, v12, v22).unwrap();
    let numeric = numerical_variance(a1, a2, v11, v12, v22);
    assert!(
        (analytic - numeric).abs() / numeric < 1e-6,
        "analytic {analytic} vs numeric {numeric}"
    );
    assert!(analytic > 0.0);
}

#[test]
fn variance_zero_cross_covariance_reduces() {
    // Diagonal marginal: variance is the weighted sum of the two terms.
    let (a1, a2) = (0.8, 1.3);
    let full = moke_standard_angle_variance(a1, a2, PHIM, 0.01, 0.002, 0.02).unwrap();
    let diagonal = moke_standard_angle_variance(a1, a2, PHIM, 0.01, 0.0, 0.02).unwrap();
    assert!((full - diagonal).abs() > 0.0);
    assert!(full > 0.0 && diagonal > 0.0);
}

#[test]
fn degenerate_inputs_fail_closed() {
    // Zero radius: gradient undefined, never zero.
    assert!(moke_standard_angle_variance(0.0, 0.0, PHIM, 0.01, 0.0, 0.01).is_err());
    // Non-finite variance input.
    assert!(moke_standard_angle_variance(1.0, 1.0, PHIM, f64::NAN, 0.0, 0.01).is_err());
    // Non-finite angle input.
    assert!(moke_standard_angle(f64::INFINITY, 1.0, PHIM).is_err());
    // Indefinite marginal (negative variance direction) fails, not zero.
    assert!(moke_standard_angle_variance(1.0, 1.0, PHIM, -0.5, 0.0, -0.5).is_err());
}

#[test]
fn dense_rotation_matches_matrix_entry_point() {
    use nalgebra::DMatrix;
    use pmoke_analysis_core::joint::{rotate_xy_covariance, rotate_xy_covariance_dense};

    let mut entries: Vec<f64> = (0..144).map(|index| 0.001 * (index as f64 + 1.0)).collect();
    // Symmetrize so the input is a plausible covariance.
    for row in 0..12 {
        for column in (row + 1)..12 {
            entries[column * 12 + row] = entries[row * 12 + column];
        }
        entries[row * 12 + row] += 1.0;
    }
    let deltas = [0.1, -0.2, 0.3, -0.4, 0.5, -0.6];
    let matrix = DMatrix::from_row_slice(12, 12, &entries);
    let expected = rotate_xy_covariance(&matrix, &deltas).unwrap();
    let dense = rotate_xy_covariance_dense(&entries, &deltas).unwrap();
    // DMatrix::iter is column-major; compare element-wise via indexing.
    for row in 0..12 {
        for column in 0..12 {
            let column_major = expected[(row, column)];
            let row_major = dense[row * 12 + column];
            assert!(
                (column_major - row_major).abs() < 1e-12,
                "({row},{column}): {column_major} vs {row_major}"
            );
        }
    }
}

#[test]
fn dense_rotation_rejects_wrong_shape() {
    use pmoke_analysis_core::joint::rotate_xy_covariance_dense;

    let deltas = [0.0; 6];
    assert!(rotate_xy_covariance_dense(&vec![1.0; 143], &deltas).is_err());
    assert!(rotate_xy_covariance_dense(&vec![1.0; 145], &deltas).is_err());
}
