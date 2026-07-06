use crate::constants::{
    FETCHED_FNAME, LI_RESULTS_NAME, LI_ROTATED_NAME, RAW_METADATA_FNAME, RAW_WAVEFORM_DIR,
};
use anyhow::{Result, anyhow, bail};
use fasteval::Evaler;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

fn eval_f64_expr(s: &str) -> Result<f64> {
    if contains_print_call(s) {
        bail!("invalid expression '{s}': print() is not allowed in config values");
    }

    let mut slab = fasteval::Slab::new();
    let parser = fasteval::Parser::new();
    let expr = parser
        .parse(s.trim(), &mut slab.ps)
        .map_err(|e| anyhow!("invalid expression '{s}': {e}"))?;

    let mut namespace = BTreeMap::from([("pi".to_string(), std::f64::consts::PI)]);
    expr.from(&slab.ps)
        .eval(&slab, &mut namespace)
        .map_err(|e| anyhow!("failed to evaluate '{s}': {e}"))
}

fn contains_print_call(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if is_expr_ident_start(bytes[i]) {
            let start = i;
            i += 1;
            while i < bytes.len() && is_expr_ident_continue(bytes[i]) {
                i += 1;
            }
            if &s[start..i] == "print" {
                let mut j = i;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if matches!(bytes.get(j), Some(b'(' | b'[')) {
                    return true;
                }
            }
        } else {
            i += 1;
        }
    }
    false
}

fn is_expr_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_expr_ident_continue(b: u8) -> bool {
    is_expr_ident_start(b) || b.is_ascii_digit()
}

fn de_vec_f64_or_expr<'de, D>(de: D) -> std::result::Result<Vec<f64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum NumOrExpr {
        Num(f64),
        Expr(String),
    }

    let xs = Vec::<NumOrExpr>::deserialize(de)?;

    let mut out = Vec::with_capacity(xs.len());
    for x in xs {
        let v = match x {
            NumOrExpr::Num(v) => v,
            NumOrExpr::Expr(s) => eval_f64_expr(&s).map_err(serde::de::Error::custom)?,
        };
        out.push(v);
    }

    Ok(out)
}

