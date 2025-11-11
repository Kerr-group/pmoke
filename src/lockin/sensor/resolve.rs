use crate::config::Config;
use crate::utils::channels::build_channel_list;
use anyhow::{Result, anyhow, bail};

pub fn sensor_column_indices(cfg: &Config) -> Result<(Vec<u8>, Vec<usize>)> {
    let sensor_ch = &cfg.roles.sensor_ch;
    if sensor_ch.is_empty() {
        bail!("sensor channel is not specified in the configuration");
    }

    let channels = build_channel_list(cfg)?;
    let mut col_idx = Vec::with_capacity(sensor_ch.len());

    for &ch in sensor_ch {
        let idx = channels.iter().position(|c| *c == ch).ok_or_else(|| {
            anyhow!(
                "sensor channel {} not found in fetched channels {:?}",
                ch,
                channels
            )
        })?;
        col_idx.push(idx);
    }

    Ok((sensor_ch.clone(), col_idx))
}
