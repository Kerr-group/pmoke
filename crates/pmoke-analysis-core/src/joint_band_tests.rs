use super::*;

fn stationary_noise(dt: f64, lags: Vec<f64>) -> NoiseModel {
    NoiseModel {
        mode: NoiseMode::StationaryCorrelated,
        reference_variance_v2: 1.0,
        variance_bins: None,
        correlation: Some(CorrelationKernel {
            lags,
            lag_step_s: dt,
        }),
    }
}

fn assert_same_bits(left: &[f64], right: &[f64]) {
    assert_eq!(left.len(), right.len());
    for (index, (&left, &right)) in left.iter().zip(right).enumerate() {
        assert_eq!(left.to_bits(), right.to_bits(), "entry {index}");
    }
}

#[test]
fn prepared_whitening_matches_dense_for_both_correlated_modes() {
    let rows = 73;
    let dt = 1.0e-6;
    let times: Vec<_> = (0..rows).map(|row| row as f64 * dt).collect();
    let signal: Vec<_> = times
        .iter()
        .map(|t| 1.0e7 + 1.0e-6 * (TAU * 1.0e5 * t).sin())
        .collect();
    let design = DMatrix::from_fn(rows, 3, |row, column| match column {
        0 => 1.0,
        1 => (TAU * 1.0e5 * times[row]).cos(),
        _ => (TAU * 1.0e5 * times[row]).sin(),
    });
    let tolerances = JointSolverTolerances::default();
    for (lags, expected_bandwidth) in [
        (vec![1.0], Some(0)),
        (vec![1.0, 0.25, -0.125, 0.0], Some(2)),
        (
            (0..rows).map(|lag| 0.75_f64.powi(lag as i32)).collect(),
            None,
        ),
    ] {
        for mode in [NoiseMode::StationaryCorrelated, NoiseMode::PhaseCorrelated] {
            let noise = NoiseModel {
                mode,
                reference_variance_v2: 0.75,
                variance_bins: (mode == NoiseMode::PhaseCorrelated)
                    .then(|| vec![1.0, 0.75, 1.5, 0.625]),
                correlation: Some(CorrelationKernel {
                    lags: lags.clone(),
                    lag_step_s: dt,
                }),
            };
            let plan = PreparedNoisePlan::prepare(&noise, rows, tolerances).unwrap();
            let prepared = plan.whiten(&design, &signal, &times, 1.0e5, 0.2).unwrap();
            let dense = whiten(&design, &signal, 0.2, 1.0e5, &times, &noise, tolerances).unwrap();
            assert_eq!(
                plan.correlation.as_ref().unwrap().lower_bandwidth,
                expected_bandwidth
            );
            assert_same_bits(prepared.design.as_slice(), dense.design.as_slice());
            assert_same_bits(prepared.response.as_slice(), dense.response.as_slice());
            assert_same_bits(
                &[prepared.variance_scale, prepared.jitter_applied_v2],
                &[dense.variance_scale, dense.jitter_applied_v2],
            );
        }
    }
}

#[test]
fn prepared_whitening_preserves_signed_zeros() {
    let dt = 1.0e-6;
    let times = [0.0, dt, 2.0 * dt, 3.0 * dt];
    let noise = stationary_noise(dt, vec![1.0]);
    let tol = JointSolverTolerances::default();
    let plan = PreparedNoisePlan::prepare(&noise, times.len(), tol).unwrap();
    assert_eq!(plan.correlation.as_ref().unwrap().lower_bandwidth, Some(0));
    // The first off-diagonal +0 times -1 is -0; subtracting it from
    // a -0 RHS gives +0. Skipping that multiply would retain -0.
    let signal = [-1.0, -0.0, 0.0, -f64::from_bits(1)];
    let design = DMatrix::from_column_slice(4, 1, &[1.0, -0.0, 0.0, f64::from_bits(1)]);
    let dense = whiten(&design, &signal, 0.0, 1.0e5, &times, &noise, tol).unwrap();
    let band = plan.whiten(&design, &signal, &times, 1.0e5, 0.0).unwrap();
    assert_eq!(dense.response[1].to_bits(), 0.0_f64.to_bits());
    assert_eq!(dense.design[(1, 0)].to_bits(), (-0.0_f64).to_bits());
    assert_same_bits(band.design.as_slice(), dense.design.as_slice());
    assert_same_bits(band.response.as_slice(), dense.response.as_slice());
}

