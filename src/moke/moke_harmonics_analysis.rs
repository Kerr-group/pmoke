use crate::config::Plot;
use crate::python;
use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::path::Path;
use std::sync::OnceLock;

#[allow(dead_code)]
const MOKE_HARMONICS_ANALYSIS_PY: &str = include_str!("pytools/moke_harmonics_analysis.py");
static MOKE_HARMONICS_ANALYSIS_MODULE: OnceLock<Py<PyModule>> = OnceLock::new();

#[allow(dead_code)]
pub struct MokeHarmonicsAnalyser {}

pub struct MokeHarmonicsAnalysisInput<'a> {
    pub plot: &'a Plot,
    pub t: &'a [f64],
    pub x: &'a [f64],
    pub ys: &'a [Vec<f64>],
    pub factor: f64,
    pub xlabel: &'a String,
    pub fig_name: String,
    pub output_path: &'a Path,
}

impl MokeHarmonicsAnalyser {
    pub fn analyse(&self, input: MokeHarmonicsAnalysisInput<'_>) -> Result<Vec<f64>> {
        let output = crate::plot::prepare_plot_output(input.plot, input.output_path)?;
        let harmonic = |index: usize, label: &str| {
            input
                .ys
                .get(index)
                .with_context(|| format!("missing rotated {label} harmonic input"))
        };
        let moke = pmoke_analysis_core::calculate_harmonics_moke(
            harmonic(2, "second")?,
            harmonic(4, "third")?,
            harmonic(6, "fourth")?,
            harmonic(10, "sixth")?,
            input.factor,
        )
        .context("failed to calculate the Moke angle from harmonic components")?
        .values_rad;
        if output.is_none() && !(input.plot.enabled && input.plot.interactive) {
            return Ok(moke);
        }

        Python::attach(|py| {
            let analysis_mod = python::cached_module(
                py,
                &MOKE_HARMONICS_ANALYSIS_MODULE,
                MOKE_HARMONICS_ANALYSIS_PY,
                "moke_harmonics_analysis.py",
                "moke_harmonics_analysis",
            )
            .context("failed to load moke_harmonics_analysis.py")?;
            let t_obj = python::f64_array1(py, input.t);
            let x_obj = python::f64_array1(py, input.x);
            let moke_obj = python::f64_array1(py, &moke);
            let output_string = output.map(|path| path.to_string_lossy().into_owned());

            let analyser = analysis_mod
                .getattr("MokeHarmonicsAnalyser")?
                .call0()
                .context("failed to create MokeHarmonicsAnalyser instance")?;

            let plot_error: Option<String> = analyser
                .call_method1(
                    "plot",
                    (
                        t_obj,
                        x_obj,
                        moke_obj,
                        input.xlabel,
                        input.fig_name,
                        output_string.is_some(),
                        input.plot.interactive && input.plot.enabled,
                        output_string,
                        input.plot.max_points,
                        input.plot.decimation.as_str(),
                    ),
                )
                .context("python MokeHarmonicsAnalyser.plot(...) failed")?
                .extract()?;
            crate::plot::finish_embedded_plot(
                input.plot,
                output,
                plot_error,
                "Moke angle from harmonic components",
            )?;

            Ok(moke)
        })
    }
}
