use super::*;
use crate::config::LockinLpfKind;
use crate::lockin::lockin_params::CutoffSource;

fn test_params() -> LockinParams {
    LockinParams {
        dt: 1.0e-4,
        sample_rate: 10_000.0,
        output_rate: 10_000.0,
        stride: 1,
        length: 20_000,
        f_ref: 1_000.0,
        lpf_kind: LockinLpfKind::FirBoxcarEnbw,
        lpf_stopband_atten_db: 60.0,
        lpf_sync_average_cycles: 1.0,
        lpf_iir_order: 2,
        cutoff_hz: None,
        cutoff_source: CutoffSource::EnbwMatch,
        fallback_used: false,
        omega: 2.0 * PI * 1_000.0,
        t_half: 0.001,
        n_half: 10,
        i_start: 13,
        i_end: 19_987,
    }
}

#[test]
fn legacy_boxcar_weights_have_unit_dc_gain() {
    let weights = legacy_boxcar_weights(test_params());
    let sum: f64 = weights.iter().sum();
    assert!((sum - 1.0).abs() < 1.0e-12);
}

#[test]
fn fir_boxcar_enbw_search_matches_reachable_target() {
    let params = test_params();
    let beta = kaiser_beta(params.lpf_stopband_atten_db);
    let min_cutoff = (params.sample_rate / 1.0e12).max(f64::MIN_POSITIVE);
    let max_cutoff = 0.45 * params.output_rate;
    let min_enbw = enbw_hz(
        &design_kaiser_lowpass_taps(params, min_cutoff, beta),
        params.sample_rate,
    );
    let max_enbw = enbw_hz(
        &design_kaiser_lowpass_taps(params, max_cutoff, beta),
        params.sample_rate,
    );
    let target = 0.5 * (min_enbw + max_enbw);
    let matched = match_boxcar_enbw_cutoff(params, beta, target);
    assert!(matched.reachable);

    let taps = design_kaiser_lowpass_taps(params, matched.cutoff_hz, beta);
    let actual = enbw_hz(&taps, params.sample_rate);
    let rel_err = (actual - target).abs() / target;
    assert!(rel_err < 1.0e-6, "relative error: {rel_err}");
}

#[test]
fn fir_boxcar_enbw_legacy_target_reachability_is_reported() {
    let params = test_params();
    let beta = kaiser_beta(params.lpf_stopband_atten_db);
    let min_cutoff = (params.sample_rate / 1.0e12).max(f64::MIN_POSITIVE);
    let max_cutoff = 0.45 * params.output_rate;
    let target = enbw_hz(&legacy_boxcar_weights(params), params.sample_rate);
    let min_enbw = enbw_hz(
        &design_kaiser_lowpass_taps(params, min_cutoff, beta),
        params.sample_rate,
    );
    let max_enbw = enbw_hz(
        &design_kaiser_lowpass_taps(params, max_cutoff, beta),
        params.sample_rate,
    );
    let matched = match_boxcar_enbw_cutoff(params, beta, target);

    assert_eq!(
        matched.reachable,
        target > min_enbw && target < max_enbw,
        "target={target}, min={min_enbw}, max={max_enbw}"
    );
}

#[test]
fn fir_boxcar_enbw_unreachable_low_target_uses_min_cutoff() {
    let params = test_params();
    let beta = kaiser_beta(params.lpf_stopband_atten_db);
    let min_cutoff = (params.sample_rate / 1.0e12).max(f64::MIN_POSITIVE);
    let min_enbw = enbw_hz(
        &design_kaiser_lowpass_taps(params, min_cutoff, beta),
        params.sample_rate,
    );
    let matched = match_boxcar_enbw_cutoff(params, beta, 0.5 * min_enbw);

    assert!(!matched.reachable);
    assert_eq!(matched.cutoff_hz, min_cutoff);
}

#[test]
fn butterworth_response_is_half_power_at_cutoff() {
    let sample_rate = 100_000.0;
    let cutoff = 1_000.0;
    let sos = design_butterworth_lowpass_sos(4, cutoff, sample_rate).unwrap();
    let response = iir_response_abs(&sos, sample_rate, cutoff);
    assert!((response - 1.0 / 2.0_f64.sqrt()).abs() < 1.0e-10);
}

#[test]
fn zero_phase_design_cutoff_compensates_half_power_point() {
    let sample_rate = 100_000.0;
    let requested_cutoff = 1_000.0;
    let order = 4;
    let design_cutoff = zero_phase_butterworth_design_cutoff(requested_cutoff, order);
    let sos = design_butterworth_lowpass_sos(order, design_cutoff, sample_rate).unwrap();
    let response = iir_response_abs(&sos, sample_rate, requested_cutoff).powi(2);
    assert!((response - 1.0 / 2.0_f64.sqrt()).abs() < 1.0e-3);
}

