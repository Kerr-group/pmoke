pub mod pulse_calculator;
pub mod sensor_builder;
pub mod sensor_integral_plot;
pub mod sensor_raw_plot;

use crate::config::Config;
use crate::constants::FETCHED_FNAME;
use crate::lockin::time::time_builder;
use crate::utils::channels::build_channel_list;
use crate::utils::csv::read_selected_columns;
use anyhow::{Context, Result, anyhow, bail};

pub fn run(cfg: &Config) -> Result<()> {
    let s_chs: &Vec<u8> = extract_sensor_ch(cfg)?;

    let channels = build_channel_list(cfg)?;

    let mut col_idx = Vec::new();

    for ch in s_chs.iter() {
        let idx = channels.iter().position(|c| *c == *ch).ok_or_else(|| {
            anyhow!(
                "sensor channel {} not found in fetched channels {:?}",
                ch,
                channels
            )
        })?;
        col_idx.push(idx);
    }

    let mut s_cfg = Vec::new();
    for idx in col_idx.iter() {
        s_cfg.push(&cfg.channels[*idx]);
    }

    let t0 = std::time::Instant::now();
    let s_cols = read_selected_columns(FETCHED_FNAME, &col_idx)
        .context("failed to read sensor columns from csv")?;
    let elapsed_read = t0.elapsed();
    println!(
        "📥 Read seosor columns {:?} in {:.2?}",
        col_idx, elapsed_read
    );

    let t = &time_builder(cfg)?;

    if t.len() != s_cols[0].len() {
        bail!(
            "time length ({}) and sensor length ({}) differ",
            t.len(),
            s_cols[0].len()
        );
    }

    let index_arr = s_cfg.iter().map(|c| c.index).collect::<Vec<u8>>();
    let factor_arr = s_cfg
        .iter()
        .map(|c| c.factor.unwrap())
        .collect::<Vec<f64>>();
    let label_arr = s_cfg
        .iter()
        .map(|c| c.label.as_ref().unwrap().as_str())
        .collect::<Vec<&str>>();
    let unit_arr = s_cfg
        .iter()
        .map(|c| c.unit_out.as_ref().unwrap().as_str())
        .collect::<Vec<&str>>();

    let stride_samples = cfg.lockin.stride_samples;

    let t_stride = t
        .iter()
        .step_by(stride_samples)
        .cloned()
        .collect::<Vec<f64>>();
    let s_stride = s_cols
        .iter()
        .map(|col| {
            col.iter()
                .step_by(stride_samples)
                .cloned()
                .collect::<Vec<f64>>()
        })
        .collect::<Vec<Vec<f64>>>();

    let bg_window_before = cfg.pulse.bg_window_before;
    let bg_window_after = cfg.pulse.bg_window_after;

    let t_fit = t
        .iter()
        .cloned()
        .filter(|&ti| {
            (ti >= bg_window_before.start && ti <= bg_window_before.end)
                || (ti >= bg_window_after.start && ti <= bg_window_after.end)
        })
        .collect::<Vec<f64>>();
    let s_fit = s_cols
        .iter()
        .map(|col| {
            col.iter()
                .cloned()
                .zip(t.iter())
                .filter(|&(_yi, ti)| {
                    (ti >= &bg_window_before.start && ti <= &bg_window_before.end)
                        || (ti >= &bg_window_after.start && ti <= &bg_window_after.end)
                })
                .map(|(yi, _ti)| yi)
                .collect::<Vec<f64>>()
        })
        .collect::<Vec<Vec<f64>>>();

    let mut c_bg_arr = Vec::new();
    for s in s_fit.iter() {
        let c = pulse_calculator::PulseBgFitter {}.fit(&t_fit, s)?;
        c_bg_arr.push(c);
    }
    // drop t_fit and s_fit to save memory
    drop(t_fit);
    drop(s_fit);

    sensor_raw_plot::SensorRawPlotter {}
        .plot(&t_stride, s_stride, &index_arr, &c_bg_arr)
        .context("failed to plot sensor data")?;

    // Do integration on sensors
    let start = std::time::Instant::now();
    let mut s_integral = Vec::new();
    for (i, s) in s_cols.iter().enumerate() {
        let c_bg = c_bg_arr[i];
        let coeff = factor_arr[i];
        let integral = pulse_calculator::PulseIntegralCalculator::new(cfg.timebase.dt)
            .integrate(s, c_bg, coeff);
        s_integral.push(integral);
    }
    let elapsed = start.elapsed();
    println!("💻 Sensor integrations completed in {:.2?}", elapsed);

    let s_integral_stride = s_integral
        .iter()
        .map(|col| {
            col.iter()
                .step_by(stride_samples)
                .cloned()
                .collect::<Vec<f64>>()
        })
        .collect::<Vec<Vec<f64>>>();

    sensor_integral_plot::SensorIntegralPlotter {}
        .plot(
            t_stride,
            s_integral_stride,
            &index_arr,
            &label_arr,
            &unit_arr,
        )
        .context("failed to plot sensor integrals")?;

    Ok(())
}

fn extract_sensor_ch(cfg: &Config) -> Result<&Vec<u8>> {
    let s_chs = &cfg.roles.sensor_ch;
    if s_chs.is_empty() {
        bail!("reference channel is not specified in the configuration");
    }
    Ok(s_chs)
}
