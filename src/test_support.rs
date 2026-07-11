use crate::config::{
    Channel, Config, Fetch, Kerr, KerrType, Lockin, LockinLpfKind, Phase, Plot, Pulse, Reference,
    Roles, Screenshot, Window,
};

pub fn test_config(sensor_ch: Vec<u8>, signal_ch: Vec<u8>) -> Config {
    let window = Window {
        start: 0.0,
        end: 1.0,
    };
    let max_ch = sensor_ch
        .iter()
        .chain(signal_ch.iter())
        .copied()
        .max()
        .unwrap_or(1);

    Config {
        version: 3,
        instruments: None,
        fetch: Fetch::default(),
        screenshot: Screenshot::default(),
        plot: Plot {
            enabled: false,
            ..Plot::default()
        },
        source_path: "config.toml".into(),
        source_text: None,
        artifact_root: None,
        plot_output_relative: None,
        legacy_timebase: None,
        force: false,
        staging_active: false,
        roles: Roles {
            sensor_ch,
            reference_ch: 1,
            signal_ch,
        },
        channels: (1..=max_ch)
            .map(|index| Channel {
                index,
                factor: Some(index as f64),
                scale_to_abs_max: None,
                label: Some(format!("ch{index}")),
                unit_out: Some("T".to_string()),
            })
            .collect(),
        pulse: Pulse {
            bg_window_before: window,
            bg_window_after: window,
        },
        reference: Reference {
            fft_window: window,
            stride_samples: 1,
            window_samples: 1,
        },
        lockin: Lockin {
            workers: 1,
            stride_samples: 1,
            lpf_kind: LockinLpfKind::FirZeroPhase,
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
        },
        phase: Phase {
            m_omega_t0_offset: Vec::new(),
        },
        kerr: Kerr {
            use_sensor_ch: 1,
            kerr_type: KerrType::Standard,
            factor: 1.0,
        },
    }
}
