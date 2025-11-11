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

pub fn reference_column_index(cfg: &Config) -> Result<(u8, usize)> {
    let ref_chs = &cfg.roles.reference_ch;
    if ref_chs.is_empty() {
        bail!("reference channel is not specified in the configuration");
    }
    if ref_chs.len() != 1 {
        bail!(
            "expected exactly one reference channel, but found {}",
            ref_chs.len()
        );
    }
    let ref_ch = ref_chs[0];

    let channels = build_channel_list(cfg)?;
    let col_idx = channels.iter().position(|c| *c == ref_ch).ok_or_else(|| {
        anyhow!(
            "reference channel {} not found in fetched channels {:?}",
            ref_ch,
            channels
        )
    })?;

    Ok((ref_ch, col_idx))
}

pub fn signal_column_indices(cfg: &Config) -> Result<(Vec<u8>, Vec<usize>)> {
    let signal_ch = &cfg.roles.signal_ch;
    if signal_ch.is_empty() {
        bail!("signal channel is not specified in the configuration");
    }

    let channels = build_channel_list(cfg)?;
    let mut col_idx = Vec::with_capacity(signal_ch.len());

    for &ch in signal_ch {
        let idx = channels.iter().position(|c| *c == ch).ok_or_else(|| {
            anyhow!(
                "signal channel {} not found in fetched channels {:?}",
                ch,
                channels
            )
        })?;
        col_idx.push(idx);
    }

    Ok((signal_ch.clone(), col_idx))
}
