use anyhow::{Context, Result};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::ffi::CString;

const PULSE_BG_FIT_PY: &str = include_str!("pytools/pulse_bg_fit.py");

pub struct PulseBgFitter {}

impl PulseBgFitter {
    pub fn fit(&self, t: &[f64], y: &[f64]) -> Result<f64> {
        Python::attach(|py| {
            let code =
                CString::new(PULSE_BG_FIT_PY).expect("pulse_bg_fit.py contains interior NUL");
            let filename = CString::new("pulse_bg_fit.py").unwrap();
            let modulename = CString::new("pulse_bg_fit").unwrap();

            let fitter_mod = PyModule::from_code(
                py,
                code.as_c_str(),
                filename.as_c_str(),
                modulename.as_c_str(),
            )
            .context("failed to load pulse_bg_fit.py")?;

            let np = py.import("numpy").context("failed to import numpy")?;
            let t_obj = np.call_method1("array", (t,))?;
            let y_obj = np.call_method1("array", (y,))?;

            let plotter = fitter_mod
                .getattr("PulseBgFit")?
                .call0()
                .context("failed to create PulseBgFit instance")?;

            let res = plotter
                .call_method1("fit", (t_obj, y_obj))
                .context("python PulseBgFit.fit(...) failed")?;

            let c: f64 = res.get_item("c")?.extract()?;

            Ok(c)
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