#[test]
fn centered_moving_average_preserves_constant_signal() {
    let input = vec![Complex64::new(2.0, -3.0); 101];
    let output = centered_moving_average_complex(&input, 7);
    assert!(output
        .iter()
        .all(|&value| value == Complex64::new(2.0, -3.0)));
}

fn centered_moving_average_complex(values: &[Complex64], len: usize) -> Vec<Complex64> {
    let mut out = values.to_vec();
    apply_centered_moving_average_complex_in_place(&mut out, len);
    out
}

#[test]
fn centered_moving_average_in_place_matches_reference() {
    let input = (0..29)
        .map(|idx| Complex64::new(idx as f64, -(idx as f64) * 0.25))
        .collect::<Vec<_>>();
    let len = 6;
    let left = (len - 1) / 2;
    let right = len - left;
    let expected = (0..input.len())
        .map(|idx| {
            let start = idx.saturating_sub(left);
            let end = (idx + right).min(input.len());
            input[start..end].iter().copied().sum::<Complex64>() / (end - start) as f64
        })
        .collect::<Vec<_>>();

    let mut actual = input;
    apply_centered_moving_average_complex_in_place(&mut actual, len);

    assert_eq!(actual, expected);
}

#[test]
fn legacy_boxcar_prefix_path_matches_direct_integration() {
    let lockin = Lockin {
        workers: 1,
        stride_samples: 7,
        lpf_kind: LockinLpfKind::BoxcarLegacy,
        lpf_half_window_cycles: 2.25,
        lpf_cutoff_hz: None,
        lpf_cutoff_ref_ratio: None,
        lpf_stopband_atten_db: 60.0,
        lpf_sync_average_cycles: 1.0,
        lpf_iir_order: 2,
        lpf_debug_output: false,
        lpf_debug_label: None,
        lpf_debug_overwrite: false,
        snr_background_window: None,
        snr_signal_window: None,
    };
    let dt = 1.0e-5;
    let f_ref = 1_000.0;
    let t = (0..5_000).map(|idx| idx as f64 * dt).collect::<Vec<_>>();
    let data = t
        .iter()
        .map(|&ti| {
            0.2 * (2.0 * PI * 1_000.0 * ti).sin()
                + 0.07 * (2.0 * PI * 2_000.0 * ti + 0.3).cos()
                + 0.01 * ti
        })
        .collect::<Vec<_>>();

    let processor = LockinProcessor::new(&t, &data, f_ref, 0.17, &lockin).unwrap();
    let actual = processor.compute_harmonic_detailed(2, false);
    let expected_x = direct_legacy_lockin(&processor, 2, RefType::Sin);
    let expected_y = direct_legacy_lockin(&processor, 2, RefType::Cos);

    assert_eq!(actual.li_x.len(), expected_x.len());
    assert_eq!(actual.li_y.len(), expected_y.len());
    let max_x_err = actual
        .li_x
        .iter()
        .zip(expected_x.iter())
        .map(|(actual, expected)| (actual - expected).abs())
        .fold(0.0, f64::max);
    let max_y_err = actual
        .li_y
        .iter()
        .zip(expected_y.iter())
        .map(|(actual, expected)| (actual - expected).abs())
        .fold(0.0, f64::max);

    assert!(max_x_err < 1.0e-13, "max_x_err={max_x_err}");
    assert!(max_y_err < 1.0e-13, "max_y_err={max_y_err}");
}

