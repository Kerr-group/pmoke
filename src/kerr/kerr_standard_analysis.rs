use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::ffi::CString;

#[allow(dead_code)]
const KERR_STANDARD_ANALYSIS_PY: &str = include_str!("pytools/kerr_standard_analysis.py");

#[allow(dead_code)]
pub struct KerrStandardAnalyser {}

impl KerrStandardAnalyser {
    pub fn analyse(
        &self,
        t: &[f64],
        x: &[f64],
        ys: &[Vec<f64>],
        factor: f64,
        xlabel: &String,
        fig_name: String,
    ) -> Result<Vec<f64>> {
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
            let t_obj = np.call_method1("array", (t,))?;
            let x_obj = np.call_method1("array", (x,))?;
            let ys_obj = np.call_method1("array", (ys,))?;

            let analyser = analysis_mod
                .getattr("KerrStandardAnalyser")?
                .call0()
                .context("failed to create KerrStandardAnalyser instance")?;

            let res = analyser
                .call_method1("analyse", (t_obj, x_obj, ys_obj, factor, xlabel, fig_name))
                .context("python KerrStandardAnalyser.analyse(...) failed")?;

            let kerr: Vec<f64> = res.get_item("kerr")?.extract()?;

            Ok(kerr)
        })
    }
}
