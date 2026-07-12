use super::*;

#[derive(Debug, Deserialize)]
pub(super) struct ConfigV1 {
    #[allow(dead_code)]
    pub(super) version: u32,
    pub(super) instruments: Option<InstrumentsV1>,
    #[serde(default)]
    pub(super) screenshot: ScreenshotV3,
    pub(super) timebase: TimebaseV1,
    pub(super) roles: RolesV1,
    pub(super) channels: Vec<ChannelV1>,
    pub(super) pulse: PulseV1,
    pub(super) reference: ReferenceV1,
    pub(super) lockin: LockinV1,
    pub(super) phase: PhaseV1,
    pub(super) kerr: KerrV1,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConfigV2 {
    #[allow(dead_code)]
    pub(super) version: u32,
    pub(super) instruments: Option<InstrumentsV2>,
    #[serde(default)]
    pub(super) fetch: FetchV2,
    #[serde(default)]
    pub(super) screenshot: ScreenshotV3,
    #[serde(default)]
    pub(super) plot: PlotV2,
    pub(super) timebase: TimebaseV2,
    pub(super) roles: RolesV2,
    pub(super) channels: Vec<ChannelV2>,
    pub(super) pulse: PulseV2,
    pub(super) reference: ReferenceV2,
    pub(super) lockin: LockinV2,
    pub(super) phase: PhaseV2,
    pub(super) kerr: KerrV2,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConfigV3 {
    pub(super) version: u32,
    pub(super) instruments: Option<InstrumentsV2>,
    #[serde(default)]
    pub(super) fetch: FetchV2,
    #[serde(default)]
    pub(super) screenshot: ScreenshotV3,
    #[serde(default)]
    pub(super) plot: PlotV2,
    pub(super) roles: RolesV2,
    pub(super) channels: Vec<ChannelV2>,
    pub(super) pulse: PulseV2,
    pub(super) reference: ReferenceV2,
    pub(super) lockin: LockinV2,
    pub(super) phase: PhaseV2,
    pub(super) kerr: KerrV2,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ConfigV4 {
    pub(super) version: u32,
    pub(super) scope: ScopeV4,
    #[serde(default)]
    pub(super) generator: Option<GeneratorV4>,
    pub(super) data: DataV4,
    #[serde(default)]
    pub(super) sensors: Vec<SensorV4>,
    pub(super) pulse: PulseV4,
    pub(super) reference: ReferenceV4,
    pub(super) lockin: LockinV4,
    pub(super) phase: PhaseV4,
    pub(super) kerr: KerrV4,
    #[serde(default)]
    pub(super) plot: PlotV4,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ScopeV4 {
    pub(super) model: String,
    pub(super) connection: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GeneratorV4 {
    pub(super) model: String,
    pub(super) connection: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DataV4 {
    pub(super) output: DataOutputV4,
    pub(super) input: FetchAnalysisInput,
    #[serde(default)]
    pub(super) screenshot: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum DataOutputV4 {
    Csv,
    Raw,
    Both,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SensorV4 {
    pub(super) channel: u8,
    pub(super) scale: SensorScaleV4,
    pub(super) label: String,
    pub(super) unit: String,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(super) enum SensorScaleV4 {
    Factor(SensorFactorScaleV4),
    MaxAbs(SensorMaxAbsScaleV4),
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SensorFactorScaleV4 {
    pub(super) factor: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SensorMaxAbsScaleV4 {
    pub(super) max_abs: f64,
    pub(super) polarity: i8,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PulseV4 {
    pub(super) background_before: Window,
    pub(super) background_after: Window,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReferenceV4 {
    pub(super) channel: u8,
    pub(super) fft_window: Window,
    pub(super) stride_samples: usize,
    pub(super) window_samples: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LockinV4 {
    pub(super) signal_channels: Vec<u8>,
    pub(super) workers: usize,
    pub(super) stride_samples: usize,
    pub(super) filter: LockinFilterV4,
    #[serde(default)]
    pub(super) debug_output: bool,
    #[serde(default)]
    pub(super) debug_label: Option<String>,
    #[serde(default)]
    pub(super) debug_overwrite: bool,
    #[serde(default)]
    pub(super) snr_background_window: Option<Window>,
    #[serde(default)]
    pub(super) snr_signal_window: Option<Window>,
    #[serde(default)]
    pub(super) save_npy: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum LockinFilterV4 {
    BoxcarLegacy {
        half_window_cycles: f64,
    },
    FirBoxcarEnbw {
        half_window_cycles: f64,
    },
    FirZeroPhase {
        half_window_cycles: f64,
        #[serde(default)]
        cutoff_hz: Option<f64>,
        #[serde(default)]
        cutoff_ref_ratio: Option<f64>,
        #[serde(default = "default_lockin_stopband_atten_db")]
        stopband_atten_db: f64,
    },
    SyncIirZeroPhase {
        half_window_cycles: f64,
        #[serde(default)]
        cutoff_hz: Option<f64>,
        #[serde(default)]
        cutoff_ref_ratio: Option<f64>,
        #[serde(default = "default_lockin_sync_average_cycles")]
        sync_average_cycles: f64,
        #[serde(default = "default_lockin_iir_order")]
        iir_order: usize,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PhaseV4 {
    #[serde(deserialize_with = "de_vec_f64_or_expr")]
    pub(super) offsets: Vec<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KerrV4 {
    pub(super) sensor: u8,
    pub(super) method: KerrType,
    pub(super) factor: f64,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum PlotModeV4 {
    Off,
    #[default]
    Save,
    Interactive,
    Both,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum PlotErrorModeV4 {
    #[default]
    Warn,
    Fail,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct PlotV4 {
    pub(super) mode: PlotModeV4,
    pub(super) output_dir: Option<String>,
    pub(super) max_points: usize,
    pub(super) decimation: PlotDecimation,
    pub(super) on_error: PlotErrorModeV4,
}

impl Default for PlotV4 {
    fn default() -> Self {
        let default = Plot::default();
        Self {
            mode: PlotModeV4::Save,
            output_dir: None,
            max_points: default.max_points,
            decimation: default.decimation,
            on_error: PlotErrorModeV4::Warn,
        }
    }
}

#[derive(Serialize)]
pub(super) struct NormalizedConfigV4 {
    pub(super) version: u32,
    pub(super) scope: ScopeOutputV4,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) generator: Option<GeneratorOutputV4>,
    pub(super) data: DataOutputConfigV4,
    pub(super) sensors: Vec<SensorOutputV4>,
    pub(super) pulse: PulseOutputV4,
    pub(super) reference: ReferenceOutputV4,
    pub(super) lockin: LockinOutputV4,
    pub(super) phase: PhaseOutputV4,
    pub(super) kerr: KerrOutputV4,
    pub(super) plot: PlotOutputV4,
}

#[derive(Serialize)]
pub(super) struct ScopeOutputV4 {
    pub(super) model: String,
    pub(super) connection: String,
}

#[derive(Serialize)]
pub(super) struct GeneratorOutputV4 {
    pub(super) model: String,
    pub(super) connection: String,
}

#[derive(Serialize)]
pub(super) struct DataOutputConfigV4 {
    pub(super) output: DataOutputV4,
    pub(super) input: FetchAnalysisInput,
    pub(super) screenshot: bool,
}

#[derive(Serialize)]
pub(super) struct SensorOutputV4 {
    pub(super) channel: u8,
    pub(super) scale: SensorScaleOutputV4,
    pub(super) label: String,
    pub(super) unit: String,
}

#[derive(Serialize)]
#[serde(untagged)]
pub(super) enum SensorScaleOutputV4 {
    Factor { factor: f64 },
    MaxAbs { max_abs: f64, polarity: i8 },
}

#[derive(Serialize)]
pub(super) struct PulseOutputV4 {
    pub(super) background_before: Window,
    pub(super) background_after: Window,
}

#[derive(Serialize)]
pub(super) struct ReferenceOutputV4 {
    pub(super) channel: u8,
    pub(super) fft_window: Window,
    pub(super) stride_samples: usize,
    pub(super) window_samples: usize,
}

#[derive(Serialize)]
pub(super) struct LockinOutputV4 {
    pub(super) signal_channels: Vec<u8>,
    pub(super) workers: usize,
    pub(super) stride_samples: usize,
    pub(super) filter: LockinFilterOutputV4,
    #[serde(skip_serializing_if = "is_false")]
    pub(super) debug_output: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) debug_label: Option<String>,
    #[serde(skip_serializing_if = "is_false")]
    pub(super) debug_overwrite: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) snr_background_window: Option<Window>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) snr_signal_window: Option<Window>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum LockinFilterOutputV4 {
    BoxcarLegacy {
        half_window_cycles: f64,
    },
    FirBoxcarEnbw {
        half_window_cycles: f64,
    },
    FirZeroPhase {
        half_window_cycles: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        cutoff_hz: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cutoff_ref_ratio: Option<f64>,
        stopband_atten_db: f64,
    },
    SyncIirZeroPhase {
        half_window_cycles: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        cutoff_hz: Option<f64>,
        #[serde(skip_serializing_if = "Option::is_none")]
        cutoff_ref_ratio: Option<f64>,
        sync_average_cycles: f64,
        iir_order: usize,
    },
}

#[derive(Serialize)]
pub(super) struct PhaseOutputV4 {
    pub(super) offsets: Vec<f64>,
}

#[derive(Serialize)]
pub(super) struct KerrOutputV4 {
    pub(super) sensor: u8,
    pub(super) method: KerrType,
    pub(super) factor: f64,
}

#[derive(Serialize)]
pub(super) struct PlotOutputV4 {
    pub(super) mode: PlotModeV4,
    pub(super) max_points: usize,
    pub(super) decimation: PlotDecimation,
    pub(super) on_error: PlotErrorModeV4,
}

fn is_false(value: &bool) -> bool {
    !value
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(super) struct FetchV2 {
    #[serde(default)]
    pub(super) output: FetchOutput,
    #[serde(default)]
    pub(super) analysis_input: FetchAnalysisInput,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct ScreenshotV3 {
    pub(super) enabled: bool,
}

impl Default for ScreenshotV3 {
    fn default() -> Self {
        let default = Screenshot::default();
        Self {
            enabled: default.enabled,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct PlotV2 {
    pub(super) enabled: bool,
    pub(super) save: bool,
    pub(super) interactive: bool,
    pub(super) output_dir: String,
    pub(super) max_points: usize,
    pub(super) decimation: PlotDecimation,
    pub(super) fail_on_error: bool,
}

impl Default for PlotV2 {
    fn default() -> Self {
        let default = Plot::default();
        Self {
            enabled: default.enabled,
            save: default.save,
            interactive: default.interactive,
            output_dir: default.output_dir,
            max_points: default.max_points,
            decimation: default.decimation,
            fail_on_error: default.fail_on_error,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct InstrumentsV1 {
    #[serde(rename = "function_generator")]
    pub(super) function_generator: Option<FunctionGeneratorV1>,
    pub(super) oscilloscope: OscilloscopeV1,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct InstrumentsV2 {
    #[serde(rename = "function_generator")]
    pub(super) function_generator: Option<FunctionGeneratorV2>,
    pub(super) oscilloscope: OscilloscopeV2,
}

#[derive(Debug, Deserialize)]
pub(super) struct FunctionGeneratorV1 {
    pub(super) connection: Connection,
    #[serde(default)]
    pub(super) model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FunctionGeneratorV2 {
    pub(super) connection: Connection,
    #[serde(default)]
    pub(super) model: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct OscilloscopeV1 {
    pub(super) connection: Connection,
    #[serde(default)]
    pub(super) model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OscilloscopeV2 {
    pub(super) connection: Connection,
    #[serde(default)]
    pub(super) model: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct TimebaseV1 {
    pub(super) t0: f64,
    pub(super) dt: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TimebaseV2 {
    pub(super) t0: f64,
    pub(super) dt: f64,
}

#[derive(Debug, Deserialize)]
pub(super) struct RolesV1 {
    #[serde(default, deserialize_with = "one_or_many")]
    pub(super) sensor_ch: Vec<u8>,
    #[serde(default, deserialize_with = "one_or_many")]
    pub(super) reference_ch: Vec<u8>,
    #[serde(default, deserialize_with = "one_or_many")]
    pub(super) signal_ch: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RolesV2 {
    #[serde(default, deserialize_with = "one_or_many")]
    pub(super) sensor_ch: Vec<u8>,
    pub(super) reference_ch: u8,
    #[serde(default, deserialize_with = "one_or_many")]
    pub(super) signal_ch: Vec<u8>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ChannelV1 {
    pub(super) index: u8,
    #[serde(default)]
    pub(super) factor: Option<f64>,
    #[serde(default)]
    pub(super) scale_to_abs_max: Option<f64>,
    #[serde(default)]
    pub(super) label: Option<String>,
    #[serde(default)]
    pub(super) unit_out: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChannelV2 {
    pub(super) index: u8,
    #[serde(default)]
    pub(super) factor: Option<f64>,
    #[serde(default)]
    pub(super) scale_to_abs_max: Option<f64>,
    #[serde(default)]
    pub(super) label: Option<String>,
    #[serde(default)]
    pub(super) unit_out: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct PulseV1 {
    pub(super) bg_window_before: Window,
    pub(super) bg_window_after: Window,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PulseV2 {
    pub(super) bg_window_before: Window,
    pub(super) bg_window_after: Window,
}

#[derive(Debug, Deserialize)]
pub(super) struct ReferenceV1 {
    pub(super) fft_window: Window,
    pub(super) stride_samples: usize,
    pub(super) window_samples: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReferenceV2 {
    pub(super) fft_window: Window,
    pub(super) stride_samples: usize,
    pub(super) window_samples: usize,
}

#[derive(Debug, Deserialize)]
pub(super) struct LockinV1 {
    pub(super) workers: usize,
    pub(super) stride_samples: usize,
    #[serde(default)]
    pub(super) filter_length_samples: Option<usize>,
    #[serde(default)]
    pub(super) demodulation: Option<LockinDemodulationV1>,
    #[serde(default)]
    pub(super) lpf_kind: Option<LockinLpfKind>,
    #[serde(default)]
    pub(super) lpf_half_window_cycles: Option<f64>,
    #[serde(default)]
    pub(super) lpf_cutoff_hz: Option<f64>,
    #[serde(default)]
    pub(super) lpf_cutoff_ref_ratio: Option<f64>,
    #[serde(default = "default_lockin_stopband_atten_db")]
    pub(super) lpf_stopband_atten_db: f64,
    #[serde(default = "default_lockin_sync_average_cycles")]
    pub(super) lpf_sync_average_cycles: f64,
    #[serde(default = "default_lockin_iir_order")]
    pub(super) lpf_iir_order: usize,
    #[serde(default)]
    pub(super) lpf_debug_output: bool,
    #[serde(default)]
    pub(super) lpf_debug_label: Option<String>,
    #[serde(default)]
    pub(super) lpf_debug_overwrite: bool,
    #[serde(default)]
    pub(super) snr_background_window: Option<Window>,
    #[serde(default)]
    pub(super) snr_signal_window: Option<Window>,
    #[serde(default)]
    pub(super) save_npy: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LockinV2 {
    pub(super) workers: usize,
    pub(super) stride_samples: usize,
    #[serde(default)]
    pub(super) lpf_kind: Option<LockinLpfKind>,
    pub(super) lpf_half_window_cycles: f64,
    #[serde(default)]
    pub(super) lpf_cutoff_hz: Option<f64>,
    #[serde(default)]
    pub(super) lpf_cutoff_ref_ratio: Option<f64>,
    #[serde(default = "default_lockin_stopband_atten_db")]
    pub(super) lpf_stopband_atten_db: f64,
    #[serde(default = "default_lockin_sync_average_cycles")]
    pub(super) lpf_sync_average_cycles: f64,
    #[serde(default = "default_lockin_iir_order")]
    pub(super) lpf_iir_order: usize,
    #[serde(default)]
    pub(super) lpf_debug_output: bool,
    #[serde(default)]
    pub(super) lpf_debug_label: Option<String>,
    #[serde(default)]
    pub(super) lpf_debug_overwrite: bool,
    #[serde(default)]
    pub(super) snr_background_window: Option<Window>,
    #[serde(default)]
    pub(super) snr_signal_window: Option<Window>,
    #[serde(default)]
    pub(super) save_npy: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum LockinDemodulationV1 {
    Complex,
}

#[derive(Debug, Deserialize)]
pub(super) struct PhaseV1 {
    #[serde(default, deserialize_with = "one_or_many")]
    pub(super) use_signal_ch: Vec<u8>,
    #[serde(default, deserialize_with = "de_vec_f64_or_expr")]
    pub(super) m_omega_t0_offset: Vec<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PhaseV2 {
    #[serde(default, deserialize_with = "de_vec_f64_or_expr")]
    pub(super) m_omega_t0_offset: Vec<f64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct KerrV1 {
    pub(super) use_sensor_ch: u8,
    pub(super) kerr_type: KerrType,
    pub(super) factor: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KerrV2 {
    pub(super) use_sensor_ch: u8,
    pub(super) kerr_type: KerrType,
    pub(super) factor: f64,
}

pub(super) fn default_lockin_stopband_atten_db() -> f64 {
    60.0
}

pub(super) fn default_lockin_sync_average_cycles() -> f64 {
    1.0
}

pub(super) fn default_lockin_iir_order() -> usize {
    2
}
