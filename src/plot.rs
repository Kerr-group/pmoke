use crate::config::{Plot, PlotDecimation};
use crate::ui;
use anyhow::{Context, Result};

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

pub fn stride_for_len(plot: &Plot, len: usize) -> usize {
    match plot.decimation {
        PlotDecimation::Stride => len.div_ceil(plot.max_points).max(1),
    }
}

pub fn decimate_1d(plot: &Plot, values: &[f64]) -> Vec<f64> {
    let stride = stride_for_len(plot, values.len());
    if stride == 1 {
        return values.to_vec();
    }
    values.iter().step_by(stride).copied().collect()
}

pub fn decimate_2d(plot: &Plot, values: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let len = values.first().map(|row| row.len()).unwrap_or(0);
    let stride = stride_for_len(plot, len);
    values
        .iter()
        .map(|row| row.iter().step_by(stride).copied().collect())
        .collect()
}

pub fn decimate_3d(plot: &Plot, values: &[Vec<Vec<f64>>]) -> Vec<Vec<Vec<f64>>> {
    let len = values
        .first()
        .and_then(|channel| channel.first())
        .map(|series| series.len())
        .unwrap_or(0);
    let stride = stride_for_len(plot, len);
    values
        .iter()
        .map(|channel| {
            channel
                .iter()
                .map(|series| series.iter().step_by(stride).copied().collect())
                .collect()
        })
        .collect()
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
}
