use crate::config::Config;
use crate::lockin::lockin_params::LockinParams;
use anyhow::Result;

pub fn get_li_range(cfg: &Config, t: &[f64], f_ref: f64) -> Result<(usize, usize, usize)> {
    let params = LockinParams::from_slice(t, f_ref, &cfg.lockin)?;

    Ok((params.i_start, params.i_end, params.stride))
}

pub fn li_stride_1d(cfg: &Config, t: &[f64], data: &[f64], f_ref: f64) -> Result<Vec<f64>> {
    let (start_idx, end_idx, stride_samples) = get_li_range(cfg, t, f_ref)?;

    Ok((start_idx..=end_idx)
        .map(|idx| data[idx * stride_samples])
        .collect())
}

pub fn li_stride_2d<C>(cfg: &Config, t: &[f64], data: &[C], f_ref: f64) -> Result<Vec<Vec<f64>>>
where
    C: AsRef<[f64]>,
{
    let (start_idx, end_idx, stride_samples) = get_li_range(cfg, t, f_ref)?;

    let sliced_data = data
        .iter()
        .map(|col| {
            let col = col.as_ref();
            (start_idx..=end_idx)
                .map(|idx| col[idx * stride_samples])
                .collect()
        })
        .collect();
    Ok(sliced_data)
}