fn one_or_many<'de, D>(de: D) -> std::result::Result<Vec<u8>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(u8),
        Many(Vec<u8>),
    }

    Ok(match OneOrMany::deserialize(de)? {
        OneOrMany::One(x) => vec![x],
        OneOrMany::Many(xs) => xs,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct Config {
    pub version: u32,
    pub instruments: Option<Instruments>,
    pub fetch: Fetch,
    pub image: Image,
    pub plot: Plot,
    #[serde(skip_serializing)]
    pub source_path: PathBuf,
    #[serde(skip_serializing)]
    pub legacy_timebase: Option<Timebase>,
    pub roles: Roles,
    pub channels: Vec<Channel>,
    pub pulse: Pulse,
    pub reference: Reference,
    pub lockin: Lockin,
    pub phase: Phase,
    pub kerr: Kerr,
}

#[derive(Debug, Clone, Serialize)]
pub struct Image {
    pub enabled: bool,
    pub scope_path: String,
}

impl Default for Image {
    fn default() -> Self {
        Self {
            enabled: false,
            scope_path: "C:/screenshot.png".to_string(),
        }
    }
}

impl Config {
    pub fn phase_signal_ch(&self) -> &[u8] {
        &self.roles.signal_ch
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FetchOutput {
    #[default]
    Csv,
    Raw,
    CsvAndRaw,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FetchAnalysisInput {
    #[default]
    Csv,
    Raw,
    Auto,
}

#[derive(Debug, Clone, Serialize)]
pub struct Fetch {
    pub output: FetchOutput,
    pub analysis_input: FetchAnalysisInput,
}

impl Default for Fetch {
    fn default() -> Self {
        Self {
            output: FetchOutput::Csv,
            analysis_input: FetchAnalysisInput::Csv,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Plot {
    pub enabled: bool,
    pub save: bool,
    pub interactive: bool,
    pub output_dir: String,
    pub max_points: usize,
    pub decimation: PlotDecimation,
    pub fail_on_error: bool,
}

impl Default for Plot {
    fn default() -> Self {
        Self {
            enabled: true,
            save: true,
            interactive: false,
            output_dir: "plots".to_string(),
            max_points: 100_000,
            decimation: PlotDecimation::Stride,
            fail_on_error: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlotDecimation {
    Stride,
}

#[derive(Debug, Clone, Serialize)]
pub struct Instruments {
    pub function_generator: Option<FunctionGenerator>,
    pub oscilloscope: Oscilloscope,
}

#[derive(Debug, Clone, Serialize)]
pub struct FunctionGenerator {
    pub connection: Connection,
    pub model: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Oscilloscope {
    pub connection: Connection,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "protocol", rename_all = "lowercase")]
pub enum Connection {
    Gpib { board: u8, address: u8 },
    Tcpip { ip: String, port: u16 },
    Usbtmc { resource: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct Timebase {
    pub t0: f64,
    pub dt: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Roles {
    pub sensor_ch: Vec<u8>,
    pub reference_ch: u8,
    pub signal_ch: Vec<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Channel {
    pub index: u8,
    pub factor: Option<f64>,
    pub label: Option<String>,
    pub unit_out: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Pulse {
    pub bg_window_before: Window,
    pub bg_window_after: Window,
}

#[derive(Debug, Clone, Serialize)]
pub struct Reference {
    pub fft_window: Window,
    pub stride_samples: usize,
    pub window_samples: usize,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Window {
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Lockin {
    pub workers: usize,
    pub stride_samples: usize,
    pub lpf_kind: LockinLpfKind,
    pub lpf_half_window_cycles: f64,
    pub lpf_cutoff_hz: Option<f64>,
    pub lpf_cutoff_ref_ratio: Option<f64>,
    pub lpf_stopband_atten_db: f64,
    pub lpf_sync_average_cycles: f64,
    pub lpf_iir_order: usize,
    pub lpf_debug_output: bool,
    pub lpf_debug_label: Option<String>,
    pub lpf_debug_overwrite: bool,
    pub snr_background_window: Option<Window>,
    pub snr_signal_window: Option<Window>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LockinLpfKind {
    FirZeroPhase,
    BoxcarLegacy,
    FirBoxcarEnbw,
    SyncIirZeroPhase,
}

#[derive(Debug, Clone, Serialize)]
pub struct Phase {
    pub m_omega_t0_offset: Vec<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KerrType {
    Standard,
    Harmonics,
}

#[derive(Debug, Clone, Serialize)]
pub struct Kerr {
    pub use_sensor_ch: u8,
    pub kerr_type: KerrType,
    pub factor: f64,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub enum ValidationTarget {
    Single,
    Trigger,
    Autoshot,
    Fetch,
    Automeasure,
    Reference,
    Sensor,
    Li,
    Phase,
    Kerr,
    Analyze,
    Process,
    Auto,
}

#[derive(Debug, Clone)]
pub struct ConfigWarning {
    pub message: String,
}

impl ConfigWarning {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub enum DiagnosticKind {
    Io,
    Parse,
    Deserialize,
    Migration,
    Validation,
}

impl fmt::Display for DiagnosticKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiagnosticKind::Io => write!(f, "I/O"),
            DiagnosticKind::Parse => write!(f, "Parse"),
            DiagnosticKind::Deserialize => write!(f, "Schema"),
            DiagnosticKind::Migration => write!(f, "Migration"),
            DiagnosticKind::Validation => write!(f, "Validation"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigDiagnostic {
    pub kind: DiagnosticKind,
    pub path: Option<String>,
    pub message: String,
    pub suggestion: Option<String>,
}

impl ConfigDiagnostic {
    fn new(
        kind: DiagnosticKind,
        path: Option<String>,
        message: impl Into<String>,
        suggestion: Option<String>,
    ) -> Self {
        Self {
            kind,
            path,
            message: message.into(),
            suggestion,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ConfigDiagnostics {
    pub version: Option<u32>,
    pub warnings: Vec<ConfigWarning>,
    pub diagnostics: Vec<ConfigDiagnostic>,
    pub normalized: Option<Config>,
}

#[derive(Debug, Clone)]
pub enum ConfigLoad {
    Ready {
        config: Config,
        warnings: Vec<ConfigWarning>,
    },
    Diagnostics(ConfigDiagnostics),
}

impl ConfigLoad {
    pub fn into_ready(self) -> Result<(Config, Vec<ConfigWarning>)> {
        match self {
            ConfigLoad::Ready { config, warnings } => Ok((config, warnings)),
            ConfigLoad::Diagnostics(diag) => Err(anyhow!(
                "configuration is not runnable: {} diagnostic(s)",
                diag.diagnostics.len()
            )),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ConfigV1 {
    #[allow(dead_code)]
    version: u32,
    instruments: Option<InstrumentsV1>,
    #[serde(default)]
    image: ImageV3,
    timebase: TimebaseV1,
    roles: RolesV1,
    channels: Vec<ChannelV1>,
    pulse: PulseV1,
    reference: ReferenceV1,
    lockin: LockinV1,
    phase: PhaseV1,
    kerr: KerrV1,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigV2 {
    #[allow(dead_code)]
    version: u32,
    instruments: Option<InstrumentsV2>,
    #[serde(default)]
    fetch: FetchV2,
    #[serde(default)]
    image: ImageV3,
    #[serde(default)]
    plot: PlotV2,
    timebase: TimebaseV2,
    roles: RolesV2,
    channels: Vec<ChannelV2>,
    pulse: PulseV2,
    reference: ReferenceV2,
    lockin: LockinV2,
    phase: PhaseV2,
    kerr: KerrV2,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ConfigV3 {
    version: u32,
    instruments: Option<InstrumentsV2>,
    #[serde(default)]
    fetch: FetchV2,
    #[serde(default)]
    image: ImageV3,
    #[serde(default)]
    plot: PlotV2,
    roles: RolesV2,
    channels: Vec<ChannelV2>,
    pulse: PulseV2,
    reference: ReferenceV2,
    lockin: LockinV2,
    phase: PhaseV2,
    kerr: KerrV2,
}

#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct FetchV2 {
    #[serde(default)]
    output: FetchOutput,
    #[serde(default)]
    analysis_input: FetchAnalysisInput,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ImageV3 {
    enabled: bool,
    scope_path: String,
}

impl Default for ImageV3 {
    fn default() -> Self {
        let default = Image::default();
        Self {
            enabled: default.enabled,
            scope_path: default.scope_path,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct PlotV2 {
    enabled: bool,
    save: bool,
    interactive: bool,
    output_dir: String,
    max_points: usize,
    decimation: PlotDecimation,
    fail_on_error: bool,
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
struct InstrumentsV1 {
    #[serde(rename = "function_generator")]
    function_generator: Option<FunctionGeneratorV1>,
    oscilloscope: OscilloscopeV1,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstrumentsV2 {
    #[serde(rename = "function_generator")]
    function_generator: Option<FunctionGeneratorV2>,
    oscilloscope: OscilloscopeV2,
}

#[derive(Debug, Deserialize)]
struct FunctionGeneratorV1 {
    connection: Connection,
    #[serde(default)]
    model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FunctionGeneratorV2 {
    connection: Connection,
    #[serde(default)]
    model: String,
}

#[derive(Debug, Deserialize)]
struct OscilloscopeV1 {
    connection: Connection,
    #[serde(default)]
    model: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OscilloscopeV2 {
    connection: Connection,
    #[serde(default)]
    model: String,
}

#[derive(Debug, Deserialize)]
struct TimebaseV1 {
    t0: f64,
    dt: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TimebaseV2 {
    t0: f64,
    dt: f64,
}

#[derive(Debug, Deserialize)]
struct RolesV1 {
    #[serde(default, deserialize_with = "one_or_many")]
    sensor_ch: Vec<u8>,
    #[serde(default, deserialize_with = "one_or_many")]
    reference_ch: Vec<u8>,
    #[serde(default, deserialize_with = "one_or_many")]
    signal_ch: Vec<u8>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RolesV2 {
    #[serde(default, deserialize_with = "one_or_many")]
    sensor_ch: Vec<u8>,
    reference_ch: u8,
    #[serde(default, deserialize_with = "one_or_many")]
    signal_ch: Vec<u8>,
}

#[derive(Debug, Deserialize)]
struct ChannelV1 {
    index: u8,
    #[serde(default)]
    factor: Option<f64>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    unit_out: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelV2 {
    index: u8,
    #[serde(default)]
    factor: Option<f64>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    unit_out: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PulseV1 {
    bg_window_before: Window,
    bg_window_after: Window,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PulseV2 {
    bg_window_before: Window,
    bg_window_after: Window,
}

#[derive(Debug, Deserialize)]
struct ReferenceV1 {
    fft_window: Window,
    stride_samples: usize,
    window_samples: usize,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReferenceV2 {
    fft_window: Window,
    stride_samples: usize,
    window_samples: usize,
}

#[derive(Debug, Deserialize)]
struct LockinV1 {
    workers: usize,
    stride_samples: usize,
    #[serde(default)]
    filter_length_samples: Option<usize>,
    #[serde(default)]
    demodulation: Option<LockinDemodulationV1>,
    #[serde(default)]
    lpf_kind: Option<LockinLpfKind>,
    #[serde(default)]
    lpf_half_window_cycles: Option<f64>,
    #[serde(default)]
    lpf_cutoff_hz: Option<f64>,
    #[serde(default)]
    lpf_cutoff_ref_ratio: Option<f64>,
    #[serde(default = "default_lockin_stopband_atten_db")]
    lpf_stopband_atten_db: f64,
    #[serde(default = "default_lockin_sync_average_cycles")]
    lpf_sync_average_cycles: f64,
    #[serde(default = "default_lockin_iir_order")]
    lpf_iir_order: usize,
    #[serde(default)]
    lpf_debug_output: bool,
    #[serde(default)]
    lpf_debug_label: Option<String>,
    #[serde(default)]
    lpf_debug_overwrite: bool,
    #[serde(default)]
    snr_background_window: Option<Window>,
    #[serde(default)]
    snr_signal_window: Option<Window>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LockinV2 {
    workers: usize,
    stride_samples: usize,
    #[serde(default)]
    lpf_kind: Option<LockinLpfKind>,
    lpf_half_window_cycles: f64,
    #[serde(default)]
    lpf_cutoff_hz: Option<f64>,
    #[serde(default)]
    lpf_cutoff_ref_ratio: Option<f64>,
    #[serde(default = "default_lockin_stopband_atten_db")]
    lpf_stopband_atten_db: f64,
    #[serde(default = "default_lockin_sync_average_cycles")]
    lpf_sync_average_cycles: f64,
    #[serde(default = "default_lockin_iir_order")]
    lpf_iir_order: usize,
    #[serde(default)]
    lpf_debug_output: bool,
    #[serde(default)]
    lpf_debug_label: Option<String>,
    #[serde(default)]
    lpf_debug_overwrite: bool,
    #[serde(default)]
    snr_background_window: Option<Window>,
    #[serde(default)]
    snr_signal_window: Option<Window>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LockinDemodulationV1 {
    Complex,
}

#[derive(Debug, Deserialize)]
struct PhaseV1 {
    #[serde(default, deserialize_with = "one_or_many")]
    use_signal_ch: Vec<u8>,
    #[serde(default, deserialize_with = "de_vec_f64_or_expr")]
    m_omega_t0_offset: Vec<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PhaseV2 {
    #[serde(default, deserialize_with = "de_vec_f64_or_expr")]
    m_omega_t0_offset: Vec<f64>,
}

#[derive(Debug, Deserialize)]
struct KerrV1 {
    use_sensor_ch: u8,
    kerr_type: KerrType,
    factor: f64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct KerrV2 {
    use_sensor_ch: u8,
    kerr_type: KerrType,
    factor: f64,
}

fn default_lockin_stopband_atten_db() -> f64 {
    60.0
}

fn default_lockin_sync_average_cycles() -> f64 {
    1.0
}

fn default_lockin_iir_order() -> usize {
    2
}

pub fn load_from_path(path: impl AsRef<Path>) -> ConfigLoad {
    let path = path.as_ref();
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            return ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: None,
                warnings: Vec::new(),
                diagnostics: vec![ConfigDiagnostic::new(
                    DiagnosticKind::Io,
                    None,
                    format!("failed to read {}: {}", path.display(), err),
                    None,
                )],
                normalized: None,
            });
        }
    };

    let mut load = load_from_str(&text);
    if let ConfigLoad::Ready { config, .. } = &mut load {
        config.source_path = path.to_path_buf();
    }
    load
}

pub fn load_from_str(s: &str) -> ConfigLoad {
    let parsed_value = match toml::from_str::<toml::Value>(s) {
        Ok(value) => value,
        Err(err) => {
            return ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: None,
                warnings: Vec::new(),
                diagnostics: vec![ConfigDiagnostic::new(
                    DiagnosticKind::Parse,
                    None,
                    format!("toml parse error: {err}"),
                    None,
                )],
                normalized: None,
            });
        }
    };

    let version = match parsed_value.get("version").and_then(|v| v.as_integer()) {
        Some(v) if v >= 0 => v as u32,
        Some(v) => {
            return ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: None,
                warnings: Vec::new(),
                diagnostics: vec![ConfigDiagnostic::new(
                    DiagnosticKind::Parse,
                    Some("version".to_string()),
                    format!("version must be a non-negative integer (got {v})"),
                    None,
                )],
                normalized: None,
            });
        }
        None => {
            return ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: None,
                warnings: Vec::new(),
                diagnostics: vec![ConfigDiagnostic::new(
                    DiagnosticKind::Parse,
                    Some("version".to_string()),
                    "missing required top-level `version`".to_string(),
                    None,
                )],
                normalized: None,
            });
        }
    };

    match version {
        1 => match deserialize_versioned::<ConfigV1>(s) {
            Ok(raw) => normalize_v1(raw),
            Err(diag) => ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: Some(1),
                warnings: Vec::new(),
                diagnostics: vec![diag],
                normalized: None,
            }),
        },
        2 => match deserialize_versioned::<ConfigV2>(s) {
            Ok(raw) => normalize_v2(raw),
            Err(diag) => ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: Some(2),
                warnings: Vec::new(),
                diagnostics: vec![diag],
                normalized: None,
            }),
        },
        3 => match deserialize_versioned::<ConfigV3>(s) {
            Ok(raw) => normalize_v3(raw),
            Err(diag) => ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: Some(3),
                warnings: Vec::new(),
                diagnostics: vec![diag],
                normalized: None,
            }),
        },
        other => {
            ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: Some(other),
                warnings: Vec::new(),
                diagnostics: vec![ConfigDiagnostic::new(
                DiagnosticKind::Parse,
                Some("version".to_string()),
                format!("unsupported config version: {other}"),
                Some("use version = 1 or 2 for legacy configs, or version = 3 for the normalized schema"
                    .to_string()),
            )],
                normalized: None,
            })
        }
    }
}

fn deserialize_versioned<T>(s: &str) -> std::result::Result<T, ConfigDiagnostic>
where
    T: for<'de> Deserialize<'de>,
{
    let de = toml::de::Deserializer::parse(s).map_err(|e| {
        ConfigDiagnostic::new(
            DiagnosticKind::Parse,
            None,
            format!("toml parse error: {e}"),
            None,
        )
    })?;

    serde_path_to_error::deserialize(de).map_err(|e| {
        ConfigDiagnostic::new(
            DiagnosticKind::Deserialize,
            Some(e.path().to_string()),
            e.to_string(),
            None,
        )
    })
}

fn normalize_v1(raw: ConfigV1) -> ConfigLoad {
    let mut warnings = Vec::new();
    let mut diagnostics = Vec::new();

    let reference_ch = match normalize_reference_ch_v1(&raw.roles.reference_ch) {
        Ok(ch) => ch,
        Err(diag) => {
            diagnostics.push(diag);
            return ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: Some(1),
                warnings,
                diagnostics,
                normalized: None,
            });
        }
    };

    if let Some(_demod) = raw.lockin.demodulation {
        warnings.push(ConfigWarning::new(
            "lockin.demodulation is deprecated in version 1 and ignored; complex demodulation is always used",
        ));
    }

    if raw.lockin.filter_length_samples.is_some() {
        warnings.push(ConfigWarning::new(
            "lockin.filter_length_samples is deprecated; it is interpreted as lockin.lpf_half_window_cycles during normalization",
        ));
    }

    if !raw.phase.use_signal_ch.is_empty() && raw.phase.use_signal_ch != raw.roles.signal_ch {
        diagnostics.push(ConfigDiagnostic::new(
            DiagnosticKind::Migration,
            Some("phase.use_signal_ch".to_string()),
            "phase.use_signal_ch is deprecated and cannot be migrated automatically when it differs from roles.signal_ch",
            Some(
                "remove phase.use_signal_ch and set roles.signal_ch to the exact signal channels you want to analyse".to_string(),
            ),
        ));
        return ConfigLoad::Diagnostics(ConfigDiagnostics {
            version: Some(1),
            warnings,
            diagnostics,
            normalized: None,
        });
    }

    let lpf_half_window_cycles = match raw
        .lockin
        .lpf_half_window_cycles
        .or_else(|| raw.lockin.filter_length_samples.map(|v| v as f64))
    {
        Some(v) => v,
        None => {
            diagnostics.push(ConfigDiagnostic::new(
                DiagnosticKind::Migration,
                Some("lockin".to_string()),
                "version 1 config must provide lockin.lpf_half_window_cycles or lockin.filter_length_samples",
                None,
            ));
            return ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: Some(1),
                warnings,
                diagnostics,
                normalized: None,
            });
        }
    };

    let lpf_kind = raw.lockin.lpf_kind.unwrap_or_else(|| {
        if raw.lockin.filter_length_samples.is_some() && raw.lockin.lpf_half_window_cycles.is_none()
        {
            LockinLpfKind::BoxcarLegacy
        } else {
            LockinLpfKind::FirZeroPhase
        }
    });

    let mut cfg = Config {
        version: 3,
        instruments: raw.instruments.map(Into::into),
        fetch: Fetch::default(),
        image: raw.image.into(),
        plot: Plot::default(),
        source_path: PathBuf::from("config.toml"),
        legacy_timebase: Some(raw.timebase.into()),
        roles: Roles {
            sensor_ch: raw.roles.sensor_ch,
            reference_ch,
            signal_ch: raw.roles.signal_ch,
        },
        channels: raw.channels.into_iter().map(Into::into).collect(),
        pulse: raw.pulse.into(),
        reference: raw.reference.into(),
        lockin: Lockin {
            workers: raw.lockin.workers,
            stride_samples: raw.lockin.stride_samples,
            lpf_kind,
            lpf_half_window_cycles,
            lpf_cutoff_hz: raw.lockin.lpf_cutoff_hz,
            lpf_cutoff_ref_ratio: raw.lockin.lpf_cutoff_ref_ratio,
            lpf_stopband_atten_db: raw.lockin.lpf_stopband_atten_db,
            lpf_sync_average_cycles: raw.lockin.lpf_sync_average_cycles,
            lpf_iir_order: raw.lockin.lpf_iir_order,
            lpf_debug_output: raw.lockin.lpf_debug_output,
            lpf_debug_label: raw.lockin.lpf_debug_label,
            lpf_debug_overwrite: raw.lockin.lpf_debug_overwrite,
            snr_background_window: raw.lockin.snr_background_window,
            snr_signal_window: raw.lockin.snr_signal_window,
        },
        phase: Phase {
            m_omega_t0_offset: raw.phase.m_omega_t0_offset,
        },
        kerr: raw.kerr.into(),
    };

    let validation = validate_common(&mut cfg);
    warnings.extend(validation.warnings);
    diagnostics.extend(validation.errors);

    if diagnostics.is_empty() {
        ConfigLoad::Ready {
            config: cfg,
            warnings,
        }
    } else {
        ConfigLoad::Diagnostics(ConfigDiagnostics {
            version: Some(1),
            warnings,
            diagnostics,
            normalized: None,
        })
    }
}

fn normalize_v2(raw: ConfigV2) -> ConfigLoad {
    let legacy_timebase = raw.timebase.into();
    let mut warnings = vec![ConfigWarning::new(
        "timebase is deprecated; waveform time axis is read from raw metadata or CSV time column",
    )];
    let mut cfg = Config {
        version: 3,
        instruments: raw.instruments.map(Into::into),
        fetch: raw.fetch.into(),
        image: raw.image.into(),
        plot: raw.plot.into(),
        source_path: PathBuf::from("config.toml"),
        legacy_timebase: Some(legacy_timebase),
        roles: Roles {
            sensor_ch: raw.roles.sensor_ch,
            reference_ch: raw.roles.reference_ch,
            signal_ch: raw.roles.signal_ch,
        },
        channels: raw.channels.into_iter().map(Into::into).collect(),
        pulse: raw.pulse.into(),
        reference: raw.reference.into(),
        lockin: Lockin {
            workers: raw.lockin.workers,
            stride_samples: raw.lockin.stride_samples,
            lpf_kind: raw.lockin.lpf_kind.unwrap_or(LockinLpfKind::FirZeroPhase),
            lpf_half_window_cycles: raw.lockin.lpf_half_window_cycles,
            lpf_cutoff_hz: raw.lockin.lpf_cutoff_hz,
            lpf_cutoff_ref_ratio: raw.lockin.lpf_cutoff_ref_ratio,
            lpf_stopband_atten_db: raw.lockin.lpf_stopband_atten_db,
            lpf_sync_average_cycles: raw.lockin.lpf_sync_average_cycles,
            lpf_iir_order: raw.lockin.lpf_iir_order,
            lpf_debug_output: raw.lockin.lpf_debug_output,
            lpf_debug_label: raw.lockin.lpf_debug_label,
            lpf_debug_overwrite: raw.lockin.lpf_debug_overwrite,
            snr_background_window: raw.lockin.snr_background_window,
            snr_signal_window: raw.lockin.snr_signal_window,
        },
        phase: Phase {
            m_omega_t0_offset: raw.phase.m_omega_t0_offset,
        },
        kerr: raw.kerr.into(),
    };

    let validation = validate_common(&mut cfg);
    if validation.errors.is_empty() {
        warnings.extend(validation.warnings);
        ConfigLoad::Ready {
            config: cfg,
            warnings,
        }
    } else {
        ConfigLoad::Diagnostics(ConfigDiagnostics {
            version: Some(2),
            warnings: {
                warnings.extend(validation.warnings);
                warnings
            },
            diagnostics: validation.errors,
            normalized: None,
        })
    }
}

fn normalize_v3(raw: ConfigV3) -> ConfigLoad {
    let mut cfg = Config {
        version: raw.version,
        instruments: raw.instruments.map(Into::into),
        fetch: raw.fetch.into(),
        image: raw.image.into(),
        plot: raw.plot.into(),
        source_path: PathBuf::from("config.toml"),
        legacy_timebase: None,
        roles: Roles {
            sensor_ch: raw.roles.sensor_ch,
            reference_ch: raw.roles.reference_ch,
            signal_ch: raw.roles.signal_ch,
        },
        channels: raw.channels.into_iter().map(Into::into).collect(),
        pulse: raw.pulse.into(),
        reference: raw.reference.into(),
        lockin: Lockin {
            workers: raw.lockin.workers,
            stride_samples: raw.lockin.stride_samples,
            lpf_kind: raw.lockin.lpf_kind.unwrap_or(LockinLpfKind::FirZeroPhase),
            lpf_half_window_cycles: raw.lockin.lpf_half_window_cycles,
            lpf_cutoff_hz: raw.lockin.lpf_cutoff_hz,
            lpf_cutoff_ref_ratio: raw.lockin.lpf_cutoff_ref_ratio,
            lpf_stopband_atten_db: raw.lockin.lpf_stopband_atten_db,
            lpf_sync_average_cycles: raw.lockin.lpf_sync_average_cycles,
            lpf_iir_order: raw.lockin.lpf_iir_order,
            lpf_debug_output: raw.lockin.lpf_debug_output,
            lpf_debug_label: raw.lockin.lpf_debug_label,
            lpf_debug_overwrite: raw.lockin.lpf_debug_overwrite,
            snr_background_window: raw.lockin.snr_background_window,
            snr_signal_window: raw.lockin.snr_signal_window,
        },
        phase: Phase {
            m_omega_t0_offset: raw.phase.m_omega_t0_offset,
        },
        kerr: raw.kerr.into(),
    };

    let validation = validate_common(&mut cfg);
    if validation.errors.is_empty() {
        ConfigLoad::Ready {
            config: cfg,
            warnings: validation.warnings,
        }
    } else {
        ConfigLoad::Diagnostics(ConfigDiagnostics {
            version: Some(3),
            warnings: validation.warnings,
            diagnostics: validation.errors,
            normalized: None,
        })
    }
}

struct ValidationSummary {
    warnings: Vec<ConfigWarning>,
    errors: Vec<ConfigDiagnostic>,
}

fn uses_explicit_cutoff(kind: LockinLpfKind) -> bool {
    matches!(
        kind,
        LockinLpfKind::FirZeroPhase | LockinLpfKind::SyncIirZeroPhase
    )
}

fn validate_common(cfg: &mut Config) -> ValidationSummary {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    if cfg.version != 3 {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("version".to_string()),
            format!(
                "normalized config must have version 3 (got {})",
                cfg.version
            ),
            None,
        ));
    }
    if let Err(message) = validate_image_scope_path(&cfg.image.scope_path) {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("image.scope_path".to_string()),
            message,
            Some("use an ASCII path such as C:/screenshot.png".to_string()),
        ));
    }
    if cfg.plot.max_points == 0 {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("plot.max_points".to_string()),
            "plot.max_points must be positive",
            None,
        ));
    }
    if cfg.plot.output_dir.trim().is_empty() {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("plot.output_dir".to_string()),
            "plot.output_dir must not be empty",
            None,
        ));
    }
    if cfg.lockin.workers == 0 {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.workers".to_string()),
            "lockin.workers must be positive",
            None,
        ));
    }
    if cfg.lockin.stride_samples == 0 {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.stride_samples".to_string()),
            "lockin.stride_samples must be positive",
            None,
        ));
    }
    if cfg.lockin.lpf_half_window_cycles <= 0.0 {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.lpf_half_window_cycles".to_string()),
            format!(
                "lockin.lpf_half_window_cycles must be positive (got {})",
                cfg.lockin.lpf_half_window_cycles
            ),
            None,
        ));
    }
    if cfg.lockin.lpf_stopband_atten_db <= 0.0 {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.lpf_stopband_atten_db".to_string()),
            format!(
                "lockin.lpf_stopband_atten_db must be positive (got {})",
                cfg.lockin.lpf_stopband_atten_db
            ),
            None,
        ));
    }
    if cfg.lockin.lpf_kind == LockinLpfKind::SyncIirZeroPhase {
        let max_sync_cycles = 100.0;
        if !cfg.lockin.lpf_sync_average_cycles.is_finite()
            || cfg.lockin.lpf_sync_average_cycles <= 0.0
            || cfg.lockin.lpf_sync_average_cycles > max_sync_cycles
        {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("lockin.lpf_sync_average_cycles".to_string()),
                format!(
                    "lockin.lpf_sync_average_cycles must be finite and in (0, {max_sync_cycles}] (got {})",
                    cfg.lockin.lpf_sync_average_cycles
                ),
                None,
            ));
        }
    }
    if cfg.lockin.lpf_kind == LockinLpfKind::SyncIirZeroPhase
        && (cfg.lockin.lpf_iir_order == 0
            || !cfg.lockin.lpf_iir_order.is_multiple_of(2)
            || cfg.lockin.lpf_iir_order > 8)
    {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.lpf_iir_order".to_string()),
            format!(
                "lockin.lpf_iir_order must be one of 2, 4, 6, or 8 (got {})",
                cfg.lockin.lpf_iir_order
            ),
            None,
        ));
    }
    if uses_explicit_cutoff(cfg.lockin.lpf_kind) {
        if let Some(cutoff_hz) = cfg.lockin.lpf_cutoff_hz
            && cutoff_hz <= 0.0
        {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("lockin.lpf_cutoff_hz".to_string()),
                format!("lockin.lpf_cutoff_hz must be positive (got {cutoff_hz})"),
                None,
            ));
        }
        if let Some(cutoff_ratio) = cfg.lockin.lpf_cutoff_ref_ratio
            && cutoff_ratio <= 0.0
        {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("lockin.lpf_cutoff_ref_ratio".to_string()),
                format!("lockin.lpf_cutoff_ref_ratio must be positive (got {cutoff_ratio})"),
                None,
            ));
        }
        if cfg.lockin.lpf_cutoff_hz.is_some() && cfg.lockin.lpf_cutoff_ref_ratio.is_some() {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("lockin".to_string()),
                "lockin.lpf_cutoff_hz and lockin.lpf_cutoff_ref_ratio are mutually exclusive for cutoff-based LPF modes",
                None,
            ));
        }
    }
    if let Some(label) = &cfg.lockin.lpf_debug_label
        && !is_safe_debug_label(label)
    {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.lpf_debug_label".to_string()),
            "lockin.lpf_debug_label must be 1-64 ASCII characters using only A-Z, a-z, 0-9, '.', '_', or '-', and must not be '.' or '..'",
            None,
        ));
    }
    if cfg.phase.m_omega_t0_offset.len() != 6 {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("phase.m_omega_t0_offset".to_string()),
            format!(
                "phase.m_omega_t0_offset must have length 6 (got {})",
                cfg.phase.m_omega_t0_offset.len()
            ),
            None,
        ));
    }
    for (idx, value) in cfg.phase.m_omega_t0_offset.iter().enumerate() {
        if !value.is_finite() {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some(format!("phase.m_omega_t0_offset[{idx}]")),
                format!("phase.m_omega_t0_offset[{idx}] must be finite (got {value})"),
                None,
            ));
        }
    }

    let mut seen = BTreeSet::new();
    for ch in &cfg.channels {
        if !seen.insert(ch.index) {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("channels".to_string()),
                format!("duplicate channel index: {}", ch.index),
                None,
            ));
        }
    }

    for &idx in &cfg.roles.sensor_ch {
        if !seen.contains(&idx) {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("roles.sensor_ch".to_string()),
                format!("roles.sensor_ch contains undefined channel index: {}", idx),
                None,
            ));
        }
    }
    for &idx in &cfg.roles.signal_ch {
        if !seen.contains(&idx) {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("roles.signal_ch".to_string()),
                format!("roles.signal_ch contains undefined channel index: {}", idx),
                None,
            ));
        }
    }
    if !seen.contains(&cfg.roles.reference_ch) {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("roles.reference_ch".to_string()),
            format!(
                "roles.reference_ch ({}) is not defined in channels",
                cfg.roles.reference_ch
            ),
            None,
        ));
    }
    if !cfg.roles.sensor_ch.contains(&cfg.kerr.use_sensor_ch) {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("kerr.use_sensor_ch".to_string()),
            format!(
                "kerr.use_sensor_ch ({}) is not included in roles.sensor_ch",
                cfg.kerr.use_sensor_ch
            ),
            None,
        ));
    }

    let check_win = |label: &str, w: Window| -> Option<ConfigDiagnostic> {
        if matches!(w.start.partial_cmp(&w.end), Some(std::cmp::Ordering::Less)) {
            None
        } else {
            Some(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some(label.to_string()),
                format!(
                    "{label}: start must be < end (start={}, end={})",
                    w.start, w.end
                ),
                None,
            ))
        }
    };
    if let Some(diag) = check_win("pulse.bg_window_before", cfg.pulse.bg_window_before) {
        errors.push(diag);
    }
    if let Some(diag) = check_win("pulse.bg_window_after", cfg.pulse.bg_window_after) {
        errors.push(diag);
    }
    if let Some(diag) = check_win("reference.fft_window", cfg.reference.fft_window) {
        errors.push(diag);
    }
    if let Some(window) = cfg.lockin.snr_background_window
        && let Some(diag) = check_win("lockin.snr_background_window", window)
    {
        errors.push(diag);
    }
    if let Some(window) = cfg.lockin.snr_signal_window
        && let Some(diag) = check_win("lockin.snr_signal_window", window)
    {
        errors.push(diag);
    }

    if uses_explicit_cutoff(cfg.lockin.lpf_kind)
        && cfg.lockin.lpf_cutoff_hz.is_none()
        && cfg.lockin.lpf_cutoff_ref_ratio.is_none()
    {
        warnings.push(ConfigWarning::new(
            "lockin.lpf_kind uses an explicit cutoff but no cutoff is specified; runtime will use the compatibility fallback cutoff 0.5 / t_half",
        ));
    }

    let mut used = BTreeSet::new();
    used.extend(cfg.roles.sensor_ch.iter().copied());
    used.extend(cfg.roles.signal_ch.iter().copied());
    used.insert(cfg.roles.reference_ch);
    for ch in &cfg.channels {
        if !used.contains(&ch.index) {
            warnings.push(ConfigWarning::new(format!(
                "channel index {} is defined in [channels] but not used in roles",
                ch.index
            )));
        }
    }

    cfg.channels.sort_by_key(|ch| ch.index);

    ValidationSummary { warnings, errors }
}

