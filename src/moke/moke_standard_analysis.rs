use super::MokeChannelOutput;
use crate::config::Plot;
use crate::python;
use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::path::Path;
use std::sync::OnceLock;

#[allow(dead_code)]
const MOKE_STANDARD_ANALYSIS_PY: &str = include_str!("pytools/moke_standard_analysis.py");
static MOKE_STANDARD_ANALYSIS_MODULE: OnceLock<Py<PyModule>> = OnceLock::new();

#[allow(dead_code)]
pub struct MokeStandardAnalyser {}

pub struct MokeStandardAnalysisInput<'a> {
    pub plot: &'a Plot,
    pub t: &'a [f64],
    pub x: &'a [f64],
    pub ys: &'a [Vec<f64>],
    pub factor: f64,
    pub xlabel: &'a String,
    pub fig_name: String,
    pub output_path: &'a Path,
    pub vm_output_path: &'a Path,
}

impl MokeStandardAnalyser {
    pub fn analyse(&self, input: MokeStandardAnalysisInput<'_>) -> Result<MokeChannelOutput> {
        let output = crate::plot::prepare_plot_output(input.plot, input.output_path)?;
        let vm_output = crate::plot::prepare_plot_output(input.plot, input.vm_output_path)?;
        Python::attach(|py| {
            let analysis_mod = python::cached_module(
                py,
                &MOKE_STANDARD_ANALYSIS_MODULE,
                MOKE_STANDARD_ANALYSIS_PY,
                "moke_standard_analysis.py",
                "moke_standard_analysis",
            )
            .context("failed to load moke_standard_analysis.py")?;
            let t_obj = python::f64_array1(py, input.t);
            let x_obj = python::f64_array1(py, input.x);
            let ys_obj = python::f64_array2(py, input.ys)?;
            let output_string = output.map(|path| path.to_string_lossy().into_owned());
            let vm_output_string = vm_output.map(|path| path.to_string_lossy().into_owned());

            let analyser = analysis_mod
                .getattr("MokeStandardAnalyser")?
                .call0()
                .context("failed to create MokeStandardAnalyser instance")?;

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
                        output_string.is_some(),
                        input.plot.interactive && input.plot.enabled,
                        output_string,
                        vm_output_string,
                        input.plot.max_points,
                        input.plot.decimation.as_str(),
                    ),
                )
                .context("python MokeStandardAnalyser.analyse(...) failed")?;

            let angle = python::extract_f64_array1(&res.get_item("angle")?)?;
            let vm = python::extract_f64_array1(&res.get_item("vm")?)?;
            let plot_error: Option<String> = res.get_item("plot_error")?.extract()?;
            crate::plot::finish_embedded_plot(input.plot, output, plot_error, "Moke standard")?;

            Ok(MokeChannelOutput { angle, vm })
        })
    }
}
