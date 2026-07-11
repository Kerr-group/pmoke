use crate::config::Plot;
use crate::plot::decimate_xy_slices;
use crate::python;
use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::sync::OnceLock;

const REF_PLOT_PY: &str = include_str!("pytools/ref_plot.py");
static REF_PLOT_MODULE: OnceLock<Py<PyModule>> = OnceLock::new();

#[allow(dead_code)]
pub struct ReferencePlotter {}

impl ReferencePlotter {
    pub fn plot(&self, plot: &Plot, t: &[f64], y: &[f64], fit: &[f64]) -> Result<()> {
        Python::attach(|py| {
            let plot_mod =
                python::cached_module(py, &REF_PLOT_MODULE, REF_PLOT_PY, "ref_plot.py", "ref_plot")
                    .context("failed to load ref_plot.py")?;
            let (t_plot, mut series_plot) = decimate_xy_slices(plot, t, &[y, fit]);
            let fit_plot = series_plot.pop().unwrap_or_default();
            let y_plot = series_plot.pop().unwrap_or_default();
            let t_obj = python::f64_array1(py, &t_plot);
            let y_obj = python::f64_array1(py, &y_plot);
            let fit_obj = python::f64_array1(py, &fit_plot);

            let plotter = plot_mod
                .getattr("ReferencePlotter")?
                .call0()
                .context("failed to create ReferencePlotter instance")?;

            plotter
                .call_method1(
                    "plot",
                    (
                        t_obj,
                        y_obj,
                        fit_obj,
                        plot.save,
                        plot.interactive,
                        &plot.output_dir,
                    ),
                )
                .context("python ReferencePlotter.plot(...) failed")?;

            Ok(())
        })
    }
}
