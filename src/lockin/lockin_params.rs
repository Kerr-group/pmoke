use crate::config::Config;
use anyhow::{Result, anyhow};
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy)]
pub struct LockinParams {
    pub dt: f64,
    pub stride: usize,
    #[allow(dead_code)]
    pub fil_length: usize,
    #[allow(dead_code)]
    pub length: usize,
    #[allow(dead_code)]
    pub f_ref: f64,

    pub omega: f64,
    pub t_fil: f64,
    pub n_fil: usize,
    pub diff_t: f64,
    pub n_int: usize,
    pub i_start: usize,
    pub i_end: usize,
}

impl LockinParams {
    fn init(dt: f64, stride: usize, fil_length: usize, length: usize, f_ref: f64) -> Self {
        let omega = 2.0 * PI * f_ref;
        let t_fil = (1.0 / f_ref) * fil_length as f64;
        let n_fil = (t_fil / dt).floor() as usize;
        let diff_t = t_fil - (n_fil as f64) * dt;
        let n_int = ((length - 1) / stride) + 1;
        let i_start = 2 + (n_fil + 1) / stride;
        let i_end = n_int - i_start;

        Self {
            dt,
            stride,
            fil_length,
            length,
            f_ref,
            omega,
            t_fil,
            n_fil,
            diff_t,
            n_int,
            i_start,
            i_end,
        }
    }

    pub fn from_config(cfg: &Config, f_ref: f64) -> Result<Self> {
        let dt = cfg.timebase.dt;
        let stride = cfg.lockin.stride_samples;
        let fil_length = cfg.lockin.filter_length_samples;

        let length = cfg
            .instruments
            .as_ref()
            .ok_or_else(|| anyhow!("Instruments config is missing"))?
            .oscilloscope
            .memory_depth;

        Ok(Self::init(dt, stride, fil_length, length, f_ref))
    }
    pub fn from_slice(t: &[f64], f_ref: f64, fil_length: usize, stride: usize) -> Result<Self> {
        if t.len() < 2 {
            return Err(anyhow!("Time slice 't' must have at least 2 elements"));
        }
        let dt = t[1] - t[0];
        let length = t.len();

        Ok(Self::init(dt, stride, fil_length, length, f_ref))
    }
}
