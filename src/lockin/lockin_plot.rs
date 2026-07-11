use crate::config::Plot;
use crate::plot::decimate_xy_3d;
use crate::python;
use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::sync::OnceLock;

const LI_PLOT_PY: &str = include_str!("pytools/lockin_plot.py");
static LI_PLOT_MODULE: OnceLock<Py<PyModule>> = OnceLock::new();

#[allow(dead_code)]
pub struct LIPlotter {}

impl LIPlotter {
    pub fn plot(
        &self,
        plot: &Plot,
        t: &[f64],
        y: &[Vec<Vec<f64>>],
        index_arr: &[u8],
        labels: &[String],
    ) -> Result<()> {
        Python::attach(|py| {
            let plot_mod = python::cached_module(
                py,
                &LI_PLOT_MODULE,
                LI_PLOT_PY,
                "lockin_plot.py",
                "lockin_plot",
            )
            .context("failed to load lockin_plot.py")?;
            let (t_plot, y_plot) = decimate_xy_3d(plot, t, y);
            let t_obj = python::f64_array1(py, &t_plot);
            let y_obj = python::f64_array3(py, &y_plot)?;

            let plotter = plot_mod
                .getattr("LIPlotter")?
                .call0()
                .context("failed to create LIPlotter instance")?;

            plotter
                .call_method1(
                    "plot",
                    (
                        t_obj,
                        y_obj,
                        index_arr,
                        labels,
                        plot.save,
                        plot.interactive,
                        &plot.output_dir,
                    ),
                )
                .context("python LIPlotter.plot(...) failed")?;

            Ok(())
        })
    }
}
