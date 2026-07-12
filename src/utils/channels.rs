use crate::config::Config;
use anyhow::{Result, bail};

pub fn build_channel_list(cfg: &Config) -> Result<Vec<u8>> {
    let mut channels: Vec<u8> = Vec::new();

    channels.extend(cfg.roles.sensor_ch.iter().copied());
    channels.extend(cfg.roles.signal_ch.iter().copied());
    channels.push(cfg.roles.reference_ch);

    channels.sort();

    for i in 1..channels.len() {
        if channels[i] == channels[i - 1] {
            bail!("Duplicate channel detected: ch{}", channels[i]);
        }
    }

    Ok(channels)
}

#[cfg(test)]
mod tests {
    use super::build_channel_list;

    #[test]
    fn includes_every_sensor_reference_and_signal_channel_in_numeric_order() {
        let mut cfg = crate::test_support::test_config(vec![4, 1], vec![3]);
        cfg.roles.reference_ch = 2;

        assert_eq!(build_channel_list(&cfg).unwrap(), vec![1, 2, 3, 4]);
    }

    #[test]
    fn rejects_a_channel_assigned_to_more_than_one_role() {
        let mut cfg = crate::test_support::test_config(vec![1], vec![3]);
        cfg.roles.reference_ch = 3;

        let error = build_channel_list(&cfg).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Duplicate channel detected: ch3")
        );
    }
}
