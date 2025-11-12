use anyhow::{Context, Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs, path::Path};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,

    pub instruments: Option<Instruments>,
    pub timebase: Timebase,
    pub roles: Roles,
    pub channels: Vec<Channel>,

    pub pulse: Pulse,
    pub lockin: Lockin,
    pub phase: Phase,
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

#[derive(Debug, Serialize, Deserialize, Clone, Copy)]
pub struct Window {
    pub start: f64,
    pub end: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Lockin {
    pub workers: usize,
    pub filter_length_samples: usize,
    pub stride_samples: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Phase {
    pub use_ch: Vec<u8>,
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
        let mut seen = BTreeSet::new();
        for ch in &self.channels {
            if !seen.insert(ch.index) {
                bail!("duplicate channel index: {}", ch.index);
            }
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
