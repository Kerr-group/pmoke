use crate::config::Plot;
use anyhow::{Context, Result, bail};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::ffi::CString;

#[allow(dead_code)]
const KERR_STANDARD_ANALYSIS_PY: &str = include_str!("pytools/kerr_standard_analysis.py");

#[allow(dead_code)]
pub struct KerrStandardAnalyser {}

pub struct KerrStandardAnalysisInput<'a> {
    pub plot: &'a Plot,
    pub t: &'a [f64],
    pub x: &'a [f64],
    pub ys: &'a [Vec<f64>],
    pub factor: f64,
    pub xlabel: &'a String,
    pub fig_name: String,
}

impl KerrStandardAnalyser {
    pub fn analyse(&self, input: KerrStandardAnalysisInput<'_>) -> Result<Vec<f64>> {
        Python::attach(|py| {
            let code = CString::new(KERR_STANDARD_ANALYSIS_PY)
                .expect("kerr_standard_analysis.py contains interior NUL");
            let filename = CString::new("kerr_standard_analysis.py").unwrap();
            let modulename = CString::new("kerr_standard_analysis").unwrap();

            let analysis_mod = PyModule::from_code(
                py,
                code.as_c_str(),
                filename.as_c_str(),
                modulename.as_c_str(),
            )
            .context("failed to load kerr_standard_analysis.py")?;

            let np = py.import("numpy").context("failed to import numpy")?;
            let t_obj = np.call_method1("array", (input.t,))?;
            let x_obj = np.call_method1("array", (input.x,))?;
            let ys_obj = np.call_method1("array", (input.ys,))?;

            let analyser = analysis_mod
                .getattr("KerrStandardAnalyser")?
                .call0()
                .context("failed to create KerrStandardAnalyser instance")?;

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
                    ),
                )
                .context("python KerrStandardAnalyser.analyse(...) failed")?;

            let kerr: Vec<f64> = res.get_item("kerr")?.extract()?;
            let plot_error: Option<String> = res.get_item("plot_error")?.extract()?;
            if let Some(plot_error) = plot_error {
                if input.plot.fail_on_error {
                    bail!("failed to plot Kerr standard analysis: {plot_error}");
                }
                crate::ui::warn(format!("Kerr standard plot skipped: {plot_error}"));
            }

            Ok(kerr)
        })
    }
}
