use crate::config::{Config, Lockin, LockinLpfKind};
use anyhow::{Result, anyhow, bail};
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy)]
pub struct LockinParams {
    pub dt: f64,
    pub sample_rate: f64,
    pub output_rate: f64,
    pub stride: usize,
    #[allow(dead_code)]
    pub length: usize,
    #[allow(dead_code)]
    pub f_ref: f64,
    pub lpf_kind: LockinLpfKind,
    pub lpf_stopband_atten_db: f64,
    pub cutoff_hz: Option<f64>,
    pub cutoff_source: CutoffSource,
    pub fallback_used: bool,

    pub omega: f64,
    pub t_half: f64,
    pub n_half: usize,
    pub i_start: usize,
    pub i_end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutoffSource {
    ExplicitHz,
    ReferenceRatio,
    Fallback,
    EnbwMatch,
    None,
}

impl CutoffSource {
    pub fn as_str(self) -> &'static str {
        match self {
            CutoffSource::ExplicitHz => "explicit_hz",
            CutoffSource::ReferenceRatio => "reference_ratio",
            CutoffSource::Fallback => "fallback",
            CutoffSource::EnbwMatch => "enbw_match",
            CutoffSource::None => "none",
        }
    }
}

impl LockinParams {
    fn init(dt: f64, length: usize, f_ref: f64, lockin: &Lockin) -> Result<Self> {
        let stride = lockin.stride_samples;
        let sample_rate = 1.0 / dt;
        let output_rate = sample_rate / stride as f64;
        let half_window_cycles = lockin.lpf_half_window_cycles;
        let omega = 2.0 * PI * f_ref;
        let t_half = half_window_cycles / f_ref;
        let n_half = ((t_half / dt).floor() as usize).max(1);
        let n_int = ((length - 1) / stride) + 1;
        let i_start = 2 + (n_half + 1) / stride;
        let i_end = n_int.saturating_sub(i_start);
        let (cutoff_hz, cutoff_source, fallback_used) =
            resolve_cutoff_hz(lockin, f_ref, t_half)?;

        if let Some(cutoff_hz) = cutoff_hz {
            if cutoff_hz >= 0.45 * output_rate {
                bail!(
                    "lockin cutoff_hz ({}) must be < 0.45 * output_rate ({})",
                    cutoff_hz,
                    0.45 * output_rate
                );
            }
            if cutoff_hz >= 0.4 * output_rate {
                eprintln!(
                    "⚠️ lockin cutoff_hz ({}) is close to output Nyquist; output_rate={}",
                    cutoff_hz, output_rate
                );
            }
        }

        Ok(Self {
            dt,
            sample_rate,
            output_rate,
            stride,
            length,
            f_ref,
            lpf_kind: lockin.lpf_kind,
            lpf_stopband_atten_db: lockin.lpf_stopband_atten_db,
            cutoff_hz,
            cutoff_source,
            fallback_used,
            omega,
            t_half,
            n_half,
            i_start,
            i_end,
        })
    }

    pub fn from_config(cfg: &Config, f_ref: f64) -> Result<Self> {
        let dt = cfg.timebase.dt;
        let length = cfg
            .instruments
            .as_ref()
            .ok_or_else(|| anyhow!("Instruments config is missing"))?
            .oscilloscope
            .memory_depth;

        Self::init(dt, length, f_ref, &cfg.lockin)
    }

    pub fn from_slice(t: &[f64], f_ref: f64, lockin: &Lockin) -> Result<Self> {
        if t.len() < 2 {
            return Err(anyhow!("Time slice 't' must have at least 2 elements"));
        }
        let dt = t[1] - t[0];
        let length = t.len();

        Self::init(dt, length, f_ref, lockin)
    }
}

fn resolve_cutoff_hz(
    lockin: &Lockin,
    f_ref: f64,
    t_half: f64,
) -> Result<(Option<f64>, CutoffSource, bool)> {
    match lockin.lpf_kind {
        LockinLpfKind::BoxcarLegacy => Ok((None, CutoffSource::None, false)),
        LockinLpfKind::FirBoxcarEnbw => Ok((None, CutoffSource::EnbwMatch, false)),
        LockinLpfKind::FirZeroPhase => {
            if let Some(cutoff_hz) = lockin.lpf_cutoff_hz {
                Ok((Some(cutoff_hz), CutoffSource::ExplicitHz, false))
            } else if let Some(ratio) = lockin.lpf_cutoff_ref_ratio {
                Ok((Some(ratio * f_ref), CutoffSource::ReferenceRatio, false))
            } else {
                Ok((Some(0.5 / t_half), CutoffSource::Fallback, true))
            }
        }
    }
}
