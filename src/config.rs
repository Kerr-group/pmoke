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

mod load;
mod migration;
mod paths;
mod render;
mod schema;
mod validation;
pub use load::{load_from_path, load_from_str};
pub use migration::{MigrationPlan, plan_latest_executable_migration, plan_migration};
pub use paths::{ArtifactPaths, ArtifactResolver};
use render::render_config_v4;
pub use render::render_normalized_config;
use schema::*;
use validation::validate_common;
pub use validation::validate_for_target;
#[cfg(test)]
use validation::validate_sensor_metadata;

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
    pub screenshot: Screenshot,
    pub plot: Plot,
    #[serde(skip_serializing)]
    pub source_path: PathBuf,
    #[serde(skip_serializing)]
    pub source_text: Option<String>,
    #[serde(skip_serializing)]
    pub artifact_root: Option<PathBuf>,
    #[serde(skip_serializing)]
    pub plot_output_relative: Option<PathBuf>,
    #[serde(skip_serializing)]
    pub legacy_timebase: Option<Timebase>,
    #[serde(skip_serializing)]
    pub force: bool,
    #[serde(skip_serializing)]
    pub staging_active: bool,
    pub roles: Roles,
    pub channels: Vec<Channel>,
    pub pulse: Pulse,
    pub reference: Reference,
    pub lockin: Lockin,
    pub phase: Phase,
    pub kerr: Kerr,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Screenshot {
    pub enabled: bool,
}

impl Config {
    pub fn phase_signal_ch(&self) -> &[u8] {
        &self.roles.signal_ch
    }

    pub fn resolver(&self) -> ArtifactResolver {
        ArtifactResolver::new(self.paths().run_dir)
    }

    pub fn paths(&self) -> ArtifactPaths {
        let mut paths = if let Some(root) = &self.artifact_root {
            ArtifactPaths::new(root)
        } else if self.version >= 4 {
            let parent = self
                .source_path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."));
            ArtifactPaths::new(parent)
        } else {
            ArtifactPaths::new(".")
        };
        paths.is_staging = self.staging_active;
        paths
    }

    pub fn artifact_path(&self, path: impl AsRef<Path>) -> PathBuf {
        let path = path.as_ref();
        if path.is_absolute() {
            return path.to_path_buf();
        }
        if let Some(root) = &self.artifact_root {
            return root.join(path);
        }
        if self.version < 4 {
            return path.to_path_buf();
        }
        self.source_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."))
            .join(path)
    }

    pub fn set_artifact_root(&mut self, root: PathBuf) {
        let plot_path = PathBuf::from(&self.plot.output_dir);
        let relative = self
            .plot_output_relative
            .clone()
            .or_else(|| (!plot_path.is_absolute()).then_some(plot_path));
        if let Some(relative) = relative {
            self.plot.output_dir = root.join(relative).to_string_lossy().into_owned();
        }
        self.artifact_root = Some(root);
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
    None,
    Stride,
    MinMax,
}

impl PlotDecimation {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Stride => "stride",
            Self::MinMax => "min_max",
        }
    }
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
    pub scale_to_abs_max: Option<f64>,
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
    pub save_npy: bool,
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
    Screenshot,
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

struct ValidationSummary {
    warnings: Vec<ConfigWarning>,
    errors: Vec<ConfigDiagnostic>,
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

impl From<ScreenshotV3> for Screenshot {
    fn from(value: ScreenshotV3) -> Self {
        Self {
            enabled: value.enabled,
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

impl From<PlotV4> for Plot {
    fn from(value: PlotV4) -> Self {
        let (enabled, save, interactive) = match value.mode {
            PlotModeV4::Off => (false, false, false),
            PlotModeV4::Save => (true, true, false),
            PlotModeV4::Interactive => (true, false, true),
            PlotModeV4::Both => (true, true, true),
        };
        Self {
            enabled,
            save,
            interactive,
            output_dir: value.output_dir,
            max_points: value.max_points,
            decimation: value.decimation,
            fail_on_error: value.on_error == PlotErrorModeV4::Fail,
        }
    }
}

impl From<LockinV4> for Lockin {
    fn from(value: LockinV4) -> Self {
        let mut lockin = Self {
            workers: value.workers,
            stride_samples: value.stride_samples,
            lpf_kind: LockinLpfKind::BoxcarLegacy,
            lpf_half_window_cycles: 0.0,
            lpf_cutoff_hz: None,
            lpf_cutoff_ref_ratio: None,
            lpf_stopband_atten_db: default_lockin_stopband_atten_db(),
            lpf_sync_average_cycles: default_lockin_sync_average_cycles(),
            lpf_iir_order: default_lockin_iir_order(),
            lpf_debug_output: value.debug_output,
            lpf_debug_label: value.debug_label,
            lpf_debug_overwrite: value.debug_overwrite,
            snr_background_window: value.snr_background_window,
            snr_signal_window: value.snr_signal_window,
            save_npy: value.save_npy,
        };
        match value.filter {
            LockinFilterV4::BoxcarLegacy { half_window_cycles } => {
                lockin.lpf_kind = LockinLpfKind::BoxcarLegacy;
                lockin.lpf_half_window_cycles = half_window_cycles;
            }
            LockinFilterV4::FirBoxcarEnbw { half_window_cycles } => {
                lockin.lpf_kind = LockinLpfKind::FirBoxcarEnbw;
                lockin.lpf_half_window_cycles = half_window_cycles;
            }
            LockinFilterV4::FirZeroPhase {
                half_window_cycles,
                cutoff_hz,
                cutoff_ref_ratio,
                stopband_atten_db,
            } => {
                lockin.lpf_kind = LockinLpfKind::FirZeroPhase;
                lockin.lpf_half_window_cycles = half_window_cycles;
                lockin.lpf_cutoff_hz = cutoff_hz;
                lockin.lpf_cutoff_ref_ratio = cutoff_ref_ratio;
                lockin.lpf_stopband_atten_db = stopband_atten_db;
            }
            LockinFilterV4::SyncIirZeroPhase {
                half_window_cycles,
                cutoff_hz,
                cutoff_ref_ratio,
                sync_average_cycles,
                iir_order,
            } => {
                lockin.lpf_kind = LockinLpfKind::SyncIirZeroPhase;
                lockin.lpf_half_window_cycles = half_window_cycles;
                lockin.lpf_cutoff_hz = cutoff_hz;
                lockin.lpf_cutoff_ref_ratio = cutoff_ref_ratio;
                lockin.lpf_sync_average_cycles = sync_average_cycles;
                lockin.lpf_iir_order = iir_order;
            }
        }
        lockin
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
            scale_to_abs_max: value.scale_to_abs_max,
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
            scale_to_abs_max: value.scale_to_abs_max,
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

impl From<ReferenceV4> for Reference {
    fn from(value: ReferenceV4) -> Self {
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
#[path = "config/tests/mod.rs"]
mod tests;
