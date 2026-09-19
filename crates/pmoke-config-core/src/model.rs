use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfigV6 {
    pub version: u32,
    pub scope: Scope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<Generator>,
    pub data: Data,
    #[serde(default)]
    pub sensors: Vec<Sensor>,
    pub pulse: Pulse,
    pub reference: Reference,
    pub lockin: Lockin,
    pub phase: Phase,
    pub moke: Moke,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signals: Vec<Signal>,
    #[serde(default)]
    pub plot: Plot,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scope {
    pub model: String,
    pub connection: String,
}

/// Inert stage-minimal default (FR-01/FR-06, Issue #264, Card C): mirrors the
/// native Card A default. A well-formed dummy so recorded-data stages can
/// load without `[scope]`; the loopback discard port parses but fails fast
/// at dial time and is never routable.
impl Default for Scope {
    fn default() -> Self {
        Self {
            model: "DHO5108".to_string(),
            connection: "tcp://127.0.0.1:9".to_string(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Generator {
    pub model: String,
    pub connection: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Data {
    pub output: DataOutput,
    pub input: DataInput,
    #[serde(default)]
    pub screenshot: bool,
}

/// Inert stage-minimal default (FR-01/FR-06, Issue #264, Card C): mirrors the
/// native Card A default (`csv`/`csv`/no screenshot). A wrong guess only
/// misdirects the recorded-data lookup (a loud file-not-found diagnostic),
/// never hardware.
impl Default for Data {
    fn default() -> Self {
        Self {
            output: DataOutput::Csv,
            input: DataInput::Csv,
            screenshot: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DataOutput {
    Csv,
    Raw,
    Both,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum DataInput {
    Csv,
    Raw,
    Auto,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Sensor {
    pub channel: u8,
    pub scale: SensorScale,
    pub label: String,
    pub unit: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged)]
pub(crate) enum SensorScale {
    Factor(FactorScale),
    MaxAbs(MaxAbsScale),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FactorScale {
    pub factor: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct MaxAbsScale {
    pub max_abs: f64,
    pub polarity: i8,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Window {
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Pulse {
    pub background_before: Window,
    pub background_after: Window,
}

/// Inert stage-minimal default (FR-01/FR-06, Issue #264, Card C): disjoint
/// finite windows matching the canonical fixture shape and the native Card A
/// default, so unrelated stages load without `[pulse]`.
impl Default for Pulse {
    fn default() -> Self {
        Self {
            background_before: Window {
                start: -0.005,
                end: -0.001,
            },
            background_after: Window {
                start: 0.01,
                end: 0.02,
            },
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Signal {
    pub channel: u8,
    pub label: String,
    pub unit: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reference {
    pub channel: u8,
    pub fft_window: Window,
    pub stride_samples: usize,
    pub window_samples: usize,
}

/// Inert stage-minimal default (FR-01/FR-06, Issue #264, Card C): mirrors the
/// native Card A default. Channel `0` is the established unspecified sentinel
/// (assigns no hardware channel; reference-gated targets reject it), so no
/// reference-gated stage can act on this default.
impl Default for Reference {
    fn default() -> Self {
        Self {
            channel: 0,
            fft_window: Window {
                start: 0.0,
                end: 0.005,
            },
            stride_samples: 100,
            window_samples: 1000,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Lockin {
    #[serde(alias = "signal_channels")]
    pub channels: Vec<u8>,
    pub workers: usize,
    pub stride_samples: usize,
    pub filter: Filter,
    #[serde(default, skip_serializing_if = "is_false")]
    pub debug_output: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_label: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub debug_overwrite: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snr_background_window: Option<Window>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snr_signal_window: Option<Window>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub save_npy: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Filter {
    BoxcarLegacy { half_window_cycles: f64 },
}

impl Filter {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::BoxcarLegacy { .. } => "boxcar_legacy",
        }
    }

    pub fn half_window_cycles(&self) -> f64 {
        let Self::BoxcarLegacy { half_window_cycles } = self;
        *half_window_cycles
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfigV7 {
    pub version: u32,
    // FR-01/FR-06 (Issue #264, Card C): absent unrelated sections fill with
    // inert defaults at parse so stage-minimal configs load, mirroring the
    // native Card A contract. `deny_unknown_fields` stays, so misspelled keys
    // are still rejected. `version` and the roles/channels core (derived from
    // sensors/signals/reference) stay required: an empty sensor set still
    // fails validation. No schema version bump (still v7).
    #[serde(default)]
    pub scope: Scope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generator: Option<Generator>,
    #[serde(default)]
    pub data: Data,
    #[serde(default)]
    pub sensors: Vec<Sensor>,
    #[serde(default)]
    pub pulse: Pulse,
    #[serde(default)]
    pub reference: Reference,
    #[serde(default)]
    pub lockin: LockinV7,
    #[serde(default)]
    pub phase: Phase,
    #[serde(default)]
    pub moke: Moke,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signals: Vec<Signal>,
    #[serde(default)]
    pub plot: Plot,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LockinV7 {
    #[serde(alias = "signal_channels")]
    pub channels: Vec<u8>,
    pub workers: usize,
    pub stride_samples: usize,
    pub window: LockinWindowV7,
    pub estimator: LockinEstimatorV7,
    #[serde(default, skip_serializing_if = "is_false")]
    pub debug_output: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_label: Option<String>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub debug_overwrite: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snr_background_window: Option<Window>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snr_signal_window: Option<Window>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub save_npy: bool,
}

/// Inert stage-minimal default (FR-01/FR-06, Issue #264, Card C): mirrors the
/// native Card A default (legacy boxcar contract with an empty channel set,
/// so every signal-gated target rejects configs that rely on this default).
/// `stride_samples` mirrors the canonical fixture because `sensor`
/// cross-reads it; the value only matters once an explicit `[lockin]`
/// replaces this default.
impl Default for LockinV7 {
    fn default() -> Self {
        Self {
            channels: Vec::new(),
            workers: 1,
            stride_samples: 100,
            window: LockinWindowV7::default(),
            estimator: LockinEstimatorV7::default(),
            debug_output: false,
            debug_label: None,
            debug_overwrite: false,
            snr_background_window: None,
            snr_signal_window: None,
            save_npy: false,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LockinWindowV7 {
    pub kind: LockinWindowKindV7,
    pub half_window_cycles: f64,
    pub edge_policy: LockinEdgePolicyV7,
}

/// Mirrors the native Card A default: the canonical reference-cycles window.
impl Default for LockinWindowV7 {
    fn default() -> Self {
        Self {
            kind: LockinWindowKindV7::ReferenceCycles,
            half_window_cycles: 1.0,
            edge_policy: LockinEdgePolicyV7::LegacyTrim,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LockinWindowKindV7 {
    ReferenceCycles,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LockinEdgePolicyV7 {
    LegacyTrim,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum LockinEstimatorV7 {
    BoxcarLegacy {},
    JointHarmonicGls(JointHarmonicGlsConfigV7),
}

impl LockinEstimatorV7 {
    pub fn name(&self) -> &'static str {
        match self {
            Self::BoxcarLegacy {} => "boxcar_legacy",
            Self::JointHarmonicGls(_) => "joint_harmonic_gls",
        }
    }
}

/// Mirrors the native Card A default: the legacy boxcar estimator.
impl Default for LockinEstimatorV7 {
    fn default() -> Self {
        Self::BoxcarLegacy {}
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct JointHarmonicGlsConfigV7 {
    pub fit_harmonics: Vec<usize>,
    pub output_harmonics: Vec<usize>,
    #[serde(default)]
    pub envelope_degree: u8,
    pub noise_mode: GlsNoiseModeV7,
    #[serde(default = "default_gls_covariance_output")]
    pub covariance_output: GlsCovarianceOutputV7,
    #[serde(default)]
    pub failure_policy: GlsFailurePolicyV7,
    #[serde(default)]
    pub calibration_source: GlsCalibrationSourceV7,
    #[serde(default)]
    pub calibrations: Vec<EstimatorCalibrationV7>,
}

/// Pre-pulse calibration source (FR-01 mirror): `artifact` (default) keeps
/// file-bound behavior; `prepulse` derives from `pulse.background_before`.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GlsCalibrationSourceV7 {
    #[default]
    Artifact,
    Prepulse,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GlsNoiseModeV7 {
    Identity,
    PhaseDiagonal,
    StationaryCorrelated,
    PhaseCorrelated,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GlsCovarianceOutputV7 {
    None,
    #[default]
    Diagonal,
    Full,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub(crate) enum GlsFailurePolicyV7 {
    #[default]
    Error,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct EstimatorCalibrationV7 {
    pub channel: u8,
    pub path: String,
    pub sha256: String,
}

fn default_gls_covariance_output() -> GlsCovarianceOutputV7 {
    GlsCovarianceOutputV7::Diagonal
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Phase {
    pub offsets: Vec<NumberOrExpression>,
}

impl Serialize for Phase {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("Phase", 1)?;
        state.serialize_field(
            "offsets",
            &self
                .offsets
                .iter()
                .map(|value| value.evaluate().unwrap_or(f64::NAN))
                .collect::<Vec<_>>(),
        )?;
        state.end()
    }
}

/// Inert stage-minimal default (FR-01/FR-06, Issue #264, Card C): six zero
/// offsets satisfy the length-6 finite validation and mirror the native Card
/// A default; the phase stage additionally requires published lock-in
/// results, so a fresh minimal config cannot act on this default.
impl Default for Phase {
    fn default() -> Self {
        Self {
            offsets: vec![NumberOrExpression::Number(0.0); 6],
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub(crate) enum NumberOrExpression {
    Number(f64),
    Expression(String),
}

impl NumberOrExpression {
    pub fn evaluate(&self) -> Result<f64, String> {
        match self {
            Self::Number(value) => Ok(*value),
            Self::Expression(expression) => evaluate_expression(expression),
        }
    }
}

fn evaluate_expression(expression: &str) -> Result<f64, String> {
    use fasteval::Evaler;
    if contains_print_call(expression) {
        return Err("print() is not allowed in config values".to_string());
    }
    let mut slab = fasteval::Slab::new();
    let parser = fasteval::Parser::new();
    let parsed = parser
        .parse(expression.trim(), &mut slab.ps)
        .map_err(|error| error.to_string())?;
    let mut namespace =
        std::collections::BTreeMap::from([("pi".to_string(), std::f64::consts::PI)]);
    parsed
        .from(&slab.ps)
        .eval(&slab, &mut namespace)
        .map_err(|error| error.to_string())
}

fn contains_print_call(expression: &str) -> bool {
    let bytes = expression.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index].is_ascii_alphabetic() || bytes[index] == b'_' {
            let start = index;
            index += 1;
            while index < bytes.len()
                && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
            {
                index += 1;
            }
            if &expression[start..index] == "print" {
                let mut next = index;
                while next < bytes.len() && bytes[next].is_ascii_whitespace() {
                    next += 1;
                }
                if matches!(bytes.get(next), Some(b'(' | b'[')) {
                    return true;
                }
            }
        } else {
            index += 1;
        }
    }
    false
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Moke {
    pub sensor: u8,
    pub method: MokeMethod,
    pub factor: f64,
}

/// Inert stage-minimal default (FR-01/FR-06, Issue #264, Card C): mirrors the
/// native Card A default. Sensor channel 1 is the canonical first sensor
/// channel, so canonical minimal skeletons keep loading; skeletons using
/// other sensor channels must carry an explicit `[moke]`.
impl Default for Moke {
    fn default() -> Self {
        Self {
            sensor: 1,
            method: MokeMethod::Standard,
            factor: 1.0,
        }
    }
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum MokeMethod {
    Standard,
    Harmonics,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Plot {
    pub mode: PlotMode,
    #[serde(skip_serializing)]
    pub output_dir: Option<String>,
    pub max_points: usize,
    pub decimation: PlotDecimation,
    pub on_error: PlotErrorMode,
}

impl Default for Plot {
    fn default() -> Self {
        Self {
            mode: PlotMode::Save,
            output_dir: None,
            max_points: 100_000,
            decimation: PlotDecimation::Stride,
            on_error: PlotErrorMode::Warn,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PlotMode {
    Off,
    #[default]
    Save,
    Interactive,
    Both,
}

impl PlotMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Save => "save",
            Self::Interactive => "interactive",
            Self::Both => "both",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum PlotErrorMode {
    #[default]
    Warn,
    Fail,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlotDecimation {
    None,
    #[default]
    Stride,
    MinMax,
}

fn is_false(value: &bool) -> bool {
    !value
}
