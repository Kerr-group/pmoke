use super::*;
use crate::config::{
    Channel, ConfigDiagnostic, DiagnosticKind, Fetch, Kerr, KerrType, Lockin, LockinLpfKind, Phase,
    Plot, Pulse, Reference, Roles, Screenshot, Window,
};

fn test_app() -> MonitorApp {
    MonitorApp::new(
        "config.toml".to_string(),
        ConfigLoad::Diagnostics(ConfigDiagnostics {
            version: None,
            warnings: Vec::new(),
            diagnostics: Vec::new(),
            normalized: None,
        }),
    )
}

fn ready_test_app(channel_count: u8) -> MonitorApp {
    let signal_ch = (1..=channel_count.min(6)).collect::<Vec<_>>();
    let window = Window {
        start: 0.0,
        end: 1.0,
    };
    MonitorApp::new(
        "config.toml".to_string(),
        ConfigLoad::Ready {
            config: Config {
                version: 3,
                instruments: None,
                fetch: Fetch::default(),
                screenshot: Screenshot::default(),
                plot: Plot::default(),
                source_path: "config.toml".into(),
                legacy_timebase: None,
                roles: Roles {
                    sensor_ch: vec![1],
                    reference_ch: 1,
                    signal_ch,
                },
                channels: (1..=channel_count)
                    .map(|index| Channel {
                        index,
                        factor: None,
                        scale_to_abs_max: None,
                        label: Some(format!("channel {index}")),
                        unit_out: None,
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
            },
            warnings: Vec::new(),
        },
    )
}

mod interaction;
mod output;
mod timeline;
mod view;