pub fn validate_image_scope_path(path: &str) -> std::result::Result<(), String> {
    if !path.is_ascii() {
        return Err("image.scope_path must contain only ASCII characters".to_string());
    }
    if path.contains(['\r', '\n', '"', '\'', ';', '\\']) {
        return Err("image.scope_path contains an unsupported character".to_string());
    }
    let Some((drive, relative)) = path.split_once(":/") else {
        return Err("image.scope_path must start with C:/, D:/, or E:/".to_string());
    };
    if !matches!(drive, "C" | "D" | "E") || relative.is_empty() {
        return Err("image.scope_path must start with C:/, D:/, or E:/".to_string());
    }
    let components = relative.split('/').collect::<Vec<_>>();
    if components
        .iter()
        .any(|part| part.is_empty() || *part == "." || *part == "..")
    {
        return Err("image.scope_path must not contain empty, '.' or '..' components".to_string());
    }
    if components.iter().any(|part| {
        !part
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    }) {
        return Err(
            "image.scope_path components may contain only letters, digits, '.', '_' and '-'"
                .to_string(),
        );
    }
    let filename = components.last().expect("relative path is non-empty");
    if filename.len() > 16 {
        return Err(
            "image.scope_path filename, including its extension, must be at most 16 characters"
                .to_string(),
        );
    }
    let extension = filename.rsplit_once('.').map(|(_, extension)| extension);
    if !matches!(
        extension.map(str::to_ascii_lowercase).as_deref(),
        Some("png" | "bmp" | "jpg")
    ) {
        return Err("image.scope_path extension must be .png, .bmp, or .jpg".to_string());
    }
    Ok(())
}

