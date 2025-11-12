use crate::config::Config;
use crate::lockin::lockin_params::LockinParams;
use anyhow::Result;

pub fn get_li_range(cfg: &Config, f_ref: f64) -> Result<(usize, usize, usize)> {
    let params = LockinParams::from_config(cfg, f_ref)?;

    Ok((params.i_start, params.i_end, params.stride))
}

pub fn stride_1d(data: &[f64], stride: usize) -> Vec<f64> {
    data.iter().step_by(stride).cloned().collect()
}

pub fn stride_2d(data: &[Vec<f64>], stride: usize) -> Vec<Vec<f64>> {
    data.iter().map(|col| stride_1d(col, stride)).collect()
}

pub fn li_stride_1d(cfg: &Config, data: &[f64], f_ref: f64) -> Result<Vec<f64>> {
    let (start_idx, end_idx, stride_samples) = get_li_range(cfg, f_ref)?;

    let strided_data = stride_1d(data, stride_samples);
    let sliced_data = &strided_data[start_idx..=end_idx];
    Ok(sliced_data.to_vec())
}

pub fn li_stride_2d(cfg: &Config, data: &[Vec<f64>], f_ref: f64) -> Result<Vec<Vec<f64>>> {
    let (start_idx, end_idx, stride_samples) = get_li_range(cfg, f_ref)?;

    let strided_data = stride_2d(data, stride_samples);
    let sliced_data: Vec<Vec<f64>> = strided_data
        .iter()
        .map(|col| col[start_idx..=end_idx].to_vec())
        .collect();
    Ok(sliced_data)
}
