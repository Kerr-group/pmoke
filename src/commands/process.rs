use std::time::Instant;

use crate::commands::analyze::run_analyze;
use crate::commands::fetch::run_fetch;
use crate::config::Config;
use crate::constants::FETCHED_FNAME;
use crate::lockin::time::time_builder;
use crate::utils::channels::build_channel_list;
use crate::utils::csv::write_csv;
use anyhow::Result;

pub fn process(cfg: &Config) -> Result<()> {
    let data = run_fetch(cfg)?;

    let channels = build_channel_list(cfg)?;
    let headers: Vec<String> = channels.iter().map(|ch| format!("ch{ch}")).collect();
    let header_refs: Vec<&str> = headers.iter().map(|s| s.as_str()).collect();

    let t_write_start = Instant::now();
    write_csv(FETCHED_FNAME, &header_refs, &data)?;
    let t_write_end = Instant::now();

    println!(
        "📝 Data written to raw.csv in {:.2?}",
        t_write_end - t_write_start
    );

    let t = time_builder(cfg)?;

    run_analyze(cfg, t, data)?;

    Ok(())
}
