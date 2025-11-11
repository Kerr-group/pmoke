use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::ffi::CString;

const SENSOR_RAW_PLOT_PY: &str = include_str!("pytools/sensor_raw_plot.py");

#[allow(dead_code)]
pub struct SensorRawPlotter {}

impl SensorRawPlotter {
    pub fn plot(
        &self,
        t: &Vec<f64>,
        y: Vec<Vec<f64>>,
        index_arr: &[u8],
        c_bg_arr: &[f64],
    ) -> Result<()> {
        Python::attach(|py| {
            let code =
                CString::new(SENSOR_RAW_PLOT_PY).expect("sensor_raw_plot.py contains interior NUL");
            let filename = CString::new("sensor_raw_plot.py").unwrap();
            let modulename = CString::new("sensor_raw_plot").unwrap();

            let plot_mod = PyModule::from_code(
                py,
                code.as_c_str(),
                filename.as_c_str(),
                modulename.as_c_str(),
            )
            .context("failed to load sensor_raw_plot.py")?;

            let np = py.import("numpy").context("failed to import numpy")?;
            let t_obj = np.call_method1("array", (t,))?;
            let y_obj = np.call_method1("array", (y,))?;
            let c_bg_obj = np.call_method1("array", (c_bg_arr,))?;

            let plotter = plot_mod
                .getattr("SensorRawPlotter")?
                .call0()
                .context("failed to create SensorRawPlotter instance")?;

            plotter
                .call_method1("plot", (t_obj, y_obj, index_arr, c_bg_obj))
                .context("python SensorRawPlotter.plot(...) failed")?;

            Ok(())
        })
    }
}
