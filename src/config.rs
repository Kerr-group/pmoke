use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::str::FromStr;
use std::{collections::BTreeSet, fs, path::Path};

fn eval_f64_expr(s: &str) -> Result<f64> {
    let expr =
        meval::Expr::from_str(s.trim()).map_err(|e| anyhow!("invalid expression '{s}': {e}"))?;

    let mut ctx = meval::Context::new();
    ctx.var("pi", std::f64::consts::PI);

    expr.eval_with_context(ctx)
        .map_err(|e| anyhow!("failed to evaluate '{s}': {e}"))
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

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,

    pub instruments: Option<Instruments>,
    pub timebase: Timebase,
    pub roles: Roles,
    pub channels: Vec<Channel>,

    pub pulse: Pulse,
    pub reference: Reference,
    pub lockin: Lockin,
    pub phase: Phase,
    pub kerr: Kerr,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Instruments {
    #[serde(rename = "function_generator")]
    pub function_generator: Option<FunctionGenerator>,
    pub oscilloscope: Oscilloscope,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FunctionGenerator {
    pub connection: Connection,
    #[serde(default)]
    pub model: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Oscilloscope {
    pub connection: Connection,
    #[serde(default)]
    pub model: String,
    pub memory_depth: usize,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "protocol", rename_all = "lowercase")]
pub enum Connection {
    Gpib { board: u8, address: u8 },
    Tcpip { ip: String, port: u16 },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Timebase {
    pub t0: f64,
    pub dt: f64,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Roles {
    #[serde(default, deserialize_with = "one_or_many")]
    pub sensor_ch: Vec<u8>,
    #[serde(default, deserialize_with = "one_or_many")]
    pub reference_ch: Vec<u8>,
    #[serde(default, deserialize_with = "one_or_many")]
    pub signal_ch: Vec<u8>,
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

#[derive(Debug, Serialize, Deserialize)]
pub struct Channel {
    pub index: u8,
    #[serde(default)]
    pub factor: Option<f64>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub unit_out: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Pulse {
    pub bg_window_before: Window,
    pub bg_window_after: Window,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Reference {
    pub fft_window: Window,
    pub stride_samples: usize,
    pub window_samples: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub struct Window {
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Lockin {
    pub workers: usize,
    pub stride_samples: usize,
    #[serde(default)]
    pub filter_length_samples: Option<usize>,
    #[serde(default = "default_lockin_demodulation")]
    pub demodulation: LockinDemodulation,
    #[serde(default)]
    pub lpf_kind: Option<LockinLpfKind>,
    #[serde(default)]
    pub lpf_half_window_cycles: Option<f64>,
    #[serde(default = "default_lockin_stopband_atten_db")]
    pub lpf_stopband_atten_db: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LockinDemodulation {
    Complex,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LockinLpfKind {
    FirZeroPhase,
    BoxcarLegacy,
}

fn default_lockin_demodulation() -> LockinDemodulation {
    LockinDemodulation::Complex
}

fn default_lockin_stopband_atten_db() -> f64 {
    60.0
}

impl Lockin {
    pub fn half_window_cycles(&self) -> Result<f64> {
        let half_window_cycles = self
            .lpf_half_window_cycles
            .or_else(|| self.filter_length_samples.map(|v| v as f64))
            .ok_or_else(|| {
                anyhow!(
                    "lockin.lpf_half_window_cycles or deprecated lockin.filter_length_samples must be set"
                )
            })?;

        if half_window_cycles <= 0.0 {
            bail!(
                "lockin half window cycles must be positive (got {})",
                half_window_cycles
            );
        }

        Ok(half_window_cycles)
    }

    pub fn effective_lpf_kind(&self) -> LockinLpfKind {
        self.lpf_kind.unwrap_or_else(|| {
            if self.filter_length_samples.is_some() && self.lpf_half_window_cycles.is_none() {
                LockinLpfKind::BoxcarLegacy
            } else {
                LockinLpfKind::FirZeroPhase
            }
        })
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Phase {
    pub use_signal_ch: Vec<u8>,
    #[serde(default, deserialize_with = "de_vec_f64_or_expr")]
    pub m_omega_t0_offset: Vec<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum KerrType {
    Standard,
    Harmonics,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Kerr {
    pub use_sensor_ch: u8,
    pub kerr_type: KerrType,
    pub factor: f64,
}

// From &str
pub fn from_str(s: &str) -> Result<Config> {
    let de = toml::de::Deserializer::parse(s).map_err(|e| anyhow!("toml parse error: {e}"))?;

    let mut cfg: Config =
        serde_path_to_error::deserialize(de).map_err(|e| anyhow!("{} at {}", e, e.path()))?;

    cfg.validate()?;

    Ok(cfg)
}

// From file
pub fn from_path(path: impl AsRef<Path>) -> Result<Config> {
    let p = path.as_ref();
    let text = fs::read_to_string(p).with_context(|| format!("failed to read {}", p.display()))?;
    from_str(&text)
}

impl Config {
    pub fn validate(&mut self) -> Result<()> {
        if self.version != 1 {
            bail!("unsupported config version: {}", self.version);
        }
        if self.timebase.dt <= 0.0 {
            bail!("timebase.dt must be positive");
        }
        if self.lockin.workers == 0 {
            bail!("lockin.workers must be positive");
        }
        if self.lockin.stride_samples == 0 {
            bail!("lockin.stride_samples must be positive");
        }
        let _ = self.lockin.half_window_cycles()?;
        if self.lockin.lpf_stopband_atten_db <= 0.0 {
            bail!(
                "lockin.lpf_stopband_atten_db must be positive (got {})",
                self.lockin.lpf_stopband_atten_db
            );
        }
        let mut seen = BTreeSet::new();
        for ch in &self.channels {
            if !seen.insert(ch.index) {
                bail!("duplicate channel index: {}", ch.index);
            }
        }

        if self.phase.m_omega_t0_offset.len() != 6 {
            bail!(
                "phase.m_omega_t0_offset must have length 6 (got {})",
                self.phase.m_omega_t0_offset.len()
            );
        }

        let kerr_ch = self.kerr.use_sensor_ch;
        if !seen.contains(&kerr_ch) {
            bail!(
                "kerr.use_sensor_ch ({}) is not defined in channels",
                kerr_ch
            );
        }
        if !self.roles.sensor_ch.contains(&kerr_ch) {
            bail!(
                "kerr.use_sensor_ch ({}) is not included in roles.sensor_ch",
                kerr_ch
            );
        }

        let has = |n: u8| seen.contains(&n);
        for (name, arr) in [
            ("sensor_ch", &self.roles.sensor_ch),
            ("reference_ch", &self.roles.reference_ch),
            ("signal_ch", &self.roles.signal_ch),
        ] {
            for &idx in arr.iter() {
                if !has(idx) {
                    bail!("roles.{name} contains undefined channel index: {idx}");
                }
            }
        }

        for ch in &self.channels {
            let idx = ch.index;
            let used_in_roles = self.roles.sensor_ch.contains(&idx)
                || self.roles.reference_ch.contains(&idx)
                || self.roles.signal_ch.contains(&idx);

            if !used_in_roles {
                bail!(
                    "channel index {} is defined in `channels` but not used in any roles (sensor_ch / reference_ch / signal_ch)",
                    idx
                );
            }
        }

        let check_win = |label: &str, w: Window| -> Result<()> {
            if !matches!(w.start.partial_cmp(&w.end), Some(std::cmp::Ordering::Less)) {
                bail!(
                    "{label}: start must be < end (start={}, end={})",
                    w.start,
                    w.end
                );
            }
            Ok(())
        };
        check_win("pulse.bg_window_before", self.pulse.bg_window_before)?;
        check_win("pulse.bg_window_after", self.pulse.bg_window_after)?;

        self.channels.sort_by_key(|ch| ch.index);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Lockin, LockinDemodulation, LockinLpfKind};

    #[test]
    fn deprecated_filter_length_maps_to_half_window_cycles() {
        let lockin = Lockin {
            workers: 4,
            stride_samples: 1000,
            filter_length_samples: Some(1),
            demodulation: LockinDemodulation::Complex,
            lpf_kind: None,
            lpf_half_window_cycles: None,
            lpf_stopband_atten_db: 60.0,
        };

        assert_eq!(lockin.half_window_cycles().unwrap(), 1.0);
        assert_eq!(lockin.effective_lpf_kind(), LockinLpfKind::BoxcarLegacy);
    }

    #[test]
    fn explicit_half_window_cycles_take_precedence() {
        let lockin = Lockin {
            workers: 4,
            stride_samples: 1000,
            filter_length_samples: Some(1),
            demodulation: LockinDemodulation::Complex,
            lpf_kind: Some(LockinLpfKind::FirZeroPhase),
            lpf_half_window_cycles: Some(1.5),
            lpf_stopband_atten_db: 60.0,
        };

        assert_eq!(lockin.half_window_cycles().unwrap(), 1.5);
        assert_eq!(lockin.effective_lpf_kind(), LockinLpfKind::FirZeroPhase);
    }

    #[test]
    fn new_half_window_defaults_to_fir() {
        let lockin = Lockin {
            workers: 4,
            stride_samples: 1000,
            filter_length_samples: None,
            demodulation: LockinDemodulation::Complex,
            lpf_kind: None,
            lpf_half_window_cycles: Some(1.0),
            lpf_stopband_atten_db: 60.0,
        };

        assert_eq!(lockin.effective_lpf_kind(), LockinLpfKind::FirZeroPhase);
    }
}