fn direct_legacy_lockin(
    processor: &LockinProcessor<'_>,
    harmonic: usize,
    ref_type: RefType,
) -> Vec<f64> {
    let mixed_signal: Vec<f64> = processor
        .t
        .iter()
        .zip(processor.data.iter())
        .map(|(&t, &data)| data * processor.ref_signal(t, harmonic, ref_type))
        .collect();

    let i_start = processor.params.i_start;
    let i_end = processor.params.i_end;
    let mut out = Vec::with_capacity(i_end - i_start + 1);

    for i_idx in i_start..=i_end {
        let i_base = i_idx * processor.params.stride;

        let mut integ = 0.0;
        for j in 0..(2 * processor.params.n_half) {
            let j0 = j as isize - processor.params.n_half as isize;
            let j1 = j0 + 1;
            let idx0 = (i_base as isize + j0) as usize;
            let idx1 = (i_base as isize + j1) as usize;

            let f0 = mixed_signal[idx0];
            let f1 = mixed_signal[idx1];

            integ += 0.5 * (f0 + f1) * processor.params.dt;
        }

        let neg_idx0 = i_base - processor.params.n_half;
        let neg_idx1 = i_base - processor.params.n_half - 1;

        let y0_neg = mixed_signal[neg_idx0];
        let y1_neg = mixed_signal[neg_idx1];

        let edge_dt =
            processor.params.t_half - (processor.params.n_half as f64) * processor.params.dt;
        let ym_neg =
            (y1_neg * edge_dt + y0_neg * (processor.params.dt - edge_dt)) / processor.params.dt;
        let edge_neg = edge_dt * 0.5 * (y0_neg + ym_neg);

        let pos_idx0 = i_base + processor.params.n_half;
        let pos_idx1 = i_base + processor.params.n_half + 1;

        let y0_pos = mixed_signal[pos_idx0];
        let y1_pos = mixed_signal[pos_idx1];

        let ym_pos =
            (y1_pos * edge_dt + y0_pos * (processor.params.dt - edge_dt)) / processor.params.dt;
        let edge_pos = edge_dt * 0.5 * (y0_pos + ym_pos);

        out.push((integ + edge_neg + edge_pos) / (2.0 * processor.params.t_half));
    }

    out
}

#[test]
fn rejects_empty_output_range_after_iir_settling_trim() {
    let mut lockin = Lockin {
        workers: 1,
        stride_samples: 1,
        lpf_kind: LockinLpfKind::SyncIirZeroPhase,
        lpf_half_window_cycles: 1.0,
        lpf_cutoff_hz: Some(1.0),
        lpf_cutoff_ref_ratio: None,
        lpf_stopband_atten_db: 60.0,
        lpf_sync_average_cycles: 1.0,
        lpf_iir_order: 2,
        lpf_debug_output: false,
        lpf_debug_label: None,
        lpf_debug_overwrite: false,
        snr_background_window: None,
        snr_signal_window: None,
    };
    let dt = 1.0e-4;
    let t = (0..1000).map(|idx| idx as f64 * dt).collect::<Vec<_>>();
    let data = vec![0.0; t.len()];

    let err = match LockinProcessor::new(&t, &data, 1_000.0, 0.0, &lockin) {
        Ok(_) => panic!("expected empty output range error"),
        Err(err) => err,
    };
    assert!(err.to_string().contains("output range is empty"));

    lockin.lpf_cutoff_hz = Some(1_000.0);
    LockinProcessor::new(&t, &data, 1_000.0, 0.0, &lockin).unwrap();
}

#[test]
fn narrow_iir_filtfilt_preserves_constant_signal() {
    let sos = design_butterworth_lowpass_sos(4, 20_000.0, 2.0e9).unwrap();
    let mut values = vec![Complex64::new(1.25, -0.5); 10_000];
    apply_sos_filtfilt_in_place(&mut values, &sos);
    let max_err = values
        .iter()
        .map(|&value| (value - Complex64::new(1.25, -0.5)).norm())
        .fold(0.0, f64::max);
    assert!(max_err < 1.0e-6, "max_err={max_err}");
}

#[test]
fn enbw_matches_white_noise_variance_reduction() {
    let params = test_params();
    let weights = legacy_boxcar_weights(params);
    let enbw = enbw_hz(&weights, params.sample_rate);
    let expected_ratio = enbw / params.sample_rate;

    let mut state = 0x1234_5678_9abc_def0_u64;
    let mut input = Vec::with_capacity(50_000);
    for _ in 0..50_000 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let unit = ((state >> 11) as f64) / ((1_u64 << 53) as f64);
        input.push((unit - 0.5) * 12.0_f64.sqrt());
    }

    let input_var = population_variance(&input);
    let mut output = Vec::new();
    for idx in 0..(input.len() - weights.len()) {
        let y = weights
            .iter()
            .zip(input[idx..].iter())
            .map(|(w, x)| w * x)
            .sum::<f64>();
        output.push(y);
    }
    let output_var = population_variance(&output);
    let actual_ratio = output_var / input_var;

    assert!(
        (actual_ratio - expected_ratio).abs() / expected_ratio < 0.15,
        "actual={actual_ratio}, expected={expected_ratio}"
    );
}

fn population_variance(values: &[f64]) -> f64 {
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    values
        .iter()
        .map(|value| {
            let diff = value - mean;
            diff * diff
        })
        .sum::<f64>()
        / values.len() as f64
}
