use crate::config::Plot;
use crate::plot::{decimate_1d, decimate_2d};
use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::ffi::CString;

const SENSOR_INTEGRAL_PLOT_PY: &str = include_str!("pytools/sensor_integral_plot.py");

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
            let code = CString::new(SENSOR_INTEGRAL_PLOT_PY)
                .expect("sensor_integral_plot.py contains interior NUL");
            let filename = CString::new("sensor_integral_plot.py").unwrap();
            let modulename = CString::new("sensor_integral_plot").unwrap();

            let plot_mod = PyModule::from_code(
                py,
                code.as_c_str(),
                filename.as_c_str(),
                modulename.as_c_str(),
            )
            .context("failed to load sensor_integral_plot.py")?;

            let np = py.import("numpy").context("failed to import numpy")?;
            let t_plot = decimate_1d(plot, t);
            let y_plot = decimate_2d(plot, y);
            let t_obj = np.call_method1("array", (t_plot,))?;
            let y_obj = np.call_method1("array", (y_plot,))?;

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
