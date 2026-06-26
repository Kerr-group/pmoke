use crate::python;
use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::sync::OnceLock;

const PULSE_BG_FIT_PY: &str = include_str!("pytools/pulse_bg_fit.py");
static PULSE_BG_FIT_MODULE: OnceLock<Py<PyModule>> = OnceLock::new();

pub struct PulseBgFitter {}

impl PulseBgFitter {
    pub fn fit(&self, t: &[f64], y: &[f64]) -> Result<f64> {
        Python::attach(|py| {
            let fitter_mod = python::cached_module(
                py,
                &PULSE_BG_FIT_MODULE,
                PULSE_BG_FIT_PY,
                "pulse_bg_fit.py",
                "pulse_bg_fit",
            )
            .context("failed to load pulse_bg_fit.py")?;
            let t_obj = python::f64_array1(py, t);
            let y_obj = python::f64_array1(py, y);

            let fitter = fitter_mod
                .getattr("PulseBgFit")?
                .call0()
                .context("failed to create PulseBgFit instance")?;

            let res = fitter
                .call_method1("fit", (t_obj, y_obj))
                .context("python PulseBgFit.fit(...) failed")?;

            res.get_item("c")?.extract().map_err(Into::into)
        })
    }
}

#[derive(Debug, Clone)]
pub struct PulseIntegralCalculator {
    dt: f64,
}

impl PulseIntegralCalculator {
    pub fn new(dt: f64) -> Self {
        Self { dt }
    }

    pub fn integrate(&self, data: &[f64], c_bg: f64, coeff: f64) -> Vec<f64> {
        let n = data.len();
        if n == 0 {
            return Vec::new();
        }
        if n == 1 {
            return vec![(data[0] - c_bg) * coeff];
        }

        let h = self.dt;
        let mut out = Vec::with_capacity(n);
        out.push(0.0);

        let mut acc = 0.0;
        for i in 1..n {
            let s0 = data[i - 1] - c_bg;
            let s1 = data[i] - c_bg;
            let incr = h * (s0 + s1) * 0.5;
            acc += incr;
            out.push(acc);
        }

        if coeff != 1.0 {
            for v in &mut out {
                *v *= coeff;
            }
        }

        out
    }
}
