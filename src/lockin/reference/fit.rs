use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::ffi::CString;

#[allow(dead_code)]
const REF_FIT_PY: &str = include_str!("pytools/ref_fit.py");

#[allow(dead_code)]
pub struct RefFitParams {
    pub f_ref: f64,
    pub a_ref: f64,
    pub omega_tref: f64,
}

#[allow(dead_code)]
pub struct ReferenceHandler {}

impl ReferenceHandler {
    pub fn fit(&self, t: &[f64], y: &[f64]) -> Result<RefFitParams> {
        Python::attach(|py| {
            let code = CString::new(REF_FIT_PY).expect("ref_fit.py contains interior NUL");
            let filename = CString::new("ref_fit.py").unwrap();
            let modulename = CString::new("ref_fit").unwrap();

            let fit_mod = PyModule::from_code(
                py,
                code.as_c_str(),
                filename.as_c_str(),
                modulename.as_c_str(),
            )
            .context("failed to load ref_fit.py")?;

            let np = py.import("numpy").context("failed to import numpy")?;
            let t_obj = np.call_method1("array", (t,))?;
            let y_obj = np.call_method1("array", (y,))?;

            let fitter = fit_mod
                .getattr("ReferenceFitter")?
                .call0()
                .context("failed to create ReferenceFitter instance")?;

            let res = fitter
                .call_method1("fit", (t_obj, y_obj))
                .context("python ReferenceFitter.fit(...) failed")?;

            let f_ref: f64 = res.get_item("f_ref")?.extract()?;
            let a_ref: f64 = res.get_item("A_ref")?.extract()?;
            let omega_tref: f64 = res.get_item("omega_tref")?.extract()?;

            Ok(RefFitParams {
                f_ref,
                a_ref,
                omega_tref,
            })
        })
    }
}
