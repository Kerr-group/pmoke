use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::ffi::CString;

#[allow(dead_code)]
const OT0_ANALYSIS_PY: &str = include_str!("pytools/omega_t0_analysis.py");

#[allow(dead_code)]
pub struct OT0Analyser {}

impl OT0Analyser {
    pub fn analyse(
        &self,
        m_ot0_1: &[f64],
        m_ot0_2: &[f64],
        m_ot0_3: &[f64],
        m_ot0_4: &[f64],
        m_ot0_5: &[f64],
        m_ot0_6: &[f64],
    ) -> Result<f64> {
        Python::attach(|py| {
            let code =
                CString::new(OT0_ANALYSIS_PY).expect("omega_t0_analysis.py contains interior NUL");
            let filename = CString::new("omega_t0_analysis.py").unwrap();
            let modulename = CString::new("omega_t0_analysis").unwrap();

            let analysis_mod = PyModule::from_code(
                py,
                code.as_c_str(),
                filename.as_c_str(),
                modulename.as_c_str(),
            )
            .context("failed to load omega_t0_analysis.py")?;

            let np = py.import("numpy").context("failed to import numpy")?;
            let m_ot0_1_obj = np.call_method1("array", (m_ot0_1,))?;
            let m_ot0_2_obj = np.call_method1("array", (m_ot0_2,))?;
            let m_ot0_3_obj = np.call_method1("array", (m_ot0_3,))?;
            let m_ot0_4_obj = np.call_method1("array", (m_ot0_4,))?;
            let m_ot0_5_obj = np.call_method1("array", (m_ot0_5,))?;
            let m_ot0_6_obj = np.call_method1("array", (m_ot0_6,))?;

            let analyser = analysis_mod
                .getattr("OT0Analyser")?
                .call0()
                .context("failed to create OT0Analyser instance")?;

            let res = analyser
                .call_method1(
                    "analyse",
                    (
                        m_ot0_1_obj,
                        m_ot0_2_obj,
                        m_ot0_3_obj,
                        m_ot0_4_obj,
                        m_ot0_5_obj,
                        m_ot0_6_obj,
                    ),
                )
                .context("python OT0Analyser.analyse(...) failed")?;

            let omega_t0: f64 = res.get_item("omega_t0")?.extract()?;

            Ok(omega_t0)
        })
    }
}
