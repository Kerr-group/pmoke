pub mod moke_harmonics_analysis;
pub mod moke_standard_analysis;
pub mod save;

use crate::analysis_results::parse_analysis_result_files;
use crate::config::{Channel, MokeType};
use crate::constants::{LI_ROTATED_HEADER, MOKE_NAME};
use crate::moke::moke_harmonics_analysis::{MokeHarmonicsAnalyser, MokeHarmonicsAnalysisInput};
use crate::moke::moke_standard_analysis::{MokeStandardAnalyser, MokeStandardAnalysisInput};
use crate::moke::save::{get_moke_headers, write_moke_results};
use crate::phase::uncertainty::{conditional_moke_variance, read_rotated_covariance_csv};
use crate::ui;
use crate::{config::Config, utils::csv::read_csv};
use anyhow::{Context, Result};
use rayon::prelude::*;
use std::time::Instant;

pub struct MokeChannelOutput {
    pub angle: Vec<f64>,
    pub vm: Vec<f64>,
}

pub fn run(cfg: &Config) -> Result<()> {
    let ch = cfg.phase_signal_ch();

    if ch.is_empty() {
        ui::skipped("Moke analysis: no phase signal channels specified");
        return Ok(());
    }

    let t0 = Instant::now();
    let pb = ui::spinner(format!(
        "reading phase-rotated lock-in results for channels {:?}",
        ch
    ));

    let resolver = cfg.resolver();
    let all_data: Vec<Vec<Vec<f64>>> = ch
        .par_iter()
        .map(|channel| read_csv(resolver.lockin_rotated_csv(*channel)))
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

    run_moke_analysis(
        cfg,
        &data.time,
        &data.sensor_rate,
        &data.sensor_integral,
        &data.results,
    )?;

    Ok(())
}

pub fn run_moke_analysis(
    cfg: &Config,
    t: &[f64],
    sensor_rate_ch: &[Vec<f64>],
    sensor_integral_ch: &[Vec<f64>],
    li_rotated_results: &[Vec<Vec<f64>>],
) -> Result<()> {
    let paths = cfg.paths();
    let moke_sensor_ch_index = cfg.moke.use_sensor_ch;

    let ch_conf: &Channel = cfg
        .channels
        .iter()
        .find(|ch| ch.index == moke_sensor_ch_index)
        .with_context(|| {
            format!("Moke sensor channel {moke_sensor_ch_index} is missing from channels")
        })?;

    let label = ch_conf.label.as_ref().with_context(|| {
        format!(
            "Channel label is missing for channel {}",
            moke_sensor_ch_index
        )
    })?;
    let unit = ch_conf.unit_out.as_ref().with_context(|| {
        format!(
            "Channel unit is missing for channel {}",
            moke_sensor_ch_index
        )
    })?;
    let concat_label = format!("{} ({})", label, unit);

    let ch = cfg.phase_signal_ch();

    let moke_type = &cfg.moke.moke_type;

    let moke_sensor_pos = moke_sensor_position(cfg)?;
    let sensor_integral = sensor_integral_ch.get(moke_sensor_pos).with_context(|| {
        format!("sensor integral column for ch{moke_sensor_ch_index} is missing")
    })?;
    let factor = cfg.moke.factor;
    let mut angle_results: Vec<Vec<f64>> = Vec::new();
    let mut vm_results: Vec<Vec<f64>> = Vec::new();
    // R2c: conditional MOKE uncertainty is Standard-only and GLS-only. The
    // rotated covariance shares the rotated XY grid; when it is absent
    // (boxcar / none-policy / missing) the per-channel variance stays empty
    // and no zeros are invented. Harmonics angles have no conditional
    // variance yet.
    let gls_standard = matches!(
        cfg.lockin.estimator,
        crate::config::LockinEstimator::JointHarmonicGls(_)
    ) && matches!(moke_type, MokeType::Standard);
    let mut angle_variance_results: Vec<Vec<f64>> = Vec::new();
    let pb = ui::progress("running Moke analysis", ch.len() as u64);
    let t0 = Instant::now();
    for (ch_i, li_rotated_result) in ch.iter().zip(li_rotated_results.iter()) {
        pb.set_message(format!("Moke analysis ch{ch_i}"));
        let fig_name = format!("{}_ch{}", MOKE_NAME, ch_i);
        let output_path = if ch.len() == 1 {
            paths.moke_plot()
        } else {
            paths.moke_channel_plot(*ch_i)
        };
        let vm_output_path = if ch.len() == 1 {
            paths.moke_vm_plot()
        } else {
            paths.moke_vm_channel_plot(*ch_i)
        };

        let channel_output = match moke_type {
            MokeType::Standard => MokeStandardAnalyser {}
                .analyse(MokeStandardAnalysisInput {
                    plot: &cfg.plot,
                    t,
                    x: sensor_integral,
                    ys: li_rotated_result,
                    factor,
                    xlabel: &concat_label,
                    fig_name,
                    output_path: &output_path,
                    vm_output_path: &vm_output_path,
                })
                .context("failed to run Moke analysis")?,
            MokeType::Harmonics => MokeHarmonicsAnalyser {}
                .analyse(MokeHarmonicsAnalysisInput {
                    plot: &cfg.plot,
                    t,
                    x: sensor_integral,
                    ys: li_rotated_result,
                    factor,
                    xlabel: &concat_label,
                    fig_name,
                    output_path: &output_path,
                    vm_output_path: &vm_output_path,
                })
                .context("failed to run Moke harmonics analysis")?,
        };

        angle_results.push(channel_output.angle);
        vm_results.push(channel_output.vm);
        // R2c: per-window delta-method variance from the rotated (x1, x2)
        // marginal (row-major indices 0, 2, 2*12+2). Malformed grids fail
        // closed; absence yields an empty column, never zeros.
        if gls_standard {
            let rotated_path = paths.lockin_rotated_covariance_csv(*ch_i);
            let variance = match read_rotated_covariance_csv(&rotated_path)? {
                None => Vec::new(),
                Some((cov_times, matrices)) => {
                    if cov_times.len() != li_rotated_result[0].len() {
                        anyhow::bail!(
                            "rotated covariance rows ({}) differ from rotated XY rows ({}) for ch{ch_i}",
                            cov_times.len(),
                            li_rotated_result[0].len()
                        );
                    }
                    let x1 = &li_rotated_result[0];
                    let x2 = &li_rotated_result[2];
                    matrices
                        .iter()
                        .enumerate()
                        .map(|(row, matrix)| {
                            conditional_moke_variance(
                                x1[row],
                                x2[row],
                                matrix[0],
                                matrix[2],
                                matrix[2 * 12 + 2],
                            )
                            .with_context(|| {
                                format!("conditional MOKE variance failed at row {row} ch{ch_i}")
                            })
                        })
                        .collect::<Result<Vec<_>>>()?
                }
            };
            angle_variance_results.push(variance);
        }
        pb.inc(1);
    }
    let path = paths.moke_csv();
    let headers = get_moke_headers(cfg)?;
    write_moke_results(
        &path,
        &headers,
        t,
        sensor_rate_ch,
        sensor_integral_ch,
        &angle_results,
        &vm_results,
        cfg.lockin.save_npy,
    )?;
    // R2c: conditional variance lives beside moke.csv in one combined CSV
    // (single column set across channels); empty columns when unavailable.
    if gls_standard {
        write_moke_variance_csv(&paths.moke_variance_csv(), ch, &angle_variance_results)?;
    }

    ui::finish_saved(
        pb,
        format!(
            "Moke analysis results for channels {:?} ({})",
            ch,
            ui::fmt_duration(t0.elapsed())
        ),
    );
    ui::success("Moke analysis completed");

    Ok(())
}