fn is_safe_debug_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 64
        && label != "."
        && label != ".."
        && label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

pub fn validate_for_target(cfg: &Config, target: ValidationTarget) -> Result<()> {
    match target {
        ValidationTarget::Single
        | ValidationTarget::Fetch
        | ValidationTarget::Process
        | ValidationTarget::Auto => {
            validate_oscilloscope_required(cfg)?;
        }
        ValidationTarget::Trigger | ValidationTarget::Autoshot | ValidationTarget::Automeasure => {
            validate_oscilloscope_required(cfg)?;
            validate_function_generator_required(cfg)?;
        }
        ValidationTarget::Reference
        | ValidationTarget::Sensor
        | ValidationTarget::Li
        | ValidationTarget::Phase
        | ValidationTarget::Kerr
        | ValidationTarget::Analyze => {}
    }

    match target {
        ValidationTarget::Reference => {
            validate_oscilloscope_required(cfg)?;
            validate_reference_roles(cfg)?;
            validate_analysis_input_exists(cfg)?;
        }
        ValidationTarget::Sensor => {
            validate_oscilloscope_required(cfg)?;
            validate_reference_roles(cfg)?;
            validate_sensor_roles(cfg)?;
            validate_sensor_metadata(cfg)?;
            validate_analysis_input_exists(cfg)?;
        }
        ValidationTarget::Li => {
            validate_oscilloscope_required(cfg)?;
            validate_reference_roles(cfg)?;
            validate_sensor_roles(cfg)?;
            validate_signal_roles(cfg)?;
            validate_sensor_metadata(cfg)?;
            validate_analysis_input_exists(cfg)?;
        }
        ValidationTarget::Phase => {
            validate_signal_roles(cfg)?;
            validate_sensor_metadata(cfg)?;
            validate_lockin_results_exist(cfg)?;
        }
        ValidationTarget::Kerr => {
            validate_signal_roles(cfg)?;
            validate_sensor_roles(cfg)?;
            validate_sensor_metadata(cfg)?;
            validate_kerr_sensor(cfg)?;
            validate_rotated_results_exist(cfg)?;
        }
        ValidationTarget::Analyze => {
            validate_oscilloscope_required(cfg)?;
            validate_reference_roles(cfg)?;
            validate_sensor_roles(cfg)?;
            validate_signal_roles(cfg)?;
            validate_sensor_metadata(cfg)?;
            validate_analysis_input_exists(cfg)?;
        }
        ValidationTarget::Process => {
            validate_reference_roles(cfg)?;
            validate_sensor_roles(cfg)?;
            validate_signal_roles(cfg)?;
            validate_sensor_metadata(cfg)?;
        }
        ValidationTarget::Auto => {
            validate_function_generator_required(cfg)?;
            validate_reference_roles(cfg)?;
            validate_sensor_roles(cfg)?;
            validate_signal_roles(cfg)?;
            validate_sensor_metadata(cfg)?;
        }
        ValidationTarget::Automeasure
        | ValidationTarget::Fetch
        | ValidationTarget::Single
        | ValidationTarget::Trigger
        | ValidationTarget::Autoshot => {}
    }

    Ok(())
}

