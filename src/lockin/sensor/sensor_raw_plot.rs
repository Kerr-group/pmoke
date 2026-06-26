use crate::config::Plot;
use crate::plot::{decimate_1d, decimate_2d};
use crate::python;
use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::sync::OnceLock;

const SENSOR_RAW_PLOT_PY: &str = include_str!("pytools/sensor_raw_plot.py");
static SENSOR_RAW_PLOT_MODULE: OnceLock<Py<PyModule>> = OnceLock::new();

#[allow(dead_code)]
pub struct SensorRawPlotter {}

impl SensorRawPlotter {
    pub fn plot(
        &self,
        plot: &Plot,
        t: &[f64],
        y: Vec<Vec<f64>>,
        index_arr: &[u8],
        c_bg_arr: &[f64],
    ) -> Result<()> {
        Python::attach(|py| {
            let plot_mod = python::cached_module(
                py,
                &SENSOR_RAW_PLOT_MODULE,
                SENSOR_RAW_PLOT_PY,
                "sensor_raw_plot.py",
                "sensor_raw_plot",
            )
            .context("failed to load sensor_raw_plot.py")?;
            let t_plot = decimate_1d(plot, t);
            let y_plot = decimate_2d(plot, &y);
            let t_obj = python::f64_array1(py, &t_plot);
            let y_obj = python::f64_array2(py, &y_plot)?;
            let c_bg_obj = python::f64_array1(py, c_bg_arr);

            let plotter = plot_mod
                .getattr("SensorRawPlotter")?
                .call0()
                .context("failed to create SensorRawPlotter instance")?;

            plotter
                .call_method1(
                    "plot",
                    (
                        t_obj,
                        y_obj,
                        index_arr,
                        c_bg_obj,
                        plot.save,
                        plot.interactive,
                        &plot.output_dir,
                    ),
                )
                .context("python SensorRawPlotter.plot(...) failed")?;

            Ok(())
        })
    }
}
