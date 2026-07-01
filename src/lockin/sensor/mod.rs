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

#[derive(Debug, Clone, Copy)]
struct SensorIntegralMaximum {
    value: f64,
    time: f64,
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

    validate_sensor_lengths(t, s_cols)?;

    let sensor_meta = extract_sensor_metadata(cfg)?;
    validate_sensor_channel_alignment(&cfg.roles.sensor_ch, s_cols, sensor_ch, &sensor_meta)?;

    let pb = ui::spinner("averaging sensor backgrounds");
    let t0 = std::time::Instant::now();
    let c_bg_arr = match calculate_background_averages(cfg, t, s_cols) {
        Ok(c_bg_arr) => c_bg_arr,
        Err(err) => {
            pb.finish_and_clear();
            return Err(err);
        }
    };
    ui::finish_success(
        pb,
        format!(
            "sensor backgrounds averaged ({})",
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

    let maxima = sensor_ch
        .iter()
        .copied()
        .zip(s_integral.iter())
        .map(|(ch, integral)| maximum_absolute_integral(t, integral, ch))
        .collect::<Result<Vec<_>>>()?;
    ui::settings_table(
        "Sensor integral maxima",
        sensor_ch
            .iter()
            .zip(sensor_meta.iter())
            .zip(maxima.iter())
            .map(|((&ch, meta), maximum)| {
                (
                    format!("ch{ch} {}", meta.label),
                    format!(
                        "max_abs={:.6e} {}, time={:.6e} s",
                        maximum.value, meta.unit, maximum.time
                    ),
                )
            })
            .collect(),
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

fn validate_sensor_channel_alignment(
    configured_sensor_ch: &[u8],
    s_cols: &[&[f64]],
    sensor_ch: &[u8],
    sensor_meta: &[SensorMeta<'_>],
) -> Result<()> {
    if sensor_ch != configured_sensor_ch {
        bail!(
            "sensor channel order {:?} does not match configured sensor channels {:?}",
            sensor_ch,
            configured_sensor_ch
        );
    }
    if s_cols.len() != sensor_ch.len() || sensor_meta.len() != sensor_ch.len() {
        bail!(
            "sensor channel count mismatch: data={}, channels={}, metadata={}",
            s_cols.len(),
            sensor_ch.len(),
            sensor_meta.len()
        );
    }
    Ok(())
}

fn maximum_absolute_integral(
    t: &[f64],
    integral: &[f64],
    sensor_ch: u8,
) -> Result<SensorIntegralMaximum> {
    if integral.is_empty() {
        bail!("sensor ch{sensor_ch} integral is empty");
    }
    if t.len() != integral.len() {
        bail!(
            "time length ({}) and sensor ch{sensor_ch} integral length ({}) differ",
            t.len(),
            integral.len()
        );
    }

    let mut maximum = None;
    for (index, (&time, &value)) in t.iter().zip(integral.iter()).enumerate() {
        if !time.is_finite() {
            bail!("sensor ch{sensor_ch} time at index {index} is not finite");
        }
        if !value.is_finite() {
            bail!("sensor ch{sensor_ch} integral at index {index} is not finite");
        }

        let candidate = SensorIntegralMaximum {
            value: value.abs(),
            time,
        };
        if maximum.is_none_or(|current: SensorIntegralMaximum| candidate.value > current.value) {
            maximum = Some(candidate);
        }
    }

    maximum.ok_or_else(|| anyhow::anyhow!("sensor ch{sensor_ch} integral is empty"))
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

fn validate_sensor_lengths(t: &[f64], s_cols: &[&[f64]]) -> Result<()> {
    for (column_index, column) in s_cols.iter().enumerate() {
        if t.len() != column.len() {
            bail!(
                "time length ({}) and sensor column {} length ({}) differ",
                t.len(),
                column_index + 1,
                column.len()
            );
        }
    }
    Ok(())
}

fn calculate_background_averages(cfg: &Config, t: &[f64], s_cols: &[&[f64]]) -> Result<Vec<f64>> {
    let bg_window_before = &cfg.pulse.bg_window_before;
    let bg_window_after = &cfg.pulse.bg_window_after;

    let is_in_bg = |ti: &f64| {
        (ti >= &bg_window_before.start && ti <= &bg_window_before.end)
            || (ti >= &bg_window_after.start && ti <= &bg_window_after.end)
    };

    if !t.iter().any(is_in_bg) {
        bail!("No data points found in background windows. Cannot calculate background average.");
    }

    s_cols
        .iter()
        .map(|col| {
            let values = col
                .iter()
                .zip(t.iter())
                .filter_map(|(&yi, ti)| is_in_bg(ti).then_some(yi));

            pulse_calculator::PulseBgAverage {}.calculate(values)
        })
        .collect::<Result<Vec<_>>>()
}

#[cfg(test)]
mod tests {
    use super::{
        SensorMeta, maximum_absolute_integral, validate_sensor_channel_alignment,
        validate_sensor_lengths,
    };

    #[test]
    fn sensor_length_validation_checks_every_channel() {
        let time = [0.0, 1.0, 2.0];
        let first = [1.0, 2.0, 3.0];
        let second = [4.0, 5.0];

        let error = validate_sensor_lengths(&time, &[&first, &second]).unwrap_err();

        assert!(error.to_string().contains("sensor column 2"));
    }

    #[test]
    fn sensor_channel_alignment_checks_order_and_count() {
        let data = [0.0, 1.0];
        let columns = [&data[..]];
        let metadata = [SensorMeta {
            factor: 1.0,
            label: "sensor",
            unit: "V s",
        }];

        validate_sensor_channel_alignment(&[2], &columns, &[2], &metadata).unwrap();

        let order_error =
            validate_sensor_channel_alignment(&[1], &columns, &[2], &metadata).unwrap_err();
        assert!(order_error.to_string().contains("order"));

        let count_error =
            validate_sensor_channel_alignment(&[2], &[], &[2], &metadata).unwrap_err();
        assert!(count_error.to_string().contains("count mismatch"));
    }

    #[test]
    fn maximum_absolute_integral_uses_full_signed_integral_series() {
        let maximum =
            maximum_absolute_integral(&[0.0, 0.1, 0.2, 0.3], &[0.0, -3.0, 2.0, 1.0], 2).unwrap();

        assert_eq!(maximum.value, 3.0);
        assert_eq!(maximum.time, 0.1);
    }

    #[test]
    fn maximum_absolute_integral_keeps_first_equal_maximum() {
        let maximum = maximum_absolute_integral(&[0.0, 1.0, 2.0], &[-2.0, 2.0, 1.0], 1).unwrap();

        assert_eq!(maximum.value, 2.0);
        assert_eq!(maximum.time, 0.0);
    }

    #[test]
    fn maximum_absolute_integral_accepts_zero_and_rejects_invalid_inputs() {
        let zero = maximum_absolute_integral(&[0.0, 1.0], &[-0.0, 0.0], 1).unwrap();
        assert_eq!(zero.value, 0.0);

        assert!(maximum_absolute_integral(&[], &[], 1).is_err());
        assert!(maximum_absolute_integral(&[0.0], &[0.0, 1.0], 1).is_err());
        assert!(maximum_absolute_integral(&[f64::NAN], &[0.0], 1).is_err());
        assert!(maximum_absolute_integral(&[0.0], &[f64::INFINITY], 1).is_err());
    }
}
