//! SVD values-only equivalence evidence (P2: skip unused U factor).
//!
//! `solve_direct_with_factor` (crates/pmoke-analysis-core/src/joint.rs) feeds
//! only the singular values into its rank/condition gates; the thin U factor
//! was computed and dropped. This target pins that the values-only SVD
//! `svd(false, false)` drives the identical gate decisions as the full
//! `svd(true, false)` path, plus a generous wall-time bound on the N=2003/P=25
//! reference shape.
//!
//! The two nalgebra SVD code paths agree to ~1 ulp (max rel diff ~4e-16 on
//! the reference fixture), not bit-for-bit, so rank/condition comparisons use
//! tight relative tolerances (1e-12) rather than exact equality. The
//! least-squares solution itself comes from the untouched QR path, so
//! beta/xy/covariance are bit-identical by construction; the oracle and
//! shared-R bit-identity tests pin those outputs.

use nalgebra::{DMatrix, DVector};
use pmoke_analysis_core::{
    DEFAULT_MAX_CONDITION, DEFAULT_RANK_TOL, HarmonicSignalModel, JointSolverTolerances,
    design_matrix, solve_direct,
};
use std::time::Instant;

/// Reference rank/condition derivation from the full (U-computing) SVD, using
/// the same threshold math as `solve_direct_with_factor`.
fn reference_rank_condition(matrix: &DMatrix<f64>) -> (usize, f64, DVector<f64>) {
    let svd = matrix.clone().svd(true, false);
    let singular = svd.singular_values;
    let sigma_max = singular[0];
    let mut rank = 0;
    for value in singular.iter() {
        if *value > DEFAULT_RANK_TOL * sigma_max {
            rank += 1;
        }
    }
    let columns = matrix.ncols();
    let condition = sigma_max / singular[columns - 1];
    (rank, condition, singular)
}

fn close_rel(got: f64, want: f64, tol: f64, context: &str) {
    let denom = want.abs().max(1.0);
    assert!(
        (got - want).abs() <= tol * denom,
        "{context}: {got} vs {want}"
    );
}

