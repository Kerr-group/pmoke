//! Gate-compatibility regressions for the P3 thin-R SVD diagnostic.
//!
//! - F1: finite inputs whose QR factor overflows must report the documented
//!   typed `non_finite_output` instead of panicking inside nalgebra's SVD.
//! - F2: a configured `max_condition` set exactly at the base design-SVD
//!   boundary must admit exactly as the base path admits, despite few-ULP
//!   thin-R drift. Thresholds are unchanged; the solver defers
//!   borderline/overflow gates to the base design diagnostic.

use nalgebra::{DMatrix, DVector};
use pmoke_analysis_core::{JointSolverTolerances, solve_direct, solve_direct_and_covariance};

/// F1: 3x2 finite design whose thin-R overflows (R nonfinite) while the
/// design SVD stays finite. Base returns typed `non_finite_output`
/// ("solver residual is non-finite"); the reused path must too, never panic.
#[test]
fn finite_qr_overflow_reports_typed_error() {
    let design = DMatrix::<f64>::from_row_slice(3, 2, &[1e200, 0.0, 0.0, 1e200, 1e200, 1e200]);
    let signal = DVector::from_vec(vec![1.0, 2.0, 3.0]);
    // Precondition: QR really overflows on this input on this toolchain.
    let upper = design.clone().qr().r();
    assert!(
        !upper.iter().all(|v| v.is_finite()),
        "expected nonfinite thin-R for the overflow fixture"
    );
    // Design SVD stays finite (base gate passes through to the solver).
    let sigma_base = design.clone().svd(false, false).singular_values;
    assert!(
        sigma_base.iter().all(|v| v.is_finite()),
        "expected finite design singulars for the overflow fixture"
    );
    let err = match solve_direct(&design, &signal, JointSolverTolerances::default()) {
        Ok(_) => panic!("overflow fixture must not solve Ok"),
        Err(e) => e,
    };
    assert_eq!(err.code(), "non_finite_output");
    let err2 = match solve_direct_and_covariance(
        &design,
        &signal,
        JointSolverTolerances::default(),
        1.0,
    ) {
        Ok(_) => panic!("overflow fixture (combined) must not solve Ok"),
        Err(e) => e,
    };
    assert_eq!(err2.code(), "non_finite_output");
}

/// F2: deterministic 5x3 fixture from the PR #300 review probe
/// (column-major storage). The base boundary `max_condition` equals the
/// design-SVD condition on this machine, so the base path admits; the
/// reused path must admit with the identical condition, and must reject
/// consistently one ULP below.
#[test]
fn configured_condition_boundary_matches_base() {
    let design = DMatrix::<f64>::from_column_slice(
        5,
        3,
        &[
            -0.4823583459179779,
            0.48838532997553874,
            0.04593407039756037,
            -0.4869719549252166,
            0.2189515300938093,
            0.01840278888191893,
            -0.14169653841366409,
            -0.36341247990884723,
            -0.4915927379538369,
            -0.09189906304919737,
            0.11480926601730468,
            0.09331729855417237,
            -0.23320877788798317,
            0.17959261817041372,
            0.09831885379135208,
        ],
    );
    let signal = DVector::from_element(5, 1.0);
    // Base diagnostic on this machine defines the boundary.
    let sigma_base = design.clone().svd(false, false).singular_values;
    let cond_base = sigma_base[0] / sigma_base[2];
    assert!(cond_base.is_finite() && cond_base > 0.0);
    let at_boundary = JointSolverTolerances {
        max_condition: cond_base,
        ..Default::default()
    };
    let (_, _, _, condition) = solve_direct(&design, &signal, at_boundary).unwrap_or_else(|e| {
        panic!(
            "boundary max_condition={cond_base:.17e} must admit like base, got {}: {}",
            e.code(),
            e
        )
    });
    assert_eq!(
        condition.to_bits(),
        cond_base.to_bits(),
        "borderline condition must equal the base diagnostic exactly"
    );
    // One ULP below the boundary both paths reject.
    let below = JointSolverTolerances {
        max_condition: cond_base.next_down(),
        ..Default::default()
    };
    let err = solve_direct(&design, &signal, below).unwrap_err();
    assert_eq!(err.code(), "ill_conditioned_design");
}
