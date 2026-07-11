use crate::python;
use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::sync::OnceLock;

#[allow(dead_code)]
const REF_ANALYSIS_PY: &str = include_str!("pytools/ref_analysis.py");
static REF_ANALYSIS_MODULE: OnceLock<Py<PyModule>> = OnceLock::new();

#[allow(dead_code)]
pub struct RefFitParams {
    pub f_ref: f64,
    pub a_ref: f64,
    pub omega_tref: f64,
}

#[allow(dead_code)]
pub struct ReferenceFFT {}

impl ReferenceFFT {
    pub fn fft(&self, dt: f64, y: &[f64]) -> Result<RefFitParams> {
        Python::attach(|py| {
            let fit_mod = python::cached_module(
                py,
                &REF_ANALYSIS_MODULE,
                REF_ANALYSIS_PY,
                "ref_analysis.py",
                "ref_analysis",
            )
            .context("failed to load ref_analysis.py")?;
            let y_obj = python::f64_array1(py, y);

            let fitter = fit_mod
                .getattr("ReferenceFFT")?
                .call0()
                .context("failed to create ReferenceFFT instance")?;

            let res = fitter
                .call_method1("fft", (dt, y_obj))
                .context("python ReferenceFFT.fft(...) failed")?;

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

#[allow(dead_code)]
pub struct ReferenceFitter {}

impl ReferenceFitter {
    pub fn fit(&self, t: &[f64], y: &[f64], params: RefFitParams) -> Result<RefFitParams> {
        Python::attach(|py| {
            let fit_mod = python::cached_module(
                py,
                &REF_ANALYSIS_MODULE,
                REF_ANALYSIS_PY,
                "ref_analysis.py",
                "ref_analysis",
            )
            .context("failed to load ref_analysis.py")?;
            let t_obj = python::f64_array1(py, t);
            let y_obj = python::f64_array1(py, y);

            let fitter = fit_mod
                .getattr("ReferenceFitter")?
                .call0()
                .context("failed to create ReferenceFitter instance")?;

            let res = fitter
                .call_method1(
                    "fit",
                    (t_obj, y_obj, params.f_ref, params.a_ref, params.omega_tref),
                )
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

#[cfg(test)]
mod tests {
    use super::ReferenceFFT;
    use std::f64::consts::PI;

    #[test]
    fn reference_fft_recovers_a_bin_centered_sine() {
        let sample_rate = 100.0e6;
        let sample_count = 4_096usize;
        let frequency = 50.0 * sample_rate / sample_count as f64;
        let amplitude = 1.75;
        let dt = 1.0 / sample_rate;
        let t: Vec<f64> = (0..sample_count).map(|index| index as f64 * dt).collect();
        let y: Vec<f64> = t
            .iter()
            .map(|&time| amplitude * (2.0 * PI * frequency * time).sin())
            .collect();

        let result = ReferenceFFT {}.fft(dt, &y).unwrap();
        assert!((result.f_ref - frequency).abs() < 1.0e-6);
        assert!(
            (result.a_ref - amplitude).abs() < 1.0e-7,
            "expected amplitude {amplitude}, got {}",
            result.a_ref
        );
        assert!(
            result.omega_tref.abs() < 2.0e-6,
            "expected zero phase, got {}",
            result.omega_tref
        );
    }
}
