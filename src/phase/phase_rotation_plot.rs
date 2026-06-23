use crate::config::Plot;
use crate::plot::{decimate_1d, decimate_3d};
use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::ffi::CString;

const PHASE_ROTATION_PLOT_PY: &str = include_str!("pytools/phase_rotation_plot.py");

#[allow(dead_code)]
pub struct PhaseRotationPlotter {}

impl PhaseRotationPlotter {
    pub fn plot(
        &self,
        plot: &Plot,
        t: &[f64],
        y: &[Vec<Vec<f64>>],
        index_arr: &[u8],
        labels: &[String],
    ) -> Result<()> {
        Python::attach(|py| {
            let code = CString::new(PHASE_ROTATION_PLOT_PY)
                .expect("phase_rotation_plot.py contains interior NUL");
            let filename = CString::new("phase_rotation_plot.py").unwrap();
            let modulename = CString::new("phase_rotation_plot").unwrap();

            let plot_mod = PyModule::from_code(
                py,
                code.as_c_str(),
                filename.as_c_str(),
                modulename.as_c_str(),
            )
            .context("failed to load phase_rotation_plot.py")?;

            let np = py.import("numpy").context("failed to import numpy")?;
            let t_plot = decimate_1d(plot, t);
            let y_plot = decimate_3d(plot, y);
            let t_obj = np.call_method1("array", (t_plot,))?;
            let y_obj = np.call_method1("array", (y_plot,))?;

            let plotter = plot_mod
                .getattr("PhaseRotationPlotter")?
                .call0()
                .context("failed to create PhaseRotationPlotter instance")?;

            plotter
                .call_method1(
                    "plot",
                    (t_obj, y_obj, index_arr, labels, plot.save, plot.interactive),
                )
                .context("python PhaseRotationPlotter.plot(...) failed")?;

            Ok(())
        })
    }
}
