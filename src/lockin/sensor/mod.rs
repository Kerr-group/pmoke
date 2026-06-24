pub mod pulse_calculator;
pub mod sensor_integral_plot;
pub mod sensor_raw_plot;

use crate::config::Config;
use crate::constants::FETCHED_FNAME;
use crate::lockin::reference::run_fit_ref;
use crate::lockin::stride::{li_stride_1d, li_stride_2d};
use crate::utils::waveform::read_waveform_channels;
use crate::{plot, ui};
use anyhow::{Context, Result, bail};

pub struct SensorMeta<'a> {
    pub factor: f64,
    pub label: &'a str,
    pub unit: &'a str,
}

pub fn run(cfg: &Config) -> Result<()> {
    let ref_fit_params = run_fit_ref(cfg)?;

    let sensor_ch = cfg.roles.sensor_ch.clone();
    let pb = ui::spinner(format!("reading sensor channels {:?}", sensor_ch));
    let t0 = std::time::Instant::now();
    let waveform =
        read_waveform_channels(cfg, &sensor_ch).context("failed to read sensor channels")?;
    let s_col_refs: Vec<&[f64]> = waveform.channels.iter().map(|col| col.as_slice()).collect();
    ui::finish_read(
        pb,
        format!(
            "sensor channels {:?} ({})",
            sensor_ch,
            ui::fmt_duration(t0.elapsed())
        ),
    );

    let _ = run_sensor(
        cfg,
        &waveform.t,
        &s_col_refs,
        &sensor_ch,
        ref_fit_params.f_ref,
    )?;
    Ok(())
}

pub fn run_sensor(
    cfg: &Config,
    t: &[f64],
    s_cols: &[&[f64]],
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

    let sensor_meta = extract_sensor_metadata(cfg)?;

    let pb = ui::spinner("fitting sensor backgrounds");
    let t0 = std::time::Instant::now();
    let c_bg_arr = match fit_background(cfg, t, s_cols) {
        Ok(c_bg_arr) => c_bg_arr,
        Err(err) => {
            pb.finish_and_clear();
            return Err(err);
        }
    };
    ui::finish_success(
        pb,
        format!(
            "sensor backgrounds fitted ({})",
            ui::fmt_duration(t0.elapsed())
        ),
    );

    let t_stride = li_stride_1d(cfg, t, t, f_ref)?;

    plot::run_plot(
        &cfg.plot,
        "plotting sensor raw data",
        "sensor raw plot completed",
        || {
            let s_stride = li_stride_2d(cfg, t, s_cols, f_ref)?;
            sensor_raw_plot::SensorRawPlotter {}
                .plot(&cfg.plot, &t_stride, s_stride, sensor_ch, &c_bg_arr)
                .context("failed to plot sensor data")
        },
    )?;

    let pb = ui::progress("integrating sensor pulses", s_cols.len() as u64);
    let start = std::time::Instant::now();

    let dt = t
        .windows(2)
        .next()
        .map(|w| w[1] - w[0])
        .ok_or_else(|| anyhow::anyhow!("time axis must contain at least two samples"))?;
    let s_integral = s_cols
        .iter()
        .zip(c_bg_arr.iter())
        .zip(sensor_meta.iter())
        .map(|((s, &c_bg), meta)| {
            let integral =
                pulse_calculator::PulseIntegralCalculator::new(dt).integrate(s, c_bg, meta.factor);
            pb.inc(1);
            integral
        })
        .collect::<Vec<_>>();

    let elapsed = start.elapsed();
    ui::finish_success(
        pb,
        format!(
            "sensor integrations completed ({})",
            ui::fmt_duration(elapsed)
        ),
    );

    let s_integral_stride = li_stride_2d(cfg, t, &s_integral, f_ref)?;

    let labels: Vec<&str> = sensor_meta.iter().map(|m| m.label).collect();
    let units: Vec<&str> = sensor_meta.iter().map(|m| m.unit).collect();

    plot::run_plot(
        &cfg.plot,
        "plotting sensor integrals",
        "sensor integral plot completed",
        || {
            sensor_integral_plot::SensorIntegralPlotter {}
                .plot(
                    &cfg.plot,
                    &t_stride,
                    &s_integral_stride,
                    sensor_ch,
                    &labels,
                    &units,
                )
                .context("failed to plot sensor integrals")
        },
    )?;

    Ok((t_stride, s_integral_stride))
}

pub fn extract_sensor_metadata<'a>(cfg: &'a Config) -> Result<Vec<SensorMeta<'a>>> {
    let sensor_ch = &cfg.roles.sensor_ch;
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

fn fit_background(cfg: &Config, t: &[f64], s_cols: &[&[f64]]) -> Result<Vec<f64>> {
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
