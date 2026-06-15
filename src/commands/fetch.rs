use crate::config::Config;
use crate::constants::FETCHED_FNAME;
use crate::ui;
use crate::utils::channels::build_channel_list;
use crate::utils::csv::write_csv;
use crate::{communications::oscilloscope::OscilloscopeHandler, utils::csv::ensure_not_exists};
use anyhow::{Context, Result, anyhow, bail};
use std::time::Instant;

pub fn fetch(cfg: &Config) -> Result<()> {
    let data = run_fetch(cfg)?;

    let channels = build_channel_list(cfg)?;
    let headers: Vec<String> = channels.iter().map(|ch| format!("ch{ch}")).collect();
    let header_refs: Vec<&str> = headers.iter().map(|s| s.as_str()).collect();

    let t_write_start = Instant::now();
    write_csv(FETCHED_FNAME, &header_refs, &data)?;
    let t_write_end = Instant::now();

    ui::saved(format!(
        "{FETCHED_FNAME} ({})",
        ui::fmt_duration(t_write_end - t_write_start)
    ));
    Ok(())
}

pub fn run_fetch(cfg: &Config) -> Result<Vec<Vec<f64>>> {
    ensure_not_exists(FETCHED_FNAME)?;

    let mut handler = OscilloscopeHandler::initialize(cfg)
        .context("failed to initialize oscilloscope handler")?;

    let osc_cfg = &cfg
        .instruments
        .as_ref()
        .ok_or_else(|| anyhow!("Instruments configuration is missing."))?
        .oscilloscope;

    let depth = osc_cfg.memory_depth;

    let channels = build_channel_list(cfg)?;
    let pb = ui::progress(
        format!("fetching {depth} samples from {} channels", channels.len()),
        channels.len() as u64,
    );

    let t_fetch_start = Instant::now();
    let data = fetch_all_channels(&mut handler, &channels, depth, &pb)?;
    let t_fetch_end = Instant::now();

    let fetch_elapsed = t_fetch_end - t_fetch_start;

    ui::finish_success(
        pb,
        format!(
            "fetched {} samples from {} channels ({}, {:.2} samples/sec)",
            depth,
            channels.len(),
            ui::fmt_duration(fetch_elapsed),
            (depth * channels.len()) as f64 / fetch_elapsed.as_secs_f64()
        ),
    );
    Ok(data)
}

pub fn fetch_all_channels(
    handler: &mut OscilloscopeHandler,
    channels: &[u8],
    depth: usize,
    progress: &indicatif::ProgressBar,
) -> Result<Vec<Vec<f64>>> {
    let mut data: Vec<Vec<f64>> = Vec::with_capacity(channels.len());

    for &ch in channels {
        progress.set_message(format!("fetching ch{ch}"));
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

        data.push(v);
        progress.inc(1);
    }

    Ok(data)
}
