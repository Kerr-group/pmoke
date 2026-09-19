pub mod pulse_calculator;
pub mod sensor_integral_plot;
pub mod sensor_raw_plot;

use crate::analysis_results::{build_analysis_headers, write_analysis_results};
use crate::config::{Channel, Config};
use crate::constants::FETCHED_FNAME;
use crate::utils::time_axis::TimeAxisRef;
use crate::utils::waveform::read_waveform_channels;
use crate::{plot, ui};
use anyhow::{Context, Result, bail};

pub struct SensorMeta<'a> {
    pub scale: SensorScale,
    pub label: &'a str,
    pub unit: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub enum SensorScale {
    Factor(f64),
    ScaleToAbsMax(f64),
}

pub struct SensorOutput {
    pub t: Vec<f64>,
    pub rate: Vec<Vec<f64>>,
    pub integral: Vec<Vec<f64>>,
}

#[derive(Debug, Clone, Copy)]
struct SensorIntegralMaximum {
    value: f64,
    time: f64,
}

pub fn run(cfg: &Config) -> Result<()> {
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

    let _ = run_sensor(cfg, &waveform.t, &s_col_refs, &sensor_ch)?;
    Ok(())
}

pub fn run_sensor<'a>(
    cfg: &Config,
    t: impl Into<TimeAxisRef<'a>>,
    s_cols: &[&[f64]],
    sensor_ch: &[u8],
) -> Result<SensorOutput> {
    let t = t.into();
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
            ui::finish_cancelled(pb, "sensor background averaging stopped");
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

    // Stride-decimated grid shared by sensor.csv and the sensor plots (D2).
    // Plain every-Nth decimation over the full range: no window trim and no
    // reference needed. Rows = floor((n - 1) / stride) + 1.
    //
    // FR-05 (Issue #264, Card B): `lockin.stride_samples` is a defaulted
    // sensor-required item, not a lock-in dependency. Card A fills an absent
    // `[lockin]` with the inert stride 100, load-time validation keeps it
    // positive, and no `[lockin]` section is needed for `pmoke sensor`; the
    // read below must stay a plain positive-stride decimation.
    let stride = cfg.lockin.stride_samples;
    if stride == 0 {
        bail!("lockin.stride_samples must be positive");
    }
    let sample_count = t.len();
    let indices: Vec<usize> = (0..sample_count).step_by(stride).collect();
    let t_full = t.values(0..sample_count);
    let t_decimated: Vec<f64> = indices.iter().map(|&i| t.value_at(i)).collect();
    let decimate = |columns: &[Vec<f64>]| -> Vec<Vec<f64>> {
        columns
            .iter()
            .map(|col| indices.iter().map(|&i| col[i]).collect())
            .collect()
    };

    plot::run_plot(
        &cfg.plot,
        &cfg.paths().sensor_raw_combined_plot(),
        "plotting sensor raw data",
        "sensor raw plot completed",
        |output| {
            let s_stride: Vec<Vec<f64>> = s_cols
                .iter()
                .map(|col| indices.iter().map(|&i| col[i]).collect())
                .collect();
            sensor_raw_plot::SensorRawPlotter {}
                .plot(
                    &cfg.plot,
                    output,
                    &t_decimated,
                    s_stride,
                    sensor_ch,
                    &c_bg_arr,
                )
                .context("failed to plot sensor data")
        },
    )?;

    let pb = ui::progress("integrating sensor pulses", s_cols.len() as u64);
    let start = std::time::Instant::now();

    let dt = t
        .dt()
        .ok_or_else(|| anyhow::anyhow!("time axis must contain at least two samples"))?;
    let sensor_series = s_cols
        .iter()
        .zip(c_bg_arr.iter())
        .zip(sensor_meta.iter())
        .map(|((s, &c_bg), meta)| {
            let series = calculate_sensor_series(dt, s, c_bg, meta)?;
            pb.inc(1);
            Ok(series)
        })
        .collect::<Result<Vec<_>>>()?;
    let scale_summary = sensor_ch
        .iter()
        .zip(sensor_meta.iter())
        .zip(sensor_series.iter())
        .filter_map(|((&ch, meta), series)| match meta.scale {
            SensorScale::ScaleToAbsMax(target) => Some((
                format!("ch{ch} {}", meta.label),
                format!(
                    "scale_to_abs_max={:.6e} {}, unscaled_max_abs={:.6e}, factor={:.6e}",
                    target,
                    meta.unit,
                    series
                        .unscaled_max_abs
                        .expect("auto-scaled sensor series must record unscaled max abs"),
                    series.factor
                ),
            )),
            SensorScale::Factor(_) => None,
        })
        .collect::<Vec<_>>();
    if !scale_summary.is_empty() {
        ui::settings_table("Sensor auto scales", scale_summary);
    }
    let s_rate = sensor_series
        .iter()
        .map(|series| series.rate.clone())
        .collect::<Vec<_>>();
    let s_integral = sensor_series
        .iter()
        .map(|series| series.integral.clone())
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

    let s_rate_decimated = decimate(&s_rate);
    let s_integral_decimated = decimate(&s_integral);

    let labels: Vec<&str> = sensor_meta.iter().map(|m| m.label).collect();
    let units: Vec<&str> = sensor_meta.iter().map(|m| m.unit).collect();

    plot::run_plot(
        &cfg.plot,
        &cfg.paths().sensor_integral_combined_plot(),
        "plotting sensor integrals",
        "sensor integral plot completed",
        |output| {
            sensor_integral_plot::SensorIntegralPlotter {}
                .plot(
                    &cfg.plot,
                    output,
                    &t_decimated,
                    &s_integral_decimated,
                    (sensor_ch, &labels, &units),
                )
                .context("failed to plot sensor integrals")
        },
    )?;

    let headers = build_analysis_headers(cfg, Vec::<String>::new())?;
    // FR-05 (Issue #264, Card B): `lockin.save_npy` is a defaulted
    // sensor-required item (Card A inert default: no npy); no `[lockin]`
    // section is needed for `pmoke sensor`.
    write_analysis_results(
        cfg.paths().sensor_csv(),
        &headers,
        &t_decimated,
        &s_rate_decimated,
        &s_integral_decimated,
        &[],
        cfg.lockin.save_npy,
    )?;

    Ok(SensorOutput {
        t: t_full,
        rate: s_rate,
        integral: s_integral,
    })
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

fn maximum_absolute_integral<'a>(
    t: impl Into<TimeAxisRef<'a>>,
    integral: &[f64],
    sensor_ch: u8,
) -> Result<SensorIntegralMaximum> {
    let t = t.into();
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
    for (index, (time, &value)) in t.iter().zip(integral.iter()).enumerate() {
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

            let scale = sensor_scale_from_channel(conf)?;
            let label = conf
                .label
                .as_deref()
                .with_context(|| format!("channel {} has no 'label'", ch))?;
            let unit = conf
                .unit_out
                .as_deref()
                .with_context(|| format!("channel {} has no 'unit_out'", ch))?;

            Ok(SensorMeta { scale, label, unit })
        })
        .collect::<Result<Vec<_>>>()
}

