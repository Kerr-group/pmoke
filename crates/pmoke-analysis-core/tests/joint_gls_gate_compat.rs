//! Gate-compatibility regressions for the P3 thin-R SVD diagnostic.
//!
//! - F1: finite inputs whose QR factor overflows must report the documented
//!   typed `non_finite_output` instead of panicking inside nalgebra's SVD.
//! - F2: a configured `max_condition` set exactly at the base design-SVD
//!   boundary must admit exactly as the base path admits, despite few-ULP
//!   thin-R drift. Thresholds are unchanged; the solver defers
//!   borderline/overflow gates to the base design diagnostic.
//! - F2-false-acceptance: the fixed 1e-12 fallback band missed real gate
//!   distances (~2.263e-12 on the review's finite full-rank 5x3 fixture).
//!   The solver must preserve the literal base success/error decision at
//!   both the rank and the condition gate for that fixture, for midpoint
//!   thresholds derived portably at runtime, and for more ill-conditioned
//!   near-dependent variants -- on both public entry points -- using the
//!   old numerical path below as the oracle. Thresholds and the 1e-12
//!   portable golden bound are unchanged.

use nalgebra::{DMatrix, DVector};
use pmoke_analysis_core::{
    AnalysisError, JointSolverTolerances, covariance_from_qr, solve_direct,
    solve_direct_and_covariance,
};

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

