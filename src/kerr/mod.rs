pub mod kerr_harmonics_analysis;
pub mod kerr_standard_analysis;
pub mod save;

use crate::config::{Channel, KerrType};
use crate::constants::{KERR_NAME, LI_ROTATED_NAME};
use crate::kerr::kerr_harmonics_analysis::KerrHarmonicsAnalyser;
use crate::kerr::kerr_standard_analysis::KerrStandardAnalyser;
use crate::kerr::save::{get_kerr_headers, write_kerr_results};
use crate::{config::Config, utils::csv::read_csv};
use anyhow::{Context, Result};
use rayon::prelude::*;
use std::time::Instant;

pub fn run(cfg: &Config) -> Result<()> {
    let ch = &cfg.phase.use_signal_ch;

    if ch.is_empty() {
        println!("⚠️ No channels specified for phase analysis. Skipping phase analysis.");
        return Ok(());
    }

    let num_sensor_ch = cfg.roles.sensor_ch.len();

    let t0 = Instant::now();

    let all_data: Vec<Vec<Vec<f64>>> = ch
        .par_iter()
        .map(|channel| {
            let fname = format!("{}_ch{}.csv", LI_ROTATED_NAME, channel);
            read_csv(fname)
        })
        .collect::<Result<Vec<_>, _>>()?;

    let elapsed_read = t0.elapsed();
    println!(
        "📥 Read lock-in rotated results for ch {:?} in {:.2?}",
        ch, elapsed_read
    );

    let t = all_data[0][0].clone(); // time column
    let sensor_integral_ch = all_data[0][1..(1 + num_sensor_ch)].to_vec();

    let li_rotated_results: Vec<Vec<Vec<f64>>> = all_data
        .iter()
        .map(|read_data| read_data[1 + num_sensor_ch..].to_vec())
        .collect();

    run_kerr_analysis(cfg, &t, &sensor_integral_ch, &li_rotated_results)?;

    Ok(())
}

pub fn run_kerr_analysis(
    cfg: &Config,
    t: &[f64],
    sensor_integral_ch: &[Vec<f64>],
    li_rotated_results: &[Vec<Vec<f64>>],
) -> Result<()> {
    println!("🔍 Running Kerr analysis...");

    let kerr_sensor_ch_index = cfg.kerr.use_sensor_ch;

    let ch_conf: &Channel = cfg
        .channels
        .iter()
        .find(|ch| ch.index == kerr_sensor_ch_index)
        .unwrap();

    let label = ch_conf.label.as_ref().with_context(|| {
        format!(
            "Channel label is missing for channel {}",
            kerr_sensor_ch_index
        )
    })?;
    let unit = ch_conf.unit_out.as_ref().with_context(|| {
        format!(
            "Channel unit is missing for channel {}",
            kerr_sensor_ch_index
        )
    })?;
    let concat_label = format!("{} ({})", label, unit);

    let ch = &cfg.phase.use_signal_ch;

    let kerr_type = &cfg.kerr.kerr_type;

    let sensor_integral = &sensor_integral_ch[kerr_sensor_ch_index as usize - 1];
    let factor = cfg.kerr.factor;
    let mut kerr_results: Vec<Vec<f64>> = Vec::new();
    for (ch_i, li_rotated_result) in ch.iter().zip(li_rotated_results.iter()) {
        let fig_name = format!("{}_ch{}", KERR_NAME, ch_i);

        let kerr_i = match kerr_type {
            KerrType::Standard => KerrStandardAnalyser {}
                .analyse(
                    t,
                    sensor_integral,
                    li_rotated_result,
                    factor,
                    &concat_label,
                    fig_name,
                )
                .context("failed to run Kerr analysis")?,
            KerrType::Harmonics => KerrHarmonicsAnalyser {}
                .analyse(
                    t,
                    sensor_integral,
                    li_rotated_result,
                    factor,
                    &concat_label,
                    fig_name,
                )
                .context("failed to run Kerr harmonics analysis")?,
        };

        kerr_results.push(kerr_i);
    }
    let fname = format!("{}_results.csv", KERR_NAME);
    let headers = get_kerr_headers(cfg)?;
    write_kerr_results(&fname, &headers, t, sensor_integral_ch, &kerr_results)?;

    println!("💾 Saved Kerr analysis results for channels {:?}.", ch);
    println!("✅ Kerr analysis completed.");

    Ok(())
}
