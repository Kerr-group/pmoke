use crate::communications::oscilloscope::OscilloscopeHandler;
use crate::config::Config;
use anyhow::{Context, Result, bail};
use colored::*;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::time::Instant;

pub fn fetch(cfg: &Config) -> Result<()> {
    let mut handler = OscilloscopeHandler::initialize(cfg)
        .context("failed to initialize oscilloscope handler")?;

    let osc_cfg = cfg
        .instruments
        .as_ref()
        .and_then(|ins| ins.oscilloscope.as_ref())
        .context("oscilloscope configuration is missing")?;

    let depth = osc_cfg.memory_depth;

    let channels = build_channel_list(cfg)?;

    println!(
        "⏳ Fetching {} samples from {} channels...",
        depth,
        channels.len()
    );

    let t_fetch_start = Instant::now();
    let data_per_ch = fetch_all_channels(&mut handler, &channels, depth)?;
    let t_fetch_end = Instant::now();

    let fetch_elapsed = t_fetch_end - t_fetch_start;

    println!(
        "✅ Fetched {} samples from {} channels in {:.2?} ({:.2} samples/sec)",
        depth,
        channels.len(),
        fetch_elapsed,
        (depth * channels.len()) as f64 / fetch_elapsed.as_secs_f64()
    );

    let t_write_start = Instant::now();
    write_csv("raw.csv", &channels, &data_per_ch, depth)?;
    let t_write_end = Instant::now();

    println!(
        "📝 Data written to raw.csv in {:.2?}",
        t_write_end - t_write_start
    );

    Ok(())
}

fn build_channel_list(cfg: &Config) -> Result<Vec<u8>> {
    let mut channels: Vec<u8> = Vec::new();

    channels.extend(cfg.roles.sensor_ch.iter().map(|&ch| ch as u8));
    channels.extend(cfg.roles.signal_ch.iter().map(|&ch| ch as u8));
    channels.extend(cfg.roles.reference_ch.iter().map(|&ch| ch as u8));

    channels.sort();

    for i in 1..channels.len() {
        if channels[i] == channels[i - 1] {
            bail!("Duplicate channel detected: ch{}", channels[i]);
        }
    }

    Ok(channels)
}

fn fetch_all_channels(
    handler: &mut OscilloscopeHandler,
    channels: &[u8],
    depth: usize,
) -> Result<Vec<Vec<f64>>> {
    let mut data_per_ch: Vec<Vec<f64>> = Vec::with_capacity(channels.len());

    for &ch in channels {
        let v = handler
            .fetch(ch, depth)
            .with_context(|| format!("failed to fetch channel {ch}"))?;

        if v.len() != depth {
            bail!(
                "channel {ch} returned {} samples, expected {}",
                v.len(),
                depth
            );
        }

        data_per_ch.push(v);
    }

    Ok(data_per_ch)
}

fn write_csv(path: &str, channels: &[u8], data_per_ch: &[Vec<f64>], depth: usize) -> Result<()> {
    let file = File::create(path).context("failed to create csv file")?;
    let mut writer = BufWriter::new(file);

    for (i, ch) in channels.iter().enumerate() {
        if i + 1 == channels.len() {
            write!(writer, "ch{ch}")?;
        } else {
            write!(writer, "ch{ch},")?;
        }
    }
    writeln!(writer)?;

    for (row_idx, _) in data_per_ch[0].iter().enumerate().take(depth) {
        for (col_idx, ch_data) in data_per_ch.iter().enumerate() {
            write!(writer, "{}", ch_data[row_idx])?;
            if col_idx + 1 != data_per_ch.len() {
                write!(writer, ",")?;
            }
        }
        writeln!(writer)?;
    }

    writer.flush()?;
    Ok(())
}
