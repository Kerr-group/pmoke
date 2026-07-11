use crate::config::Plot;
use crate::python;
use anyhow::{Context, Result, bail};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::sync::OnceLock;

#[allow(dead_code)]
const KERR_HARMONICS_ANALYSIS_PY: &str = include_str!("pytools/kerr_harmonics_analysis.py");
static KERR_HARMONICS_ANALYSIS_MODULE: OnceLock<Py<PyModule>> = OnceLock::new();

#[allow(dead_code)]
pub struct KerrHarmonicsAnalyser {}

pub struct KerrHarmonicsAnalysisInput<'a> {
    pub plot: &'a Plot,
    pub t: &'a [f64],
    pub x: &'a [f64],
    pub ys: &'a [Vec<f64>],
    pub factor: f64,
    pub xlabel: &'a String,
    pub fig_name: String,
}

impl KerrHarmonicsAnalyser {
    pub fn analyse(&self, input: KerrHarmonicsAnalysisInput<'_>) -> Result<Vec<f64>> {
        Python::attach(|py| {
            let analysis_mod = python::cached_module(
                py,
                &KERR_HARMONICS_ANALYSIS_MODULE,
                KERR_HARMONICS_ANALYSIS_PY,
                "kerr_harmonics_analysis.py",
                "kerr_harmonics_analysis",
            )
            .context("failed to load kerr_harmonics_analysis.py")?;
            let t_obj = python::f64_array1(py, input.t);
            let x_obj = python::f64_array1(py, input.x);
            let ys_obj = python::f64_array2(py, input.ys)?;

            let analyser = analysis_mod
                .getattr("KerrHarmonicsAnalyser")?
                .call0()
                .context("failed to create KerrHarmonicsAnalyser instance")?;

            let res = analyser
                .call_method1(
                    "analyse",
                    (
                        t_obj,
                        x_obj,
                        ys_obj,
                        input.factor,
                        input.xlabel,
                        input.fig_name,
                        input.plot.save && input.plot.enabled,
                        input.plot.interactive && input.plot.enabled,
                        &input.plot.output_dir,
                        input.plot.max_points,
                        input.plot.decimation.as_str(),
                    ),
                )
                .context("python KerrHarmonicsAnalyser.analyse(...) failed")?;

            let kerr = python::extract_f64_array1(&res.get_item("kerr")?)?;
            let plot_error: Option<String> = res.get_item("plot_error")?.extract()?;
            if let Some(plot_error) = plot_error {
                if input.plot.fail_on_error {
                    bail!("failed to plot Kerr harmonics analysis: {plot_error}");
                }
                crate::ui::warn(format!("Kerr harmonics plot skipped: {plot_error}"));
            }

            Ok(kerr)
        })
    }
}
