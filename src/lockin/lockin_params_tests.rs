use super::*;

fn test_lockin() -> Lockin {
    Lockin {
        workers: 1,
        stride_samples: 1,
        lpf_kind: LockinLpfKind::FirZeroPhase,
        lpf_half_window_cycles: 1.0,
        lpf_cutoff_hz: Some(10.0),
        lpf_cutoff_ref_ratio: None,
        lpf_stopband_atten_db: 60.0,
        lpf_sync_average_cycles: 1.0,
        lpf_iir_order: 2,
        lpf_debug_output: false,
        lpf_debug_label: None,
        lpf_debug_overwrite: false,
        snr_background_window: None,
        snr_signal_window: None,
    }
}

#[test]
fn rejects_half_window_shorter_than_one_sample() {
    let lockin = test_lockin();
    let err = LockinParams::init(1.0e-3, 10_000, 2_000.0, &lockin).unwrap_err();
    assert!(err.to_string().contains("half-window"));
}

#[test]
fn accepts_half_window_at_one_sample() {
    let lockin = test_lockin();
    let params = LockinParams::init(1.0e-3, 10_000, 1_000.0, &lockin).unwrap();
    assert_eq!(params.n_half, 1);
}