/// The values-only SVD drives the same rank/condition gates as the full SVD:
/// same rank, condition within 1e-12, and the shipped `solve_direct` agrees
/// with the full-SVD reference on pass/fail and reported values.
#[test]
fn values_only_svd_matches_full_svd_gates() {
    let tolerances = JointSolverTolerances::default();
    // Well-conditioned fixture design (N=97, p=25): both paths report full
    // rank and a condition far inside the cap.
    const N: usize = 97;
    const DT: f64 = 17.8e-9;
    const F_REF: f64 = 1.17e6;
    const PHASE: f64 = 0.3;
    let model = HarmonicSignalModel {
        fit_harmonics: (1..=12).collect(),
        output_harmonics: (1..=6).collect(),
        envelope_degree: 0,
    };
    let times: Vec<f64> = (0..N).map(|i| i as f64 * DT).collect();
    let design = design_matrix(&times, F_REF, PHASE, &model, 1.0 / DT).unwrap();
    assert_eq!((design.nrows(), design.ncols()), (N, 25));
    let lite = design.clone().svd(false, false);
    let (ref_rank, ref_condition, ref_singular) = reference_rank_condition(&design);
    assert_eq!(lite.singular_values.len(), ref_singular.len());
    for (index, (got, want)) in lite
        .singular_values
        .iter()
        .zip(ref_singular.iter())
        .enumerate()
    {
        close_rel(*got, *want, 1.0e-12, &format!("fixture sigma {index}"));
    }
    let response = DVector::from_element(N, 1.0);
    let (_, _, rank, condition) = solve_direct(&design, &response, tolerances).unwrap();
    assert_eq!(rank, ref_rank, "fixture rank differs");
    assert_eq!(rank, 25, "fixture design must be full rank");
    close_rel(condition, ref_condition, 1.0e-12, "fixture condition");
    assert!(
        condition < DEFAULT_MAX_CONDITION,
        "fixture condition {condition} must be inside the cap"
    );

    // Rank-deficient 3x2: both paths agree the rank is below the columns, and
    // the shipped solver rejects with the same code.
    let deficient = DMatrix::from_row_slice(3, 2, &[1.0, 2.0, 3.0, 6.0, 5.0, 10.0]);
    let (def_rank, _, _) = reference_rank_condition(&deficient);
    assert!(def_rank < 2, "reference must see rank {def_rank} < 2");
    let lite_def = deficient.clone().svd(false, false);
    let mut lite_rank = 0;
    for value in lite_def.singular_values.iter() {
        if *value > DEFAULT_RANK_TOL * lite_def.singular_values[0] {
            lite_rank += 1;
        }
    }
    assert_eq!(lite_rank, def_rank, "deficient rank differs");
    assert_eq!(
        solve_direct(
            &deficient,
            &DVector::from_vec(vec![1.0, 2.0, 3.0]),
            tolerances
        )
        .unwrap_err()
        .code(),
        "rank_deficient_design"
    );

    // Ill-conditioned diag(1, 1e-9): condition 1e9 above the 1e8 cap on both
    // paths; the shipped solver rejects with the same code.
    let ill = DMatrix::from_row_slice(2, 2, &[1.0, 0.0, 0.0, 1e-9]);
    let (ill_rank, ill_condition, _) = reference_rank_condition(&ill);
    assert_eq!(ill_rank, 2);
    assert!(ill_condition > DEFAULT_MAX_CONDITION);
    let lite_ill = ill.clone().svd(false, false);
    close_rel(
        lite_ill.singular_values[0] / lite_ill.singular_values[1],
        ill_condition,
        1.0e-12,
        "ill condition",
    );
    assert_eq!(
        solve_direct(&ill, &DVector::from_vec(vec![1.0, 1.0]), tolerances)
            .unwrap_err()
            .code(),
        "ill_conditioned_design"
    );

    // Borderline diag(1, 1e-7): passes on both paths with rank 2 and matching
    // condition inside the cap.
    let borderline = DMatrix::from_row_slice(2, 2, &[1.0, 0.0, 0.0, 1e-7]);
    let (ok_rank, ok_condition, _) = reference_rank_condition(&borderline);
    assert_eq!(ok_rank, 2);
    let (_, _, rank, condition) =
        solve_direct(&borderline, &DVector::from_vec(vec![1.0, 1.0]), tolerances).unwrap();
    assert_eq!(rank, ok_rank, "borderline rank differs");
    close_rel(condition, ok_condition, 1.0e-12, "borderline condition");
    assert!(condition < DEFAULT_MAX_CONDITION);
}

/// Generous-bound wall-time regression gate on the N=2003/P=25 reference
/// shape: repeated shipped solves must complete far inside 60 s. Measured
/// release cost is sub-millisecond per solve; the bound only trips on a
/// pathological regression (per-window sleep, quadratic blowup, or a
/// reintroduced per-window factorization).
#[test]
fn values_only_reference_shape_wall_time_bound() {
    const N: usize = 2003;
    const DT: f64 = 1e-7;
    const F_REF: f64 = 10e3;
    const PHASE: f64 = 0.3;
    let model = HarmonicSignalModel {
        fit_harmonics: (1..=12).collect(),
        output_harmonics: (1..=6).collect(),
        envelope_degree: 0,
    };
    let times: Vec<f64> = (0..N).map(|i| i as f64 * DT).collect();
    let design = design_matrix(&times, F_REF, PHASE, &model, 1.0 / DT).unwrap();
    assert_eq!((design.nrows(), design.ncols()), (N, 25));
    let response = DVector::from_element(N, 1.0);
    let tolerances = JointSolverTolerances::default();
    // Warmup outside the timed region.
    solve_direct(&design, &response, tolerances).unwrap();
    const SOLVES: usize = 10;
    let t0 = Instant::now();
    for _ in 0..SOLVES {
        solve_direct(&design, &response, tolerances).unwrap();
    }
    let elapsed = t0.elapsed();
    let per_solve_us = elapsed.as_secs_f64() * 1e6 / SOLVES as f64;
    println!("reference shape solve: {SOLVES} solves in {elapsed:.2?} ({per_solve_us:.1}us/solve)");
    assert!(
        elapsed.as_secs() < 60,
        "solve wall-time regression: {SOLVES} solves took {elapsed:?} (bound 60s)"
    );
}