fn validate_reference_roles(cfg: &Config) -> Result<()> {
    if cfg.roles.reference_ch == 0 {
        bail!("roles.reference_ch must be set");
    }
    Ok(())
}

fn validate_sensor_roles(cfg: &Config) -> Result<()> {
    if cfg.roles.sensor_ch.is_empty() {
        bail!("roles.sensor_ch must contain at least one channel");
    }
    Ok(())
}

fn validate_signal_roles(cfg: &Config) -> Result<()> {
    if cfg.roles.signal_ch.is_empty() {
        bail!("roles.signal_ch must contain at least one channel");
    }
    Ok(())
}

fn validate_kerr_sensor(cfg: &Config) -> Result<()> {
    if !cfg.roles.sensor_ch.contains(&cfg.kerr.use_sensor_ch) {
        bail!(
            "kerr.use_sensor_ch ({}) must be included in roles.sensor_ch",
            cfg.kerr.use_sensor_ch
        );
    }
    Ok(())
}

fn validate_oscilloscope_required(cfg: &Config) -> Result<()> {
    cfg.instruments
        .as_ref()
        .ok_or_else(|| anyhow!("instruments configuration is required for this command"))?;
    Ok(())
}

fn validate_function_generator_required(cfg: &Config) -> Result<()> {
    let instruments = cfg
        .instruments
        .as_ref()
        .ok_or_else(|| anyhow!("instruments configuration is required for this command"))?;

    if instruments.function_generator.is_none() {
        bail!("instruments.function_generator is required for this command");
    }
    Ok(())
}