fn sensor_scale_from_channel(channel: &Channel) -> Result<SensorScale> {
    match (channel.factor, channel.scale_to_abs_max) {
        (Some(factor), None) => {
            if !factor.is_finite() {
                bail!("channel {} factor must be finite", channel.index);
            }
            Ok(SensorScale::Factor(factor))
        }
        (None, Some(scale_to_abs_max)) => {
            if !scale_to_abs_max.is_finite() || scale_to_abs_max == 0.0 {
                bail!(
                    "channel {} scale_to_abs_max must be finite and non-zero",
                    channel.index
                );
            }
            Ok(SensorScale::ScaleToAbsMax(scale_to_abs_max))
        }
        (Some(_), Some(_)) => bail!(
            "channel {} cannot set both 'factor' and 'scale_to_abs_max'",
            channel.index
        ),
        (None, None) => bail!(
            "channel {} must set either 'factor' or 'scale_to_abs_max'",
            channel.index
        ),
    }
}

fn validate_sensor_lengths<'a>(t: impl Into<TimeAxisRef<'a>>, s_cols: &[&[f64]]) -> Result<()> {
    let t = t.into();
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

fn calculate_background_averages(
    cfg: &Config,
    t: TimeAxisRef<'_>,
    s_cols: &[&[f64]],
) -> Result<Vec<f64>> {
    let bg_window_before = &cfg.pulse.bg_window_before;
    let bg_window_after = &cfg.pulse.bg_window_after;

    let is_in_bg = |ti: f64| {
        (ti >= bg_window_before.start && ti <= bg_window_before.end)
            || (ti >= bg_window_after.start && ti <= bg_window_after.end)
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
fn calculate_sensor_rates(
    s_cols: &[&[f64]],
    c_bg_arr: &[f64],
    sensor_meta: &[SensorMeta<'_>],
) -> Result<Vec<Vec<f64>>> {
    s_cols
        .iter()
        .zip(c_bg_arr.iter())
        .zip(sensor_meta.iter())
        .map(|((s, &c_bg), meta)| {
            let factor = match meta.scale {
                SensorScale::Factor(factor) => factor,
                SensorScale::ScaleToAbsMax(_) => {
                    bail!("scale_to_abs_max requires integral-based factor calculation")
                }
            };
            Ok(s.iter()
                .map(|&value| (value - c_bg) * factor)
                .collect::<Vec<_>>())
        })
        .collect()
}

#[derive(Debug)]
struct SensorSeries {
    factor: f64,
    unscaled_max_abs: Option<f64>,
    rate: Vec<f64>,
    integral: Vec<f64>,
}

fn calculate_sensor_series(
    dt: f64,
    data: &[f64],
    background: f64,
    meta: &SensorMeta<'_>,
) -> Result<SensorSeries> {
    let unscaled_integral =
        pulse_calculator::PulseIntegralCalculator::new(dt).integrate(data, background, 1.0);
    let (factor, unscaled_max_abs) = match meta.scale {
        SensorScale::Factor(factor) => {
            if !factor.is_finite() {
                bail!("sensor factor is not finite");
            }
            (factor, None)
        }
        SensorScale::ScaleToAbsMax(target) => {
            if !target.is_finite() || target == 0.0 {
                bail!("sensor scale_to_abs_max must be finite and non-zero");
            }
            let max_abs = max_abs_finite(&unscaled_integral)?;
            (target / max_abs, Some(max_abs))
        }
    };
    if !factor.is_finite() {
        bail!("sensor factor is not finite");
    }
    let rate = data
        .iter()
        .map(|&value| (value - background) * factor)
        .collect::<Vec<_>>();
    let integral = unscaled_integral
        .iter()
        .map(|&value| value * factor)
        .collect::<Vec<_>>();
    Ok(SensorSeries {
        factor,
        unscaled_max_abs,
        rate,
        integral,
    })
}

fn max_abs_finite(values: &[f64]) -> Result<f64> {
    let mut max_abs = None;
    for &value in values {
        if !value.is_finite() {
            bail!("sensor unscaled integral contains a non-finite value");
        }
        let abs = value.abs();
        if max_abs.is_none_or(|current| abs > current) {
            max_abs = Some(abs);
        }
    }
    let max_abs = max_abs.ok_or_else(|| anyhow::anyhow!("sensor unscaled integral is empty"))?;
    if max_abs == 0.0 {
        bail!("sensor unscaled integral maximum is zero");
    }
    Ok(max_abs)
}

#[cfg(test)]
mod tests {
    use super::{
        SensorMeta, SensorScale, calculate_sensor_rates, calculate_sensor_series,
        maximum_absolute_integral, validate_sensor_channel_alignment, validate_sensor_lengths,
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
            scale: SensorScale::Factor(1.0),
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

    #[test]
    fn sensor_rate_applies_background_subtraction_and_factor() {
        let first = [1.0, 2.0, 4.0];
        let second = [10.0, 8.0, 6.0];
        let metadata = [
            SensorMeta {
                scale: SensorScale::Factor(2.0),
                label: "field",
                unit: "T",
            },
            SensorMeta {
                scale: SensorScale::Factor(-0.5),
                label: "current",
                unit: "A",
            },
        ];

        let rates = calculate_sensor_rates(&[&first, &second], &[1.5, 8.0], &metadata).unwrap();

        assert_eq!(rates[0], vec![-1.0, 1.0, 5.0]);
        assert_eq!(rates[1], vec![-1.0, -0.0, 1.0]);
    }

    #[test]
    fn sensor_scale_to_abs_max_uses_unscaled_integral_maximum() {
        let data = [1.0, 3.0, 5.0];
        let metadata = SensorMeta {
            scale: SensorScale::ScaleToAbsMax(-8.0),
            label: "field",
            unit: "T",
        };

        let series = calculate_sensor_series(1.0, &data, 1.0, &metadata).unwrap();

        assert_eq!(series.factor, -2.0);
        assert_eq!(series.unscaled_max_abs, Some(4.0));
        assert_eq!(series.rate, vec![-0.0, -4.0, -8.0]);
        assert_eq!(series.integral, vec![-0.0, -2.0, -8.0]);
    }

    #[test]
    fn sensor_scale_to_abs_max_rejects_zero_integral() {
        let metadata = SensorMeta {
            scale: SensorScale::ScaleToAbsMax(1.0),
            label: "field",
            unit: "T",
        };

        let error = calculate_sensor_series(1.0, &[1.0, 1.0], 1.0, &metadata).unwrap_err();

        assert!(error.to_string().contains("maximum is zero"));
    }

    struct TemporaryDirectory(std::path::PathBuf);

    impl TemporaryDirectory {
        fn new(tag: &str) -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "pmoke-sensor-{}-{}-{nonce}",
                tag,
                std::process::id()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }

    impl Drop for TemporaryDirectory {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn sensor_test_config(
        directory: &TemporaryDirectory,
        stride_samples: usize,
    ) -> crate::config::Config {
        let mut cfg = crate::test_support::test_config(vec![1], vec![]);
        // No reference channel: the sensor stage must not require one.
        cfg.roles.reference_ch = 0;
        cfg.set_artifact_root(directory.0.clone());
        cfg.lockin.stride_samples = stride_samples;
        cfg
    }

    #[test]
    fn run_sensor_writes_full_rate_output_and_decimated_csv_without_reference() {
        use crate::utils::csv::read_csv;

        let directory = TemporaryDirectory::new("decoupling");
        let cfg = sensor_test_config(&directory, 3);

        let sample_count = 10;
        let time: Vec<f64> = (0..sample_count).map(|index| index as f64 * 0.1).collect();
        let sensor: Vec<f64> = (0..sample_count)
            .map(|index| if index < 5 { 0.0 } else { 1.0 })
            .collect();

        let output = super::run_sensor(&cfg, &time[..], &[&sensor[..]], &[1]).unwrap();

        // Full-rate series are returned for downstream striding.
        assert_eq!(output.t.len(), sample_count);
        assert_eq!(output.rate.len(), 1);
        assert_eq!(output.rate[0].len(), sample_count);
        assert_eq!(output.integral[0].len(), sample_count);
        assert!(output.rate[0].iter().all(|value| value.is_finite()));

        // CSV shares the stride-decimated grid: floor((n - 1) / stride) + 1 rows.
        let expected_rows = (sample_count - 1) / 3 + 1;
        let columns = read_csv(cfg.paths().sensor_csv()).unwrap();
        // Layout: time, sensor rate, sensor integral.
        assert_eq!(columns.len(), 3);
        assert_eq!(columns[0].len(), expected_rows);
        for (row, value) in columns[0].iter().enumerate() {
            let expected = (row * 3) as f64 * 0.1;
            assert!(
                (value - expected).abs() < 1e-12,
                "row {row}: expected time {expected}, got {value}"
            );
        }
    }

    #[test]
    fn sensor_validation_passes_without_reference_channel() {
        use crate::config::ValidationTarget;
        use crate::config::validate_for_target;

        let directory = TemporaryDirectory::new("validation");
        let mut cfg = sensor_test_config(&directory, 1);
        cfg.instruments = Some(crate::config::Instruments {
            function_generator: None,
            oscilloscope: crate::config::Oscilloscope {
                connection: crate::config::Connection::Tcpip {
                    ip: "127.0.0.1".to_string(),
                    port: 55255,
                },
                model: "DHO5108".to_string(),
            },
        });
        std::fs::write(directory.0.join("raw.csv"), b"ch1,ch2,ch3\n0,0,0\n").unwrap();

        validate_for_target(&cfg, ValidationTarget::Sensor).unwrap();

        // The lock-in stage still requires a reference channel.
        let error = validate_for_target(&cfg, ValidationTarget::Li).unwrap_err();
        assert!(error.to_string().contains("reference_ch"));
    }
}
