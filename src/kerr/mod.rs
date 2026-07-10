pub mod kerr_harmonics_analysis;
pub mod kerr_standard_analysis;
pub mod save;

use crate::config::{Channel, KerrType};
use crate::constants::{KERR_NAME, LI_ROTATED_HEADER, LI_ROTATED_NAME};
use crate::kerr::kerr_harmonics_analysis::{KerrHarmonicsAnalyser, KerrHarmonicsAnalysisInput};
use crate::kerr::kerr_standard_analysis::{KerrStandardAnalyser, KerrStandardAnalysisInput};
use crate::kerr::save::{get_kerr_headers, write_kerr_results};
use crate::ui;
use crate::{config::Config, utils::csv::read_csv};
use anyhow::{Context, Result, bail};
use rayon::prelude::*;
use std::time::Instant;

pub fn run(cfg: &Config) -> Result<()> {
    let ch = cfg.phase_signal_ch();

    if ch.is_empty() {
        ui::skipped("Kerr analysis: no phase signal channels specified");
        return Ok(());
    }

    let num_sensor_ch = cfg.roles.sensor_ch.len();

    let t0 = Instant::now();
    let pb = ui::spinner(format!(
        "reading phase-rotated lock-in results for channels {:?}",
        ch
    ));

    let all_data: Vec<Vec<Vec<f64>>> = ch
        .par_iter()
        .map(|channel| {
            let fname = format!("{}_ch{}.csv", LI_ROTATED_NAME, channel);
            read_csv(cfg.artifact_path(fname))
        })
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

    let expected_columns = 1 + num_sensor_ch + num_sensor_ch + LI_ROTATED_HEADER.len();
    validate_result_column_count(&all_data, expected_columns, "phase-rotated lock-in results")?;

    let t = all_data[0][0].clone(); // time column
    let sensor_rate_ch = all_data[0][1..(1 + num_sensor_ch)].to_vec();
    let sensor_integral_ch = all_data[0][(1 + num_sensor_ch)..(1 + num_sensor_ch * 2)].to_vec();
    validate_context_columns_match(
        &all_data,
        1 + num_sensor_ch * 2,
        "phase-rotated lock-in results",
    )?;

    let li_start = 1 + num_sensor_ch * 2;
    let li_end = li_start + LI_ROTATED_HEADER.len();
    let li_rotated_results: Vec<Vec<Vec<f64>>> = all_data
        .iter()
        .map(|read_data| read_data[li_start..li_end].to_vec())
        .collect();

    run_kerr_analysis(
        cfg,
        &t,
        &sensor_rate_ch,
        &sensor_integral_ch,
        &li_rotated_results,
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
    let fname = format!("{}_results.csv", KERR_NAME);
    let path = cfg.artifact_path(&fname);
    let headers = get_kerr_headers(cfg)?;
    write_kerr_results(
        &path,
        &headers,
        t,
        sensor_rate_ch,
        sensor_integral_ch,
        &kerr_results,
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

fn validate_result_column_count(
    all_data: &[Vec<Vec<f64>>],
    expected_columns: usize,
    label: &str,
) -> Result<()> {
    for (index, read_data) in all_data.iter().enumerate() {
        if read_data.len() != expected_columns {
            bail!(
                "{label} file {index} has {} columns, expected {expected_columns}; old CSV layouts are not supported",
                read_data.len()
            );
        }
    }
    Ok(())
}

fn validate_context_columns_match(
    all_data: &[Vec<Vec<f64>>],
    context_columns: usize,
    label: &str,
) -> Result<()> {
    let Some(first) = all_data.first() else {
        return Ok(());
    };
    for (file_index, read_data) in all_data.iter().enumerate().skip(1) {
        for col_idx in 0..context_columns {
            if read_data[col_idx] != first[col_idx] {
                bail!("{label} file {file_index} context column {col_idx} differs from file 0");
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        kerr_sensor_position, validate_context_columns_match, validate_result_column_count,
    };
    use crate::test_support::test_config;

    #[test]
    fn kerr_rejects_old_rotated_result_layout_column_count() {
        let old_layout = vec![vec![vec![0.0]; 1 + 2 + 12]];

        let error = validate_result_column_count(
            &old_layout,
            1 + 2 + 2 + 12,
            "phase-rotated lock-in results",
        )
        .unwrap_err();

        assert!(
            error
                .to_string()
                .contains("old CSV layouts are not supported")
        );
    }

    #[test]
    fn kerr_rejects_mismatched_time_rate_or_integral_context() {
        let first = vec![vec![0.0], vec![1.0], vec![2.0], vec![3.0], vec![4.0]];
        let second = vec![vec![0.0], vec![1.0], vec![2.0], vec![3.5], vec![4.0]];

        let error =
            validate_context_columns_match(&[first, second], 4, "phase-rotated lock-in results")
                .unwrap_err();

        assert!(error.to_string().contains("context column 3"));
    }

    #[test]
    fn kerr_sensor_position_uses_configured_sensor_channel_order() {
        let mut cfg = test_config(vec![2, 4], vec![3]);

        cfg.kerr.use_sensor_ch = 4;
        assert_eq!(kerr_sensor_position(&cfg).unwrap(), 1);

        cfg.kerr.use_sensor_ch = 1;
        assert!(kerr_sensor_position(&cfg).is_err());
    }
}
