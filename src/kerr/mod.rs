pub mod kerr_harmonics_analysis;
pub mod kerr_standard_analysis;
pub mod save;

use crate::analysis_results::parse_analysis_result_files;
use crate::config::{Channel, KerrType};
use crate::constants::{KERR_NAME, LI_ROTATED_HEADER};
use crate::kerr::kerr_harmonics_analysis::{KerrHarmonicsAnalyser, KerrHarmonicsAnalysisInput};
use crate::kerr::kerr_standard_analysis::{KerrStandardAnalyser, KerrStandardAnalysisInput};
use crate::kerr::save::{get_kerr_headers, write_kerr_results};
use crate::ui;
use crate::{config::Config, utils::csv::read_csv};
use anyhow::{Context, Result};
use rayon::prelude::*;
use std::time::Instant;

pub fn run(cfg: &Config) -> Result<()> {
    let ch = cfg.phase_signal_ch();

    if ch.is_empty() {
        ui::skipped("Kerr analysis: no phase signal channels specified");
        return Ok(());
    }

    let t0 = Instant::now();
    let pb = ui::spinner(format!(
        "reading phase-rotated lock-in results for channels {:?}",
        ch
    ));

    let paths = cfg.paths();
    let all_data: Vec<Vec<Vec<f64>>> = ch
        .par_iter()
        .map(|channel| read_csv(paths.lockin_rotated_csv(*channel)))
        .collect::<Result<Vec<_>, _>>()?;

    let elapsed_read = t0.elapsed();
    ui::finish_read(
        pb,
        format!(
            "phase-rotated lock-in results for channels {:?} ({})",
            ch,
            ui::fmt_duration(elapsed_read)
        ),
    );

    let data = parse_analysis_result_files(
        &all_data,
        cfg.roles.sensor_ch.len(),
        LI_ROTATED_HEADER.len(),
        "phase-rotated lock-in results",
    )?;

    run_kerr_analysis(
        cfg,
        &data.time,
        &data.sensor_rate,
        &data.sensor_integral,
        &data.results,
    )?;

    Ok(())
}

pub fn run_kerr_analysis(
    cfg: &Config,
    t: &[f64],
    sensor_rate_ch: &[Vec<f64>],
    sensor_integral_ch: &[Vec<f64>],
    li_rotated_results: &[Vec<Vec<f64>>],
) -> Result<()> {
    let paths = cfg.paths();
    let kerr_sensor_ch_index = cfg.kerr.use_sensor_ch;

    let ch_conf: &Channel = cfg
        .channels
        .iter()
        .find(|ch| ch.index == kerr_sensor_ch_index)
        .with_context(|| {
            format!("Kerr sensor channel {kerr_sensor_ch_index} is missing from channels")
        })?;

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

    let ch = cfg.phase_signal_ch();

    let kerr_type = &cfg.kerr.kerr_type;

    let kerr_sensor_pos = kerr_sensor_position(cfg)?;
    let sensor_integral = sensor_integral_ch.get(kerr_sensor_pos).with_context(|| {
        format!("sensor integral column for ch{kerr_sensor_ch_index} is missing")
    })?;
    let factor = cfg.kerr.factor;
    let mut kerr_results: Vec<Vec<f64>> = Vec::new();
    let pb = ui::progress("running Kerr analysis", ch.len() as u64);
    for (ch_i, li_rotated_result) in ch.iter().zip(li_rotated_results.iter()) {
        pb.set_message(format!("Kerr analysis ch{ch_i}"));
        let fig_name = format!("{}_ch{}", KERR_NAME, ch_i);

        let kerr_i = match kerr_type {
            KerrType::Standard => KerrStandardAnalyser {}
                .analyse(KerrStandardAnalysisInput {
                    plot: &cfg.plot,
                    t,
                    x: sensor_integral,
                    ys: li_rotated_result,
                    factor,
                    xlabel: &concat_label,
                    fig_name,
                })
                .context("failed to run Kerr analysis")?,
            KerrType::Harmonics => KerrHarmonicsAnalyser {}
                .analyse(KerrHarmonicsAnalysisInput {
                    plot: &cfg.plot,
                    t,
                    x: sensor_integral,
                    ys: li_rotated_result,
                    factor,
                    xlabel: &concat_label,
                    fig_name,
                })
                .context("failed to run Kerr harmonics analysis")?,
        };

        kerr_results.push(kerr_i);
        pb.inc(1);
    }
    let path = paths.kerr_csv();
    let headers = get_kerr_headers(cfg)?;
    write_kerr_results(
        &path,
        &headers,
        t,
        sensor_rate_ch,
        sensor_integral_ch,
        &kerr_results,
        cfg.lockin.save_npy,
    )?;

    ui::finish_saved(pb, format!("Kerr analysis results for channels {:?}", ch));
    ui::success("Kerr analysis completed");

    Ok(())
}

fn kerr_sensor_position(cfg: &Config) -> Result<usize> {
    let kerr_sensor_ch_index = cfg.kerr.use_sensor_ch;
    cfg.roles
        .sensor_ch
        .iter()
        .position(|&ch| ch == kerr_sensor_ch_index)
        .with_context(|| {
            format!("kerr.use_sensor_ch {kerr_sensor_ch_index} is not in roles.sensor_ch")
        })
}

#[cfg(test)]
mod tests {
    use super::kerr_sensor_position;
    use crate::test_support::test_config;

    #[test]
    fn kerr_sensor_position_uses_configured_sensor_channel_order() {
        let mut cfg = test_config(vec![2, 4], vec![3]);

        cfg.kerr.use_sensor_ch = 4;
        assert_eq!(kerr_sensor_position(&cfg).unwrap(), 1);

        cfg.kerr.use_sensor_ch = 1;
        assert!(kerr_sensor_position(&cfg).is_err());
    }
}
