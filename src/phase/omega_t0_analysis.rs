use crate::config::Plot;
use crate::python;
use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::path::Path;
use std::sync::OnceLock;

#[allow(dead_code)]
const OT0_ANALYSIS_PY: &str = include_str!("pytools/omega_t0_analysis.py");
static OT0_ANALYSIS_MODULE: OnceLock<Py<PyModule>> = OnceLock::new();

#[allow(dead_code)]
pub struct OT0Analyser {}

impl OT0Analyser {
    pub fn analyse(&self, plot: &Plot, output_path: &Path, m_omega_t0: [&[f64]; 6]) -> Result<f64> {
        let output = crate::plot::prepare_plot_output(plot, output_path)?;
        // Heartbeat before the opaque embedded-Python step (least-squares
        // fit plus offset scatter render): on a recorded-scale grid this
        // is the longest silent span of the phase stage, so the log must
        // show where the channel waits and for how long. The call runs on
        // the pipeline thread with no locks held and a single
        // `Python::attach` (no nested attach, no GIL handoff), so a stall
        // here is render/fit cost, never a deadlock on this side.
        let samples = m_omega_t0
            .iter()
            .map(|series| series.len())
            .max()
            .unwrap_or(0);
        crate::ui::info(format!(
            "phase omega_t0 fit started over {samples} samples per harmonic"
        ));
        let started = std::time::Instant::now();
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
            let output_string = output.map(|path| path.to_string_lossy().into_owned());

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
                        output_string.is_some(),
                        plot.interactive && plot.enabled,
                        output_string,
                        plot.max_points,
                        plot.decimation.as_str(),
                    ),
                )
                .context("python OT0Analyser.analyse(...) failed")?;

            let omega_t0: f64 = res.get_item("omega_t0")?.extract()?;
            let plot_error: Option<String> = res.get_item("plot_error")?.extract()?;
            let nfev: u64 = res
                .get_item("nfev")?
                .extract()
                .context("python OT0Analyser.analyse(...) returned no fit evaluation count")?;
            crate::plot::finish_embedded_plot(plot, output, plot_error, "omega_t0 analysis")?;
            crate::ui::info(format!(
                "phase omega_t0 fit completed: omega_t0={omega_t0:.6e} nfev={nfev} ({})",
                crate::ui::fmt_duration(started.elapsed())
            ));

            Ok(omega_t0)
        })
    }
}
