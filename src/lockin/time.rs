use crate::config::Config;
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

pub fn time_stride_builder(cfg: &Config) -> Result<Vec<f64>> {
    let t = time_builder(cfg)?;
    let stride_samples = cfg.lockin.stride_samples;
    let t_stride = t
        .iter()
        .step_by(stride_samples)
        .cloned()
        .collect::<Vec<f64>>();
    Ok(t_stride)
}