fn validate_sensor_metadata(cfg: &Config) -> Result<()> {
    for ch in &cfg.roles.sensor_ch {
        let meta = cfg
            .channels
            .iter()
            .find(|c| c.index == *ch)
            .ok_or_else(|| anyhow!("channel {} is not defined in [channels]", ch))?;

        if meta.factor.is_none() {
            bail!("channel {} has no 'factor'", ch);
        }
        if meta.label.is_none() {
            bail!("channel {} has no 'label'", ch);
        }
        if meta.unit_out.is_none() {
            bail!("channel {} has no 'unit_out'", ch);
        }
    }
    Ok(())
}

fn validate_analysis_input_exists(cfg: &Config) -> Result<()> {
    match cfg.fetch.analysis_input {
        FetchAnalysisInput::Csv => validate_raw_csv_exists(),
        FetchAnalysisInput::Raw => validate_raw_metadata_exists(),
        FetchAnalysisInput::Auto => {
            let raw_dir = Path::new(RAW_WAVEFORM_DIR);
            let metadata = raw_dir.join(RAW_METADATA_FNAME);
            if metadata.exists() {
                Ok(())
            } else if raw_dir.exists() {
                bail!("raw metadata not found: {}", metadata.display())
            } else {
                validate_raw_csv_exists()
            }
        }
    }
}

