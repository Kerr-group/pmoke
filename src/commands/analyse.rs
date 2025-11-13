use crate::communications::oscilloscope::OscilloscopeHandler;
use crate::config::Config;
use crate::constants::FETCHED_FNAME;
use crate::kerr::run_kerr_analysis;
use crate::lockin::time::time_builder;
use crate::phase::run_phase_analysis;
use crate::utils::channels::build_channel_list;
use crate::utils::csv::write_csv;
use crate::{commands::fetch::fetch_all_channels, lockin::run_li};
use anyhow::{Context, Result, anyhow};
use std::time::Instant;

pub fn analyse(cfg: &Config) -> Result<()> {
    let mut handler = OscilloscopeHandler::initialize(cfg)
        .context("failed to initialize oscilloscope handler")?;

    let osc_cfg = &cfg
        .instruments
        .as_ref()
        .ok_or_else(|| anyhow!("Instruments configuration is missing."))?
        .oscilloscope;

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

    let headers: Vec<String> = channels.iter().map(|ch| format!("ch{ch}")).collect();
    let header_refs: Vec<&str> = headers.iter().map(|s| s.as_str()).collect();

    let t_write_start = Instant::now();
    write_csv(FETCHED_FNAME, &header_refs, &data_per_ch)?;
    let t_write_end = Instant::now();

    println!(
        "📝 Data written to raw.csv in {:.2?}",
        t_write_end - t_write_start
    );

    let t = time_builder(cfg)?;

    // run lock-in analysis here
    let (t_stride, sensor_integral_stride, li_results) = run_li(cfg, &t, &data_per_ch)?;
    drop(t);

    // run phase analysis here
    let ch = &cfg.phase.use_signal_ch;

    if ch.is_empty() {
        println!("⚠️ No channels specified for phase analysis. Skipping phase analysis.");
        return Ok(());
    }
    let li_rotated_results =
        run_phase_analysis(cfg, &t_stride, &sensor_integral_stride, &li_results)?;
    drop(li_results);

    // run kerr analysis here
    let _ = run_kerr_analysis(cfg, &t_stride, &sensor_integral_stride, &li_rotated_results)?;

    Ok(())
}
