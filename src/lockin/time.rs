use crate::config::Config;
use crate::lockin::stride::{get_li_range, li_stride_1d, stride_1d};
use anyhow::Result;

pub fn time_builder(cfg: &Config) -> Result<Vec<f64>> {
    let t0 = cfg.timebase.t0;
    let dt = cfg.timebase.dt;
    let num_points = cfg
        .instruments
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Instruments configuration is missing."))?
        .oscilloscope
        .memory_depth;

    let time: Vec<f64> = (0..num_points).map(|i| t0 + i as f64 * dt).collect();
    Ok(time)
}
