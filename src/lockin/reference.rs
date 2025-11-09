use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::ffi::CString;

const FFT_PY: &str = include_str!("pytools/fft.py");

pub fn fft() -> PyResult<()> {
    Python::attach(|py| {
        let code = CString::new(FFT_PY).expect("python code has no NUL");
        let filename = CString::new("fft.py").unwrap();
        let modulename = CString::new("fft").unwrap();

        let module = PyModule::from_code(py, &code, &filename, &modulename)?;

        let sys = py.import("sys")?;
        sys.setattr("dont_write_bytecode", true)?;
        sys.getattr("modules")?.set_item("fft", &module)?;

        let cls = module.getattr("PreciseFFT")?;

        let np = py.import("numpy")?;

        let t = np.call_method1("linspace", (-100.0_f64, 100.0_f64, 1_000_000usize))?;

        let two_pi = 2.0_f64 * std::f64::consts::PI;
        let arg = np.call_method1("multiply", (two_pi, &t))?; // 2π * t
        let y = np.call_method1("cos", (arg,))?;

        let inst = cls.call1((t, y, 5))?;

        let res = inst.call_method1("get_target_freq_component", (two_pi * 1.0_f64,))?;
        let (amp, phase): (f64, f64) = res.extract()?;

        let dc: f64 = inst.call_method0("get_dc_component")?.extract()?;

        println!("Amplitude: {amp}, Phase: {phase}, DC Component: {dc}");
        Ok(())
    })
}
