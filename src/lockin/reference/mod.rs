pub mod fit;
pub mod ref_plot;

use std::f64::consts::PI;

use crate::constants::FETCHED_FNAME;
use crate::lockin::time::time_builder;
use crate::utils::channels::build_channel_list;
use crate::utils::csv::read_selected_columns;
use crate::{
    config::Config,
    lockin::reference::fit::{RefFitParams, ReferenceHandler},
};
use anyhow::{Context, Result, anyhow, bail};

pub fn run(cfg: &Config) -> Result<RefFitParams> {
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
    let ref_cols = read_selected_columns(FETCHED_FNAME, &[col_idx])
        .context("failed to read reference column from csv")?;
    let elapsed_read = t0.elapsed();
    println!(
        "📥 read reference column {} in {:.2?}",
        col_idx + 1,
        elapsed_read
    );

    let ref_data = &ref_cols[0];

    let t = &time_builder(cfg)?;

    if t.len() != ref_data.len() {
        bail!(
            "time length ({}) and reference length ({}) differ",
            t.len(),
            ref_data.len()
        );
    }

    let results = ReferenceHandler {}
        .fit(t, ref_data)
        .context("failed to fit reference signal")?;

    let f = results.f_ref;
    let a = results.a_ref;
    let omegat = results.omega_tref;

    let t_period = 1.0 / f;
    let t_start_plot = 0.0;
    let t_end_plot = 3.0 * t_period;

    let idx_start = t.iter().position(|&ti| ti >= t_start_plot).unwrap_or(0);
    let idx_end = t
        .iter()
        .position(|&ti| ti > t_end_plot)
        .unwrap_or(t.len() - 1);

    let t_plot: Vec<f64> = t[idx_start..=idx_end].to_vec();
    let ref_plot: Vec<f64> = ref_data[idx_start..=idx_end].to_vec();

    let fit_plot: Vec<f64> = t_plot
        .iter()
        .map(|&ti| a * (2.0 * PI * f * ti - omegat).sin())
        .collect();

    ref_plot::ReferencePlotter {}
        .plot(&t_plot, &ref_plot, &fit_plot)
        .context("failed to plot reference signal")?;

    Ok(results)
}

fn extract_single_reference_ch(cfg: &Config) -> Result<u8> {
    let ref_chs = &cfg.roles.reference_ch;
    if ref_chs.is_empty() {
        bail!("reference channel is not specified in the configuration");
    }
    if ref_chs.len() != 1 {
        bail!("multiple reference channels are not supported");
    }
    Ok(ref_chs[0] as u8)
}