fn validate_raw_csv_exists() -> Result<()> {
    validate_file_exists(Path::new(FETCHED_FNAME), FETCHED_FNAME)
}

fn validate_raw_metadata_exists() -> Result<()> {
    let path = Path::new(RAW_WAVEFORM_DIR).join(RAW_METADATA_FNAME);
    validate_file_exists(&path, &path.display().to_string())
}

fn validate_lockin_results_exist(cfg: &Config) -> Result<()> {
    for ch in cfg.phase_signal_ch() {
        let fname = format!("{}_ch{}.csv", LI_RESULTS_NAME, ch);
        validate_file_exists(Path::new(&fname), &fname)?;
    }
    Ok(())
}

fn validate_rotated_results_exist(cfg: &Config) -> Result<()> {
    for ch in cfg.phase_signal_ch() {
        let fname = format!("{}_ch{}.csv", LI_ROTATED_NAME, ch);
        validate_file_exists(Path::new(&fname), &fname)?;
    }
    Ok(())
}

fn validate_file_exists(path: &Path, label: &str) -> Result<()> {
    if path.exists() {
        Ok(())
    } else {
        bail!("{label} does not exist")
    }
}

fn normalize_reference_ch_v1(reference_ch: &[u8]) -> std::result::Result<u8, ConfigDiagnostic> {
    match reference_ch {
        [] => Err(ConfigDiagnostic::new(
            DiagnosticKind::Migration,
            Some("roles.reference_ch".to_string()),
            "version 1 config must provide exactly one reference channel",
            Some("set roles.reference_ch to a single channel index".to_string()),
        )),
        [ch] => Ok(*ch),
        _ => Err(ConfigDiagnostic::new(
            DiagnosticKind::Migration,
            Some("roles.reference_ch".to_string()),
            format!(
                "version 1 config cannot be migrated automatically because roles.reference_ch has {} entries",
                reference_ch.len()
            ),
            Some("set roles.reference_ch to a single channel index".to_string()),
        )),
    }
}

