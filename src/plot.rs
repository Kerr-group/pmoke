use crate::config::{Plot, PlotDecimation};
use crate::ui;
use anyhow::{Context, Result, bail};
use std::fs;

pub type DecimatedSeries2d = (Vec<f64>, Vec<Vec<f64>>);
pub type DecimatedSeries3d = (Vec<f64>, Vec<Vec<Vec<f64>>>);

pub fn run_plot(
    plot: &Plot,
    progress: impl Into<String>,
    completed: impl Into<String>,
    f: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let progress = progress.into();
    let completed = completed.into();

    if !plot.enabled {
        ui::skipped(format!("{progress}: disabled"));
        return Ok(());
    }
    if !plot.save && !plot.interactive {
        ui::skipped(format!("{progress}: save=false and interactive=false"));
        return Ok(());
    }
    if plot.save {
        fs::create_dir_all(&plot.output_dir)
            .with_context(|| format!("failed to create plot output dir: {}", plot.output_dir))?;
    }

    let progress_message = if plot.interactive {
        format!("{progress} (close plot window to continue)")
    } else {
        progress.clone()
    };
    let pb = ui::spinner(progress_message);
    match f().with_context(|| progress.clone()) {
        Ok(()) => {
            ui::finish_success(pb, completed);
            Ok(())
        }
        Err(err) if plot.fail_on_error => {
            pb.finish_and_clear();
            Err(err)
        }
        Err(err) => {
            pb.finish_and_clear();
            ui::warn(format!("{progress} skipped: {err:#}"));
            Ok(())
        }
    }
}

#[cfg(test)]
pub fn stride_for_len(plot: &Plot, len: usize) -> usize {
    match plot.decimation {
        PlotDecimation::None | PlotDecimation::MinMax => 1,
        PlotDecimation::Stride => len.div_ceil(plot.max_points).max(1),
    }
}

#[cfg(test)]
pub fn decimate_1d(plot: &Plot, values: &[f64]) -> Vec<f64> {
    if plot.decimation == PlotDecimation::MinMax && values.len() > plot.max_points {
        let indices = min_max_indices(&[values], values.len(), plot.max_points);
        return apply_indices(values, &indices);
    }
    let stride = stride_for_len(plot, values.len());
    if stride == 1 {
        return values.to_vec();
    }
    values.iter().step_by(stride).copied().collect()
}

#[cfg(test)]
pub fn decimate_2d(plot: &Plot, values: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let indices = decimation_indices(plot, values);
    apply_indices_2d(values, &indices)
}

pub fn decimate_xy_2d(plot: &Plot, x: &[f64], values: &[Vec<f64>]) -> Result<DecimatedSeries2d> {
    let series = values.iter().map(Vec::as_slice).collect::<Vec<_>>();
    decimate_xy_slices(plot, x, &series)
}

pub fn decimate_xy_slices(plot: &Plot, x: &[f64], values: &[&[f64]]) -> Result<DecimatedSeries2d> {
    validate_decimation_input(plot, x, values)?;
    let indices = decimation_indices_slices(plot, values);
    Ok((
        apply_indices(x, &indices),
        apply_indices_2d(values, &indices),
    ))
}

pub fn decimate_xy_3d(
    plot: &Plot,
    x: &[f64],
    values: &[Vec<Vec<f64>>],
) -> Result<DecimatedSeries3d> {
    let series = values
        .iter()
        .flat_map(|channel| channel.iter().map(Vec::as_slice))
        .collect::<Vec<_>>();
    validate_decimation_input(plot, x, &series)?;
    let indices = decimation_indices_slices(plot, &series);
    Ok((
        apply_indices(x, &indices),
        apply_indices_3d(values, &indices),
    ))
}

fn validate_decimation_input(plot: &Plot, x: &[f64], values: &[&[f64]]) -> Result<()> {
    if plot.max_points == 0 {
        bail!("plot max_points must be positive");
    }
    if values.is_empty() {
        bail!("plot decimation requires at least one data series");
    }
    for (index, values) in values.iter().enumerate() {
        if values.len() != x.len() {
            bail!(
                "plot series {index} has {} points, expected {} to match the x axis",
                values.len(),
                x.len()
            );
        }
    }
    Ok(())
}

fn apply_indices(values: &[f64], indices: &[usize]) -> Vec<f64> {
    indices
        .iter()
        .filter_map(|&index| values.get(index).copied())
        .collect()
}

fn apply_indices_2d<T: AsRef<[f64]>>(values: &[T], indices: &[usize]) -> Vec<Vec<f64>> {
    values
        .iter()
        .map(|series| apply_indices(series.as_ref(), indices))
        .collect()
}

fn apply_indices_3d(values: &[Vec<Vec<f64>>], indices: &[usize]) -> Vec<Vec<Vec<f64>>> {
    values
        .iter()
        .map(|channel| apply_indices_2d(channel, indices))
        .collect()
}

#[cfg(test)]
fn decimation_indices(plot: &Plot, values: &[Vec<f64>]) -> Vec<usize> {
    let series = values.iter().map(Vec::as_slice).collect::<Vec<_>>();
    decimation_indices_slices(plot, &series)
}

fn decimation_indices_slices(plot: &Plot, values: &[&[f64]]) -> Vec<usize> {
    let len = values.first().map_or(0, |series| series.len());
    if len == 0 {
        return Vec::new();
    }
    match plot.decimation {
        PlotDecimation::None => (0..len).collect(),
        PlotDecimation::Stride => {
            let stride = len.div_ceil(plot.max_points).max(1);
            (0..len).step_by(stride).collect()
        }
        PlotDecimation::MinMax if len <= plot.max_points => (0..len).collect(),
        PlotDecimation::MinMax => min_max_indices(values, len, plot.max_points),
    }
}

