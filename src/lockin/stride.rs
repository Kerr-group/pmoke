use crate::config::Config;
use crate::lockin::lockin_params::LockinParams;
use crate::utils::time_axis::TimeAxisRef;
use anyhow::Result;

pub fn get_li_range<'a>(
    cfg: &Config,
    t: impl Into<TimeAxisRef<'a>>,
    f_ref: f64,
) -> Result<(usize, usize, usize)> {
    let t = t.into();
    let dt = t
        .dt()
        .ok_or_else(|| anyhow::anyhow!("time axis must contain at least two samples"))?;
    let params = LockinParams::from_geometry(t.len(), dt, f_ref, &cfg.lockin)?;

    Ok((params.i_start, params.i_end, params.stride))
}

pub fn li_stride_1d<'a>(
    cfg: &Config,
    t: impl Into<TimeAxisRef<'a>>,
    data: &[f64],
    f_ref: f64,
) -> Result<Vec<f64>> {
    let (start_idx, end_idx, stride_samples) = get_li_range(cfg, t, f_ref)?;

    Ok((start_idx..=end_idx)
        .map(|idx| data[idx * stride_samples])
        .collect())
}

pub fn li_stride_time(cfg: &Config, t: TimeAxisRef<'_>, f_ref: f64) -> Result<Vec<f64>> {
    let (start_idx, end_idx, stride_samples) = get_li_range(cfg, t, f_ref)?;
    Ok((start_idx..=end_idx)
        .map(|idx| t.value_at(idx * stride_samples))
        .collect())
}

pub fn li_stride_2d<'a, C>(
    cfg: &Config,
    t: impl Into<TimeAxisRef<'a>>,
    data: &[C],
    f_ref: f64,
) -> Result<Vec<Vec<f64>>>
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