/// Writes the conditional MOKE variance artifact
/// (`moke/moke_variance.csv`, R2c): one row per MOKE output window, one
/// `Ch{N} angle_variance (rad^2)` column per channel. Empty columns mark
/// explicit unavailability (boxcar / none-policy / missing covariance);
/// non-finite variances fail closed instead of publishing zeros.
fn write_moke_variance_csv(
    path: &std::path::Path,
    channels: &[u8],
    variances: &[Vec<f64>],
) -> Result<()> {
    use crate::utils::csv::write_csv;
    if channels.len() != variances.len() {
        anyhow::bail!(
            "moke variance channels ({}) and columns ({}) differ",
            channels.len(),
            variances.len()
        );
    }
    let rows = variances.first().map_or(0, Vec::len);
    if variances.iter().any(|column| column.len() != rows) {
        anyhow::bail!("moke variance columns have unequal lengths");
    }
    if variances.iter().flatten().any(|value| !value.is_finite()) {
        anyhow::bail!("moke variance has non-finite entries");
    }
    let headers: Vec<String> = channels
        .iter()
        .map(|channel| format!("Ch{channel} angle_variance (rad^2)"))
        .collect();
    let header_refs: Vec<&str> = headers.iter().map(String::as_str).collect();
    let columns: Vec<&[f64]> = variances.iter().map(Vec::as_slice).collect();
    write_csv(path, &header_refs, &columns)?;
    Ok(())
}

fn moke_sensor_position(cfg: &Config) -> Result<usize> {
    let moke_sensor_ch_index = cfg.moke.use_sensor_ch;
    cfg.roles
        .sensor_ch
        .iter()
        .position(|&ch| ch == moke_sensor_ch_index)
        .with_context(|| {
            format!("moke.use_sensor_ch {moke_sensor_ch_index} is not in roles.sensor_ch")
        })
}

#[cfg(test)]
mod tests {
    use super::moke_sensor_position;
    use crate::test_support::test_config;

    #[test]
    fn moke_sensor_position_uses_configured_sensor_channel_order() {
        let mut cfg = test_config(vec![2, 4], vec![3]);

        cfg.moke.use_sensor_ch = 4;
        assert_eq!(moke_sensor_position(&cfg).unwrap(), 1);

        cfg.moke.use_sensor_ch = 1;
        assert!(moke_sensor_position(&cfg).is_err());
    }
}