fn min_max_indices(values: &[&[f64]], len: usize, max_points: usize) -> Vec<usize> {
    if max_points == 1 {
        return vec![global_extreme_index(values, len)];
    }
    let aligned = values
        .iter()
        .copied()
        .filter(|series| series.len() == len)
        .collect::<Vec<_>>();
    if aligned.is_empty() {
        return (0..len).step_by(len.div_ceil(max_points)).collect();
    }
    let extrema_per_bin = aligned.len().saturating_mul(2).max(1);
    let bin_count = (max_points / extrema_per_bin).max(1).min(len);
    let mut indices = Vec::with_capacity(bin_count.saturating_mul(extrema_per_bin));
    for bin in 0..bin_count {
        let start = partition_boundary(bin, len, bin_count);
        let end = partition_boundary(bin + 1, len, bin_count).max(start + 1);
        for series in &aligned {
            let mut minimum = None;
            let mut maximum = None;
            for index in start..end {
                let value = series[index];
                if !value.is_finite() {
                    continue;
                }
                if minimum.is_none_or(|current: usize| value < series[current]) {
                    minimum = Some(index);
                }
                if maximum.is_none_or(|current: usize| value > series[current]) {
                    maximum = Some(index);
                }
            }
            indices.push(minimum.unwrap_or(start));
            indices.push(maximum.unwrap_or(start));
        }
    }
    indices.sort_unstable();
    indices.dedup();
    if indices.len() <= max_points {
        return indices;
    }
    let stride = indices.len().div_ceil(max_points);
    indices.into_iter().step_by(stride).collect()
}

fn partition_boundary(position: usize, len: usize, partition_count: usize) -> usize {
    debug_assert!(partition_count > 0);
    debug_assert!(position <= partition_count);
    // usize is at most 64 bits on supported Rust targets, so the full product
    // fits in u128 even when both operands are usize::MAX.
    ((position as u128) * (len as u128) / (partition_count as u128)) as usize
}

fn global_extreme_index(values: &[&[f64]], len: usize) -> usize {
    let mut best = (0, f64::NEG_INFINITY);
    for series in values.iter().filter(|series| series.len() == len) {
        for (index, value) in series.iter().copied().enumerate() {
            if value.is_finite() && value.abs() > best.1 {
                best = (index, value.abs());
            }
        }
    }
    best.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plot(max_points: usize) -> Plot {
        Plot {
            max_points,
            ..Plot::default()
        }
    }

    #[test]
    fn stride_decimation_limits_time_axis_points() {
        let plot = plot(4);
        let values = (0..10).map(|x| x as f64).collect::<Vec<_>>();

        assert_eq!(decimate_1d(&plot, &values), vec![0.0, 3.0, 6.0, 9.0]);
    }

    #[test]
    fn stride_decimation_keeps_multiseries_alignment() {
        let plot = plot(3);
        let values = vec![
            vec![0.0, 1.0, 2.0, 3.0, 4.0],
            vec![10.0, 11.0, 12.0, 13.0, 14.0],
        ];

        assert_eq!(
            decimate_2d(&plot, &values),
            vec![vec![0.0, 2.0, 4.0], vec![10.0, 12.0, 14.0]]
        );
    }

    #[test]
    fn min_max_decimation_preserves_a_narrow_spike_and_alignment() {
        let mut plot = plot(6);
        plot.decimation = PlotDecimation::MinMax;
        let time = (0..100).map(|index| index as f64).collect::<Vec<_>>();
        let mut signal = vec![0.0; 100];
        signal[47] = 100.0;
        let context = time.iter().map(|value| value + 1_000.0).collect::<Vec<_>>();

        let (time_plot, values_plot) = decimate_xy_2d(&plot, &time, &[signal, context]).unwrap();

        let spike_position = time_plot.iter().position(|value| *value == 47.0).unwrap();
        assert_eq!(values_plot[0][spike_position], 100.0);
        assert_eq!(values_plot[1][spike_position], 1_047.0);
        assert!(time_plot.len() <= plot.max_points);
        assert!(
            values_plot
                .iter()
                .all(|series| series.len() == time_plot.len())
        );
    }

    #[test]
    fn no_decimation_ignores_the_display_point_limit() {
        let mut plot = plot(2);
        plot.decimation = PlotDecimation::None;
        let values = vec![0.0, 1.0, 2.0, 3.0];

        assert_eq!(decimate_1d(&plot, &values), values);
    }

    #[test]
    fn min_max_decimation_ignores_nan_when_finite_extrema_exist() {
        let mut plot = plot(2);
        plot.decimation = PlotDecimation::MinMax;
        let values = vec![f64::NAN, -2.0, 5.0, f64::NAN];

        assert_eq!(decimate_1d(&plot, &values), vec![-2.0, 5.0]);
    }

    #[test]
    fn aligned_decimation_rejects_ragged_series_and_zero_limit() {
        let mut plot = plot(10);
        let error = decimate_xy_2d(&plot, &[0.0, 1.0], &[vec![1.0]]).unwrap_err();
        assert!(error.to_string().contains("expected 2"));

        plot.max_points = 0;
        let error = decimate_xy_2d(&plot, &[0.0], &[vec![1.0]]).unwrap_err();
        assert!(error.to_string().contains("must be positive"));
    }

    #[test]
    fn partition_boundaries_do_not_overflow_at_usize_limits() {
        let len = usize::MAX;

        assert_eq!(partition_boundary(0, len, 3), 0);
        assert_eq!(partition_boundary(1, len, 3), len / 3);
        assert_eq!(partition_boundary(2, len, 3), len - len.div_ceil(3));
        assert_eq!(partition_boundary(3, len, 3), len);
    }
}
