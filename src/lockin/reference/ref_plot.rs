use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::ffi::CString;

const REF_PLOT_PY: &str = include_str!("pytools/ref_plot.py");

#[allow(dead_code)]
pub struct ReferencePlotter {}

impl ReferencePlotter {
    pub fn plot(&self, t: &[f64], y: &[f64], fit: &[f64]) -> Result<()> {
        Python::attach(|py| {
            let code = CString::new(REF_PLOT_PY).expect("ref_plot.py contains interior NUL");
            let filename = CString::new("ref_plot.py").unwrap();
            let modulename = CString::new("ref_plot").unwrap();

            let plot_mod = PyModule::from_code(
                py,
                code.as_c_str(),
                filename.as_c_str(),
                modulename.as_c_str(),
            )
            .context("failed to load ref_plot.py")?;

            let np = py.import("numpy").context("failed to import numpy")?;
            let t_obj = np.call_method1("array", (t,))?;
            let y_obj = np.call_method1("array", (y,))?;
            let fit_obj = np.call_method1("array", (fit,))?;

            let plotter = plot_mod
                .getattr("ReferencePlotter")?
                .call0()
                .context("failed to create ReferencePlotter instance")?;

            plotter
                .call_method1("plot", (t_obj, y_obj, fit_obj))
                .context("python ReferencePlotter.plot(...) failed")?;

            Ok(())
        })
    }
}
