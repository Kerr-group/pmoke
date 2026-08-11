use super::*;
use std::f64::consts::PI;

fn test_lockin() -> Lockin {
    Lockin {
        workers: 1,
        stride_samples: 10,
        lpf_kind: LockinLpfKind::BoxcarLegacy,
        lpf_half_window_cycles: 1.0,
        lpf_debug_output: false,
        lpf_debug_label: None,
        lpf_debug_overwrite: false,
        snr_background_window: None,
        snr_signal_window: None,
        save_npy: false,
    }
}

fn test_waveform() -> (Vec<f64>, Vec<f64>) {
    let dt = 1.0e-5;
    let f_ref = 1_000.0;
    let time = (0..20_000)
        .map(|index| index as f64 * dt)
        .collect::<Vec<_>>();
    let signal = time
        .iter()
        .map(|&t| 0.7 * (2.0 * PI * f_ref * t + 0.3).sin())
        .collect::<Vec<_>>();
    (time, signal)
}

#[test]
fn boxcar_processor_has_stable_trimmed_range() {
    let (time, signal) = test_waveform();
    let processor = LockinProcessor::new(&time, &signal, 1_000.0, 0.0, &test_lockin()).unwrap();

    assert_eq!(processor.params().lpf_kind, LockinLpfKind::BoxcarLegacy);
    assert_eq!(processor.base_index_range(), processor.output_index_range());
    assert_eq!(
        processor.output_times().len(),
        processor.output_index_range().1 - processor.output_index_range().0 + 1
    );
}

#[test]
fn boxcar_processor_returns_finite_harmonics_without_filter_debug_data() {
    let (time, signal) = test_waveform();
    let processor = LockinProcessor::new(&time, &signal, 1_000.0, 0.0, &test_lockin()).unwrap();
    let result = processor.compute_harmonic_detailed(1, true);

    assert!(result.mixed_signal.is_none());
    assert_eq!(result.li_x.len(), result.li_y.len());
    assert!(!result.li_x.is_empty());
    assert!(result.li_x.iter().all(|value| value.is_finite()));
    assert!(result.li_y.iter().all(|value| value.is_finite()));
}

#[test]
fn boxcar_processor_rejects_non_finite_signal() {
    let (time, mut signal) = test_waveform();
    signal[100] = f64::NAN;
    let error = LockinProcessor::new(&time, &signal, 1_000.0, 0.0, &test_lockin())
        .err()
        .unwrap();
    assert!(error.to_string().contains("finite"));
}

#[test]
fn boxcar_processor_rejects_trace_without_output_samples() {
    let time = (0..20)
        .map(|index| index as f64 * 1.0e-5)
        .collect::<Vec<_>>();
    let signal = vec![0.0; time.len()];
    let error = LockinProcessor::new(&time, &signal, 1_000.0, 0.0, &test_lockin())
        .err()
        .unwrap();
    assert!(error.to_string().contains("output range is empty"));
}

#[test]
fn legacy_boxcar_response_is_unity_at_zero_frequency() {
    let (time, signal) = test_waveform();
    let processor = LockinProcessor::new(&time, &signal, 1_000.0, 0.0, &test_lockin()).unwrap();
    let params = processor.params();

    assert!((legacy_boxcar_response_abs(params, 0.0) - 1.0).abs() < 1.0e-12);
    assert!(legacy_boxcar_enbw_hz(params).is_finite());
    assert!(legacy_boxcar_enbw_hz(params) > 0.0);
}