impl From<InstrumentsV1> for Instruments {
    fn from(value: InstrumentsV1) -> Self {
        Self {
            function_generator: value.function_generator.map(Into::into),
            oscilloscope: value.oscilloscope.into(),
        }
    }
}

impl From<InstrumentsV2> for Instruments {
    fn from(value: InstrumentsV2) -> Self {
        Self {
            function_generator: value.function_generator.map(Into::into),
            oscilloscope: value.oscilloscope.into(),
        }
    }
}

impl From<FetchV2> for Fetch {
    fn from(value: FetchV2) -> Self {
        Self {
            output: value.output,
            analysis_input: value.analysis_input,
        }
    }
}

impl From<ImageV3> for Image {
    fn from(value: ImageV3) -> Self {
        Self {
            enabled: value.enabled,
            scope_path: value.scope_path,
        }
    }
}

impl From<PlotV2> for Plot {
    fn from(value: PlotV2) -> Self {
        Self {
            enabled: value.enabled,
            save: value.save,
            interactive: value.interactive,
            output_dir: value.output_dir,
            max_points: value.max_points,
            decimation: value.decimation,
            fail_on_error: value.fail_on_error,
        }
    }
}

impl From<FunctionGeneratorV1> for FunctionGenerator {
    fn from(value: FunctionGeneratorV1) -> Self {
        Self {
            connection: value.connection,
            model: value.model,
        }
    }
}

impl From<FunctionGeneratorV2> for FunctionGenerator {
    fn from(value: FunctionGeneratorV2) -> Self {
        Self {
            connection: value.connection,
            model: value.model,
        }
    }
}

impl From<OscilloscopeV1> for Oscilloscope {
    fn from(value: OscilloscopeV1) -> Self {
        Self {
            connection: value.connection,
            model: value.model,
        }
    }
}

impl From<OscilloscopeV2> for Oscilloscope {
    fn from(value: OscilloscopeV2) -> Self {
        Self {
            connection: value.connection,
            model: value.model,
        }
    }
}

impl From<TimebaseV1> for Timebase {
    fn from(value: TimebaseV1) -> Self {
        Self {
            t0: value.t0,
            dt: value.dt,
        }
    }
}

impl From<TimebaseV2> for Timebase {
    fn from(value: TimebaseV2) -> Self {
        Self {
            t0: value.t0,
            dt: value.dt,
        }
    }
}

impl From<ChannelV1> for Channel {
    fn from(value: ChannelV1) -> Self {
        Self {
            index: value.index,
            factor: value.factor,
            label: value.label,
            unit_out: value.unit_out,
        }
    }
}

impl From<ChannelV2> for Channel {
    fn from(value: ChannelV2) -> Self {
        Self {
            index: value.index,
            factor: value.factor,
            label: value.label,
            unit_out: value.unit_out,
        }
    }
}

impl From<PulseV1> for Pulse {
    fn from(value: PulseV1) -> Self {
        Self {
            bg_window_before: value.bg_window_before,
            bg_window_after: value.bg_window_after,
        }
    }
}

impl From<PulseV2> for Pulse {
    fn from(value: PulseV2) -> Self {
        Self {
            bg_window_before: value.bg_window_before,
            bg_window_after: value.bg_window_after,
        }
    }
}

impl From<ReferenceV1> for Reference {
    fn from(value: ReferenceV1) -> Self {
        Self {
            fft_window: value.fft_window,
            stride_samples: value.stride_samples,
            window_samples: value.window_samples,
        }
    }
}

impl From<ReferenceV2> for Reference {
    fn from(value: ReferenceV2) -> Self {
        Self {
            fft_window: value.fft_window,
            stride_samples: value.stride_samples,
            window_samples: value.window_samples,
        }
    }
}

impl From<KerrV1> for Kerr {
    fn from(value: KerrV1) -> Self {
        Self {
            use_sensor_ch: value.use_sensor_ch,
            kerr_type: value.kerr_type,
            factor: value.factor,
        }
    }
}

impl From<KerrV2> for Kerr {
    fn from(value: KerrV2) -> Self {
        Self {
            use_sensor_ch: value.use_sensor_ch,
            kerr_type: value.kerr_type,
            factor: value.factor,
        }
    }
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