/// Old-path oracle: the pre-reuse numerical path, gate for gate and flop
/// for flop (literal base control from the PR #300 review, namespace
/// adapted). The design SVD is values-only on the whitened design; the
/// solution runs thin QR with the same back-substitution and scale-safe
/// residual as the solver under test.
#[allow(clippy::type_complexity)]
fn solve_old_path(
    whitened_design: &DMatrix<f64>,
    whitened_signal: &DVector<f64>,
    tolerances: JointSolverTolerances,
) -> Result<(DVector<f64>, f64, usize, f64, DMatrix<f64>), AnalysisError> {
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
    let svd = whitened_design.clone().svd(false, false);
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

/// Review's finite full-rank 5x3 false-acceptance fixture (column-major).
fn false_acceptance_fixture() -> (DMatrix<f64>, DVector<f64>) {
    let design = DMatrix::<f64>::from_column_slice(
        5,
        3,
        &[
            0.0387016914487478,
            -0.49018820809197816,
            -0.053885297561740875,
            -0.47983974423733494,
            -0.22034393779736067,
            0.23083058826741598,
            0.3071218742805436,
            -0.3222217346282319,
            -0.023416836644433836,
            0.17234010996560312,
            0.038680228492326656,
            -0.49022304842150954,
            -0.05388188863419866,
            -0.47985788853026834,
            -0.22035247215034973,
        ],
    );
    (design, DVector::from_element(5, 1.0))
}

/// Assert both public entry points reproduce the old-path verdict exactly:
/// same error code on rejection; bit-identical beta/residual/rank (plus
/// covariance parity for the combined API) and 1e-12-bounded condition on
/// acceptance.
fn assert_parity_with_old_path(
    design: &DMatrix<f64>,
    signal: &DVector<f64>,
    tolerances: JointSolverTolerances,
    context: &str,
) {
    let expected = solve_old_path(design, signal, tolerances);
    let direct = solve_direct(design, signal, tolerances);
    match (&expected, &direct) {
        (Err(want), Err(got)) => assert_eq!(
            got.code(),
            want.code(),
            "{context}: solve_direct error code differs (want {}, got {})",
            want.code(),
            got.code()
        ),
        (Ok(want), Ok(got)) => {
            assert!(
                want.0
                    .iter()
                    .zip(got.0.iter())
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "{context}: solve_direct beta bits differ"
            );
            assert_eq!(
                want.1.to_bits(),
                got.1.to_bits(),
                "{context}: residual bits differ"
            );
            assert_eq!(want.2, got.2, "{context}: rank differs");
            // Condition is bit-identical on the fallback path and agrees to
            // ~1e-15 on the clear-acceptance fast path; the suite pins 1e-12.
            let drift = (want.3 - got.3).abs() / want.3.abs().max(1e-300);
            assert!(drift <= 1e-12, "{context}: condition drifted {drift:e}");
        }
        (want, got) => panic!(
            "{context}: solve_direct admission differs (want {}, got {})",
            want.as_ref()
                .map(|v| format!("Ok(rank={})", v.2))
                .unwrap_or_else(|e| e.code().to_string()),
            got.as_ref()
                .map(|v| format!("Ok(rank={})", v.2))
                .unwrap_or_else(|e| e.code().to_string())
        ),
    }
    let combined = solve_direct_and_covariance(design, signal, tolerances, 1.0);
    match (&expected, &combined) {
        (Err(want), Err(got)) => assert_eq!(
            got.code(),
            want.code(),
            "{context}: combined error code differs (want {}, got {})",
            want.code(),
            got.code()
        ),
        (Ok(want), Ok(got)) => {
            assert!(
                want.0
                    .iter()
                    .zip(got.0.iter())
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "{context}: combined beta bits differ"
            );
            assert_eq!(
                want.1.to_bits(),
                got.1.to_bits(),
                "{context}: combined residual bits differ"
            );
            assert_eq!(want.2, got.2, "{context}: combined rank differs");
            let drift = (want.3 - got.3).abs() / want.3.abs().max(1e-300);
            assert!(
                drift <= 1e-12,
                "{context}: combined condition drifted {drift:e}"
            );
            let covariance = covariance_from_qr(design, 1.0).unwrap();
            assert!(
                covariance
                    .iter()
                    .zip(got.4.iter())
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "{context}: combined covariance bits differ"
            );
        }
        (want, got) => panic!(
            "{context}: combined admission differs (want {}, got {})",
            want.as_ref()
                .map(|v| format!("Ok(rank={})", v.2))
                .unwrap_or_else(|e| e.code().to_string()),
            got.as_ref()
                .map(|v| format!("Ok(rank={})", v.2))
                .unwrap_or_else(|e| e.code().to_string())
        ),
    }
}

/// F2-false-acceptance: the review's literal gate configurations. The base
/// path rejects (`ill_conditioned_design` / `rank_deficient_design`) while
/// the fixed 1e-12 band admitted both; the solver must preserve the base
/// decision on both entry points. Parity assertions hold on any platform;
/// the expected codes below are the review-machine outcomes.
#[test]
fn false_acceptance_fixture_matches_base_at_both_gates() {
    let (design, signal) = false_acceptance_fixture();
    let sigma_base = design.clone().svd(false, false).singular_values;
    let cond_base = sigma_base[0] / sigma_base[2];
    println!("fixture base condition: {cond_base:.17e}");
    // f64-shortest forms of the review's midpoint thresholds; separation
    // margins (~1.9e-7 absolute) dwarf literal rounding (~1 ULP).
    let condition_case = JointSolverTolerances {
        max_condition: 8.545491453992215e4,
        ..Default::default()
    };
    assert_parity_with_old_path(&design, &signal, condition_case, "condition gate");
    let rank_case = JointSolverTolerances {
        rank_tol: 1.1702077117318137e-5,
        ..Default::default()
    };
    assert_parity_with_old_path(&design, &signal, rank_case, "rank gate");
}

/// Portable midpoint thresholds: derived at runtime from the two
/// diagnostics on the executing machine, so the threshold always sits
/// between the base and thin-R values whenever they differ, on any
/// platform. Both gates, both entry points, plus one-ULP neighbours of
/// each base boundary.
#[test]
fn midpoint_thresholds_match_base_on_both_apis() {
    let (design, signal) = false_acceptance_fixture();
    let sigma_base = design.clone().svd(false, false).singular_values;
    let sigma_r = design.clone().qr().r().svd(true, false).singular_values;
    let cond_base = sigma_base[0] / sigma_base[2];
    let cond_r = sigma_r[0] / sigma_r[2];
    assert!(cond_base.is_finite() && cond_r.is_finite() && cond_base > 0.0 && cond_r > 0.0);
    println!("base condition {cond_base:.17e}, thin-R condition {cond_r:.17e}");
    for limit in [
        cond_base.next_down(),
        cond_base,
        cond_base.next_up(),
        (cond_base + cond_r) / 2.0,
    ] {
        let tol = JointSolverTolerances {
            max_condition: limit,
            ..Default::default()
        };
        assert_parity_with_old_path(
            &design,
            &signal,
            tol,
            &format!("condition limit {limit:.17e}"),
        );
    }
    let ratio_base = sigma_base[2] / sigma_base[0];
    let ratio_r = sigma_r[2] / sigma_r[0];
    for rank_tol in [
        ratio_base.next_down(),
        ratio_base,
        ratio_base.next_up(),
        (ratio_base + ratio_r) / 2.0,
    ] {
        let tol = JointSolverTolerances {
            rank_tol,
            max_condition: 1e16,
            ..Default::default()
        };
        assert_parity_with_old_path(&design, &signal, tol, &format!("rank_tol {rank_tol:.17e}"));
    }
}

/// More ill-conditioned full-rank variants: near-dependent columns with
/// tunable dependence `delta`. Each variant must preserve the base
/// admission decision at runtime-derived midpoint thresholds on both
/// entry points.
#[test]
fn near_dependent_variants_match_base() {
    let (_, signal) = false_acceptance_fixture();
    let base = false_acceptance_fixture().0;
    let col0: Vec<f64> = (0..5).map(|r| base[(r, 0)]).collect();
    let col2: Vec<f64> = (0..5).map(|r| base[(r, 2)]).collect();
    for delta in [1e-4, 1e-6, 1e-8] {
        let mut variant = base.clone();
        for r in 0..5 {
            variant[(r, 2)] = col0[r] + delta * (col2[r] - col0[r]);
        }
        let sigma_base = variant.clone().svd(false, false).singular_values;
        let sigma_r = variant.clone().qr().r().svd(true, false).singular_values;
        let cond_base = sigma_base[0] / sigma_base[2];
        let cond_r = sigma_r[0] / sigma_r[2];
        assert!(cond_base.is_finite() && cond_base > 0.0);
        println!("delta={delta:e} base condition {cond_base:.17e}, thin-R condition {cond_r:.17e}");
        let cond_tol = JointSolverTolerances {
            max_condition: (cond_base + cond_r) / 2.0,
            ..Default::default()
        };
        assert_parity_with_old_path(
            &variant,
            &signal,
            cond_tol,
            &format!("delta={delta:e} condition"),
        );
        let ratio_base = sigma_base[2] / sigma_base[0];
        let ratio_r = sigma_r[2] / sigma_r[0];
        let rank_tol = JointSolverTolerances {
            rank_tol: (ratio_base + ratio_r) / 2.0,
            max_condition: 1e16,
            ..Default::default()
        };
        assert_parity_with_old_path(
            &variant,
            &signal,
            rank_tol,
            &format!("delta={delta:e} rank"),
        );
    }
}

/// Ordinary controls: deterministic pseudo-random full-rank inputs far from
/// any gate must keep bit-identical beta/residual/rank with the old path
/// and 1e-12-bounded condition on both entry points (fast path, no fallback
/// expected).
#[test]
fn ordinary_controls_match_old_path_bitwise() {
    let mut state = 0x11223344u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state as f64 / u64::MAX as f64 - 0.5
    };
    for trial in 0..100 {
        let design = DMatrix::from_fn(9, 4, |_, _| next());
        let signal = DVector::from_element(9, 1.0);
        assert_parity_with_old_path(
            &design,
            &signal,
            JointSolverTolerances::default(),
            &format!("ordinary control {trial}"),
        );
    }
    println!("100 ordinary 9x4 controls: parity with old path on both entry points");
}
