use crate::config::Plot;
use crate::plot::decimate_xy_2d;
use crate::python;
use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::sync::OnceLock;

const SENSOR_INTEGRAL_PLOT_PY: &str = include_str!("pytools/sensor_integral_plot.py");
static SENSOR_INTEGRAL_PLOT_MODULE: OnceLock<Py<PyModule>> = OnceLock::new();

#[allow(dead_code)]
pub struct SensorIntegralPlotter {}

impl SensorIntegralPlotter {
    pub fn plot(
        &self,
        plot: &Plot,
        t: &[f64],
        y: &[Vec<f64>],
        index_arr: &[u8],
        label_arr: &[&str],
        unit_arr: &[&str],
    ) -> Result<()> {
        Python::attach(|py| {
            let plot_mod = python::cached_module(
                py,
                &SENSOR_INTEGRAL_PLOT_MODULE,
                SENSOR_INTEGRAL_PLOT_PY,
                "sensor_integral_plot.py",
                "sensor_integral_plot",
            )
            .context("failed to load sensor_integral_plot.py")?;
            let (t_plot, y_plot) = decimate_xy_2d(plot, t, y)?;
            let t_obj = python::f64_array1(py, &t_plot);
            let y_obj = python::f64_array2(py, &y_plot)?;

            let plotter = plot_mod
                .getattr("SensorIntegralPlotter")?
                .call0()
                .context("failed to create SensorIntegralPlotter instance")?;

            plotter
                .call_method1(
                    "plot",
                    (
                        t_obj,
                        y_obj,
                        index_arr,
                        label_arr,
                        unit_arr,
                        plot.save,
                        plot.interactive,
                        &plot.output_dir,
                    ),
                )
                .context("python SensorIntegralPlotter.plot(...) failed")?;

            Ok(())
        })
    }
}
