use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfigV5 {
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
    pub kerr: Kerr,
    #[serde(default)]
    pub plot: Plot,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Scope {
    pub model: String,
    pub connection: String,
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

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reference {
    pub channel: u8,
    pub fft_window: Window,
    pub stride_samples: usize,
    pub window_samples: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Lockin {
    pub signal_channels: Vec<u8>,
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
pub(crate) struct Kerr {
    pub sensor: u8,
    pub method: KerrMethod,
    pub factor: f64,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum KerrMethod {
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
