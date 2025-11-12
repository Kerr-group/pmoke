pub mod fit;
pub mod ref_plot;

use std::f64::consts::PI;

use crate::config::Config;
use crate::constants::FETCHED_FNAME;
use crate::lockin::reference::fit::{RefFitParams, ReferenceFitter};
use crate::lockin::time::time_builder;
use crate::utils::channels::build_channel_list;
use crate::utils::csv::read_selected_columns;
use anyhow::{Context, Result, anyhow, bail};

pub fn run(cfg: &Config) -> Result<()> {
    let t = time_builder(cfg)?;
    let _ = get_ref_fit_params(cfg, &t)?;
    Ok(())
}

pub fn get_ref_fit_params(cfg: &Config, t: &[f64]) -> Result<RefFitParams> {
    let ref_ch = extract_single_reference_ch(cfg)?;

    let channels = build_channel_list(cfg)?;
    let col_idx = channels
        .iter()
        .position(|ch| *ch == ref_ch)
        .ok_or_else(|| {
            anyhow!(
                "reference channel {} not found in fetched channels {:?}",
                ref_ch,
                channels
            )
        })?;

    let t0 = std::time::Instant::now();
    let ref_data = read_selected_columns(FETCHED_FNAME, &[col_idx])
        .context("failed to read reference column from csv")?
        .pop()
        .ok_or_else(|| {
            anyhow!(
                "read_selected_columns returned no data for column index {}",
                col_idx
            )
        })?;
    let elapsed_read = t0.elapsed();
    println!(
        "📥 Read reference column {} in {:.2?}",
        col_idx + 1,
        elapsed_read
    );

    let results = ReferenceFitter {}
        .fit(t, &ref_data)
        .context("failed to fit reference signal")?;

    Ok(results)
}

pub fn run_reference(t: &[f64], ref_data: &[f64]) -> Result<RefFitParams> {
    if t.len() != ref_data.len() {
        bail!(
            "time length ({}) and reference length ({}) differ",
            t.len(),
            ref_data.len()
        );
    }

    if t.is_empty() {
        println!("(Info) Time and reference data are empty. Skipping fit and plot.");
        bail!("Cannot fit empty data.");
    }

    let results = ReferenceFitter {}
        .fit(t, ref_data)
        .context("failed to fit reference signal")?;

    plot_fit_results(t, ref_data, &results).context("failed to plot reference signal")?;

    Ok(results)
}

fn plot_fit_results(t: &[f64], ref_data: &[f64], results: &RefFitParams) -> Result<()> {
    if t.is_empty() {
        println!("(Info) No data to plot.");
        return Ok(());
    }

    let f = results.f_ref;
    let a = results.a_ref;
    let omegat = results.omega_tref;

    if f == 0.0 {
        bail!("Reference frequency is zero, cannot plot results.");
    }

    let t_period = 1.0 / f;
    let t_start_plot = 0.0;
    let t_end_plot = 3.0 * t_period;

    let idx_start = t.iter().position(|&ti| ti >= t_start_plot).unwrap_or(0);

    let idx_end = t.iter().position(|&ti| ti > t_end_plot).unwrap_or(t.len()); // <--- t.len() - 1 から変更

    if idx_start >= idx_end {
        println!("(Info) No data in the specified plot time range.");
        return Ok(());
    }

    let t_plot = &t[idx_start..idx_end];
    let ref_plot = &ref_data[idx_start..idx_end];
    // -------------------------

    let fit_plot: Vec<f64> = t_plot
        .iter()
        .map(|&ti| a * (2.0 * PI * f * ti - omegat).sin())
        .collect();

    ref_plot::ReferencePlotter {}
        .plot(t_plot, ref_plot, &fit_plot)
        .context("failed to plot reference signal")?;

    Ok(())
}

fn extract_single_reference_ch(cfg: &Config) -> Result<u8> {
    match cfg.roles.reference_ch.len() {
        0 => bail!("reference channel is not specified in the configuration"),
        1 => Ok(cfg.roles.reference_ch[0]),
        _ => bail!("multiple reference channels are not supported"),
    }
}