#[test]
fn exact_band_includes_subnormals_and_rejects_invalid_factors() {
    let mut lower = DMatrix::identity(4, 4);
    lower[(2, 0)] = -0.0;
    assert_eq!(certified_lower_bandwidth(&lower), Some(0));
    lower[(2, 0)] = f64::from_bits(1);
    assert_eq!(certified_lower_bandwidth(&lower), Some(2));
    lower[(3, 0)] = f64::from_bits(1);
    assert_eq!(certified_lower_bandwidth(&lower), None);
    lower[(3, 0)] = 0.0;
    // The original solver rejects nonfinite entries even above the diagonal.
    lower[(0, 3)] = f64::NAN;
    assert_eq!(certified_lower_bandwidth(&lower), None);
    lower[(0, 3)] = 0.0;
    lower[(3, 3)] = 0.0;
    assert_eq!(certified_lower_bandwidth(&lower), None);
}

#[test]
fn jittered_factor_keeps_the_exact_band() {
    let dt = 1.0e-6;
    let times: Vec<_> = (0..7).map(|i| i as f64 * dt).collect();
    let noise = stationary_noise(dt, vec![1.0, 0.75]);
    let tol = JointSolverTolerances {
        max_jitter_v2: 0.5,
        ..JointSolverTolerances::default()
    };
    let signal = [1.0, -0.25, 0.375, -0.5, 0.0, 0.2, -0.2];
    let design = DMatrix::from_column_slice(7, 1, &[1.0; 7]);
    let plan = PreparedNoisePlan::prepare(&noise, 7, tol).unwrap();
    assert_eq!(plan.correlation.as_ref().unwrap().lower_bandwidth, Some(1));
    let dense = whiten(&design, &signal, 0.0, 1.0e5, &times, &noise, tol).unwrap();
    let band = plan.whiten(&design, &signal, &times, 1.0e5, 0.0).unwrap();
    assert_eq!(dense.jitter_applied_v2.to_bits(), 0.5_f64.to_bits());
    assert_eq!(
        band.jitter_applied_v2.to_bits(),
        dense.jitter_applied_v2.to_bits()
    );
    assert_same_bits(band.design.as_slice(), dense.design.as_slice());
    assert_same_bits(band.response.as_slice(), dense.response.as_slice());
}

#[test]
fn banded_whitening_preserves_nonfinite_error_codes() {
    let dt = 1.0e-6;
    let times = [0.0, dt, 2.0 * dt, 3.0 * dt];
    let mut noise = stationary_noise(dt, vec![1.0, 0.375]);
    let tol = JointSolverTolerances::default();
    let signal = [f64::MAX, -f64::MAX, 0.0, 0.0];
    let design = DMatrix::from_column_slice(4, 1, &[1.0; 4]);
    for (variance, expected) in [(0.25, "non_finite_input"), (1.0, "non_finite_output")] {
        noise.reference_variance_v2 = variance;
        let plan = PreparedNoisePlan::prepare(&noise, times.len(), tol).unwrap();
        assert_eq!(plan.correlation.as_ref().unwrap().lower_bandwidth, Some(1));
        let dense = whiten(&design, &signal, 0.0, 1.0e5, &times, &noise, tol).unwrap_err();
        let band = plan
            .whiten(&design, &signal, &times, 1.0e5, 0.0)
            .unwrap_err();
        assert_eq!(dense.code(), expected);
        assert_eq!(band.code(), expected);
        assert_eq!(band.message(), dense.message());
    }
}
