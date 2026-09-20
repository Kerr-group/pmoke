use crate::python;
use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::sync::OnceLock;

#[allow(dead_code)]
const REF_ANALYSIS_PY: &str = include_str!("pytools/ref_analysis.py");
static REF_ANALYSIS_MODULE: OnceLock<Py<PyModule>> = OnceLock::new();

#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub struct RefFitParams {
    pub f_ref: f64,
    pub a_ref: f64,
    pub omega_tref: f64,
    /// Relative standard uncertainty of the fitted reference frequency
    /// (Issue #274): `max` of the fit standard error and the within-run
    /// segment-split probe from `fit_with_uncertainty`. `None` is
    /// explicitly unknown (legacy fit path): inference then falls back to
    /// the floor gate.
    pub f_ref_rel_uncertainty: Option<f64>,
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
                f_ref_rel_uncertainty: None,
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
                f_ref_rel_uncertainty: params.f_ref_rel_uncertainty,
            })
        })
    }

    /// Full sine fit plus the per-side relative frequency uncertainty
    /// (Issue #274): the fit standard error widened by the within-run
    /// segment-split probe. A missing or unusable `u_rel` becomes `None`
    /// (explicitly unknown); the fitted values are unaffected.
    pub fn fit_with_uncertainty(
        &self,
        t: &[f64],
        y: &[f64],
        params: RefFitParams,
    ) -> Result<RefFitParams> {
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
                    "fit_with_uncertainty",
                    (t_obj, y_obj, params.f_ref, params.a_ref, params.omega_tref),
                )
                .context("python ReferenceFitter.fit_with_uncertainty(...) failed")?;

            let f_ref: f64 = res.get_item("f_ref")?.extract()?;
            let a_ref: f64 = res.get_item("A_ref")?.extract()?;
            let omega_tref: f64 = res.get_item("omega_tref")?.extract()?;
            let f_ref_rel_uncertainty: Option<f64> = res
                .get_item("u_rel")
                .ok()
                .and_then(|value| value.extract().ok())
                .flatten();

            Ok(RefFitParams {
                f_ref,
                a_ref,
                omega_tref,
                f_ref_rel_uncertainty,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{RefFitParams, ReferenceFFT, ReferenceFitter};
    use std::f64::consts::PI;

    fn bin_centered_sine(sample_rate: f64, sample_count: usize, bins: f64) -> (f64, Vec<f64>) {
        let frequency = bins * sample_rate / sample_count as f64;
        let dt = 1.0 / sample_rate;
        let t: Vec<f64> = (0..sample_count).map(|index| index as f64 * dt).collect();
        (frequency, t)
    }

    #[test]
    fn fit_with_uncertainty_matches_plain_fit_and_reports_u() {
        // Deterministic tone plus a tiny deterministic harmonic so the
        // residuals (and hence the standard error) are nonzero.
        let sample_rate = 100.0e6;
        let sample_count = 4_096usize;
        let (frequency, t) = bin_centered_sine(sample_rate, sample_count, 50.0);
        let dt = 1.0 / sample_rate;
        let y: Vec<f64> = t
            .iter()
            .map(|&time| {
                0.5 * (2.0 * PI * frequency * time).sin()
                    + 1e-6 * (2.0 * PI * 3.0 * frequency * time).sin()
            })
            .collect();
        let seed = ReferenceFFT {}.fft(dt, &y).unwrap();
        let params = RefFitParams {
            f_ref: seed.f_ref,
            a_ref: seed.a_ref,
            omega_tref: seed.omega_tref,
            f_ref_rel_uncertainty: None,
        };
        let plain = ReferenceFitter {}.fit(&t, &y, params).unwrap();
        let with_u = ReferenceFitter {}
            .fit_with_uncertainty(&t, &y, params)
            .unwrap();
        // Identical fitted values through either entry point.
        assert!((with_u.f_ref - plain.f_ref).abs() == 0.0);
        // The deliberate harmonic perturbs the single-tone fit by ~2e-4
        // Hz; assert well above that bias, far below any tolerance scale.
        assert!((with_u.f_ref - frequency).abs() < 1.0e-3);
        // A sane positive uncertainty far below the ppb tolerance scale.
        let u = with_u
            .f_ref_rel_uncertainty
            .expect("clean synthetic fit must report u_rel");
        assert!(u.is_finite() && u > 0.0 && u < 1e-6, "u_rel = {u}");
    }

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

    fn reference_with_offset(offset: f64) -> (f64, RefFitParams) {
        let sample_rate = 100.0e6;
        let sample_count = 4_096usize;
        let frequency = 50.0 * sample_rate / sample_count as f64;
        let amplitude = 0.5;
        let dt = 1.0 / sample_rate;
        let t: Vec<f64> = (0..sample_count).map(|index| index as f64 * dt).collect();
        let y: Vec<f64> = t
            .iter()
            .map(|&time| offset + amplitude * (2.0 * PI * frequency * time).sin())
            .collect();

        (frequency, ReferenceFFT {}.fft(dt, &y).unwrap())
    }

    #[test]
    fn reference_fft_ignores_positive_dc_offset() {
        let (frequency, result) = reference_with_offset(0.30);

        assert!((result.f_ref - frequency).abs() < 1.0e-6);
        assert!((result.a_ref - 0.5).abs() < 1.0e-7);
    }

    #[test]
    fn reference_fft_ignores_negative_dc_offset() {
        let (frequency, result) = reference_with_offset(-0.30);

        assert!((result.f_ref - frequency).abs() < 1.0e-6);
        assert!((result.a_ref - 0.5).abs() < 1.0e-7);
    }

    #[test]
    fn reference_fft_rejects_constant_input() {
        let error = ReferenceFFT {}
            .fft(1.0 / 100.0e6, &vec![0.30; 4_096])
            .unwrap_err();

        assert!(format!("{error:#}").contains("no non-DC component"));
    }

    #[test]
    fn reference_fft_rejects_nonfinite_input() {
        let mut input = vec![0.0; 4_096];
        input[0] = f64::NAN;
        let error = ReferenceFFT {}.fft(1.0 / 100.0e6, &input).unwrap_err();

        assert!(format!("{error:#}").contains("requires finite samples"));
    }
}
