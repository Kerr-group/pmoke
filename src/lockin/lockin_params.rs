use crate::config::{Config, Lockin, LockinLpfKind};
use anyhow::{Result, anyhow};
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy)]
pub struct LockinParams {
    pub dt: f64,
    pub stride: usize,
    #[allow(dead_code)]
    pub length: usize,
    #[allow(dead_code)]
    pub f_ref: f64,
    pub lpf_kind: LockinLpfKind,
    pub lpf_stopband_atten_db: f64,

    pub omega: f64,
    pub t_half: f64,
    pub n_half: usize,
    pub i_start: usize,
    pub i_end: usize,
}

impl LockinParams {
    fn init(dt: f64, length: usize, f_ref: f64, lockin: &Lockin) -> Result<Self> {
        let stride = lockin.stride_samples;
        let half_window_cycles = lockin.half_window_cycles()?;
        let omega = 2.0 * PI * f_ref;
        let t_half = half_window_cycles / f_ref;
        let n_half = ((t_half / dt).floor() as usize).max(1);
        let n_int = ((length - 1) / stride) + 1;
        let i_start = 2 + (n_half + 1) / stride;
        let i_end = n_int.saturating_sub(i_start);

        Ok(Self {
            dt,
            stride,
            length,
            f_ref,
            lpf_kind: lockin.effective_lpf_kind(),
            lpf_stopband_atten_db: lockin.lpf_stopband_atten_db,
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
