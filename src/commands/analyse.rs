use crate::commands::fetch::run_fetch;
use crate::communications::oscilloscope::OscilloscopeHandler;
use crate::config::Config;
use crate::constants::FETCHED_FNAME;
use crate::kerr::run_kerr_analysis;
use crate::lockin::time::time_builder;
use crate::phase::run_phase_analysis;
use crate::utils::channels::build_channel_list;
use crate::utils::csv::{ensure_not_exists, write_csv};
use crate::{commands::fetch::fetch_all_channels, lockin::run_li};
use anyhow::Result;

pub fn analyse(cfg: &Config) -> Result<()> {
    let data_per_ch = run_fetch(cfg)?;

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
