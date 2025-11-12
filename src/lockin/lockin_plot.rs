use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::ffi::CString;

const LI_PLOT_PY: &str = include_str!("pytools/lockin_plot.py");

#[allow(dead_code)]
pub struct LIPlotter {}

impl LIPlotter {
    pub fn plot(
        &self,
        t: &[f64],
        y: &[Vec<Vec<f64>>],
        index_arr: &[u8],
        labels: &[String],
    ) -> Result<()> {
        Python::attach(|py| {
            let code = CString::new(LI_PLOT_PY).expect("lockin_plot.py contains interior NUL");
            let filename = CString::new("lockin_plot.py").unwrap();
            let modulename = CString::new("lockin_plot").unwrap();

            let plot_mod = PyModule::from_code(
                py,
                code.as_c_str(),
                filename.as_c_str(),
                modulename.as_c_str(),
            )
            .context("failed to load lockin_plot.py")?;

            let np = py.import("numpy").context("failed to import numpy")?;
            let t_obj = np.call_method1("array", (t,))?;
            let y_obj = np.call_method1("array", (y,))?;

            let plotter = plot_mod
                .getattr("LIPlotter")?
                .call0()
                .context("failed to create LIPlotter instance")?;

            plotter
                .call_method1("plot", (t_obj, y_obj, index_arr, labels))
                .context("python LIPlotter.plot(...) failed")?;

            Ok(())
        })
    }
}
