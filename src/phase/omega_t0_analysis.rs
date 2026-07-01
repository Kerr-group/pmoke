use crate::config::Plot;
use crate::python;
use anyhow::{Context, Result, bail};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::sync::OnceLock;

#[allow(dead_code)]
const OT0_ANALYSIS_PY: &str = include_str!("pytools/omega_t0_analysis.py");
static OT0_ANALYSIS_MODULE: OnceLock<Py<PyModule>> = OnceLock::new();

#[allow(dead_code)]
pub struct OT0Analyser {}

impl OT0Analyser {
    pub fn analyse(&self, plot: &Plot, m_omega_t0: [&[f64]; 6]) -> Result<f64> {
        Python::attach(|py| {
            let analysis_mod = python::cached_module(
                py,
                &OT0_ANALYSIS_MODULE,
                OT0_ANALYSIS_PY,
                "omega_t0_analysis.py",
                "omega_t0_analysis",
            )
            .context("failed to load omega_t0_analysis.py")?;
            let m_ot0_1_obj = python::f64_array1(py, m_omega_t0[0]);
            let m_ot0_2_obj = python::f64_array1(py, m_omega_t0[1]);
            let m_ot0_3_obj = python::f64_array1(py, m_omega_t0[2]);
            let m_ot0_4_obj = python::f64_array1(py, m_omega_t0[3]);
            let m_ot0_5_obj = python::f64_array1(py, m_omega_t0[4]);
            let m_ot0_6_obj = python::f64_array1(py, m_omega_t0[5]);

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
                        plot.save && plot.enabled,
                        plot.interactive && plot.enabled,
                        &plot.output_dir,
                        plot.max_points,
                    ),
                )
                .context("python OT0Analyser.analyse(...) failed")?;

            let omega_t0: f64 = res.get_item("omega_t0")?.extract()?;
            let plot_error: Option<String> = res.get_item("plot_error")?.extract()?;
            if let Some(plot_error) = plot_error {
                if plot.fail_on_error {
                    bail!("failed to plot omega_t0 analysis: {plot_error}");
                }
                crate::ui::warn(format!("omega_t0 plot skipped: {plot_error}"));
            }

            Ok(omega_t0)
        })
    }
}
