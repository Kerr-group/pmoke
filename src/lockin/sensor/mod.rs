pub mod pulse_calculator;
pub mod sensor_integral_plot;
pub mod sensor_raw_plot;

use crate::config::Config;
use crate::constants::FETCHED_FNAME;
use crate::lockin::reference::get_ref_fit_params;
use crate::lockin::resolve::sensor_column_indices;
use crate::lockin::stride::{li_stride_1d, li_stride_2d};
use crate::lockin::time::time_builder;
use crate::utils::csv::read_selected_columns;
use anyhow::{Context, Result, bail};

pub struct SensorMeta<'a> {
    pub factor: f64,
    pub label: &'a str,
    pub unit: &'a str,
}

pub fn run(cfg: &Config) -> Result<()> {
    let t = time_builder(cfg)?;
    let ref_fit_params = get_ref_fit_params(cfg, &t)?;

    let (sensor_ch, col_idx) = sensor_column_indices(cfg)?;
    let t0 = std::time::Instant::now();
    let s_cols = read_selected_columns(FETCHED_FNAME, &col_idx)
        .context("failed to read sensor columns from csv")?;
    println!(
        "📥 Read sensor columns {:?} in {:.2?}",
        col_idx,
        t0.elapsed()
    );

    let _ = run_sensor(cfg, &t, &s_cols, &sensor_ch, ref_fit_params.f_ref)?;
    Ok(())
}

pub fn run_sensor(
    cfg: &Config,
    t: &[f64],
    s_cols: &[Vec<f64>],
    sensor_ch: &[u8],
    f_ref: f64,
) -> Result<(Vec<f64>, Vec<Vec<f64>>)> {
    if s_cols.is_empty() {
        bail!("No sensor data columns were read from {}", FETCHED_FNAME);
    }

    if t.len() != s_cols[0].len() {
        bail!(
            "time length ({}) and sensor length ({}) differ",
            t.len(),
            s_cols[0].len()
        );
    }

    let sensor_meta = extract_sensor_metadata(cfg, sensor_ch)?;

    let c_bg_arr = fit_background(cfg, t, s_cols)?;

    let t_stride = li_stride_1d(cfg, t, f_ref)?;
    let s_stride = li_stride_2d(cfg, s_cols, f_ref)?;

    sensor_raw_plot::SensorRawPlotter {}
        .plot(&t_stride, s_stride, sensor_ch, &c_bg_arr)
        .context("failed to plot sensor data")?;

    let start = std::time::Instant::now();

    let dt = cfg.timebase.dt;
    let s_integral = s_cols
        .iter()
        .zip(c_bg_arr.iter())
        .zip(sensor_meta.iter())
        .map(|((s, &c_bg), meta)| {
            pulse_calculator::PulseIntegralCalculator::new(dt).integrate(s, c_bg, meta.factor)
        })
        .collect::<Vec<_>>();

    let elapsed = start.elapsed();
    println!("💻 Sensor integrations completed in {:.2?}", elapsed);

    let s_integral_stride = li_stride_2d(cfg, &s_integral, f_ref)?;

    let labels: Vec<&str> = sensor_meta.iter().map(|m| m.label).collect();
    let units: Vec<&str> = sensor_meta.iter().map(|m| m.unit).collect();

    sensor_integral_plot::SensorIntegralPlotter {}
        .plot(&t_stride, &s_integral_stride, sensor_ch, &labels, &units)
        .context("failed to plot sensor integrals")?;

    Ok((t_stride, s_integral_stride))
}

pub fn extract_sensor_metadata<'a>(
    cfg: &'a Config,
    sensor_ch: &[u8],
) -> Result<Vec<SensorMeta<'a>>> {
    sensor_ch
        .iter()
        .map(|ch| {
            let conf = cfg
                .channels
                .iter()
                .find(|c| c.index == *ch)
                .with_context(|| format!("channel {} is not defined in [channels]", ch))?;

            let factor = conf
                .factor
                .with_context(|| format!("channel {} has no 'factor'", ch))?;
            let label = conf
                .label
                .as_deref()
                .with_context(|| format!("channel {} has no 'label'", ch))?;
            let unit = conf
                .unit_out
                .as_deref()
                .with_context(|| format!("channel {} has no 'unit_out'", ch))?;

            Ok(SensorMeta {
                factor,
                label,
                unit,
            })
        })
        .collect::<Result<Vec<_>>>()
}

fn fit_background(cfg: &Config, t: &[f64], s_cols: &[Vec<f64>]) -> Result<Vec<f64>> {
    let bg_window_before = &cfg.pulse.bg_window_before;
    let bg_window_after = &cfg.pulse.bg_window_after;

    let is_in_bg = |ti: &f64| {
        (ti >= &bg_window_before.start && ti <= &bg_window_before.end)
            || (ti >= &bg_window_after.start && ti <= &bg_window_after.end)
    };

    let t_fit = t.iter().cloned().filter(is_in_bg).collect::<Vec<f64>>();

    if t_fit.is_empty() {
        bail!("No data points found in background windows. Cannot fit background.");
    }

    s_cols
        .iter()
        .map(|col| {
            let s_fit = col
                .iter()
                .cloned()
                .zip(t.iter())
                .filter(|&(_yi, ti)| is_in_bg(ti)) // 同じ条件でフィルタ
                .map(|(yi, _ti)| yi)
                .collect::<Vec<f64>>();

            pulse_calculator::PulseBgFitter {}.fit(&t_fit, &s_fit)
        })
        .collect::<Result<Vec<_>>>()
}
