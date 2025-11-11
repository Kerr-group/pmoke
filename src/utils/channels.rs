use crate::config::Config;
use anyhow::{Result, bail};

pub fn build_channel_list(cfg: &Config) -> Result<Vec<u8>> {
    let mut channels: Vec<u8> = Vec::new();

    channels.extend(cfg.roles.sensor_ch.iter().copied());
    channels.extend(cfg.roles.signal_ch.iter().copied());
    channels.extend(cfg.roles.reference_ch.iter().copied());

    channels.sort();

    for i in 1..channels.len() {
        if channels[i] == channels[i - 1] {
            bail!("Duplicate channel detected: ch{}", channels[i]);
        }
    }

    Ok(channels)
}
