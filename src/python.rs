use anyhow::{Context, Result};
use numpy::{PyArray1, PyArray2, PyArray3, PyArrayMethods};
use pyo3::prelude::*;
use pyo3::types::PyModule;
use std::ffi::CString;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

static RUST_TO_PYTHON_BYTES: AtomicU64 = AtomicU64::new(0);
static RUST_TO_PYTHON_NANOS: AtomicU64 = AtomicU64::new(0);
static PYTHON_TO_RUST_BYTES: AtomicU64 = AtomicU64::new(0);
static PYTHON_TO_RUST_NANOS: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct PythonTransferStats {
    pub rust_to_python_bytes: u64,
    pub rust_to_python_nanoseconds: u64,
    pub python_to_rust_bytes: u64,
    pub python_to_rust_nanoseconds: u64,
}

pub fn transfer_stats() -> PythonTransferStats {
    PythonTransferStats {
        rust_to_python_bytes: RUST_TO_PYTHON_BYTES.load(Ordering::Relaxed),
        rust_to_python_nanoseconds: RUST_TO_PYTHON_NANOS.load(Ordering::Relaxed),
        python_to_rust_bytes: PYTHON_TO_RUST_BYTES.load(Ordering::Relaxed),
        python_to_rust_nanoseconds: PYTHON_TO_RUST_NANOS.load(Ordering::Relaxed),
    }
}

#[doc(hidden)]
pub fn reset_transfer_stats() {
    RUST_TO_PYTHON_BYTES.store(0, Ordering::Relaxed);
    RUST_TO_PYTHON_NANOS.store(0, Ordering::Relaxed);
    PYTHON_TO_RUST_BYTES.store(0, Ordering::Relaxed);
    PYTHON_TO_RUST_NANOS.store(0, Ordering::Relaxed);
}

pub fn cached_module<'py>(
    py: Python<'py>,
    cache: &'static OnceLock<Py<PyModule>>,
    source: &'static str,
    filename: &'static str,
    module_name: &'static str,
) -> Result<Bound<'py, PyModule>> {
    if let Some(module) = cache.get() {
        return Ok(module.bind(py).clone());
    }

    let source = CString::new(source).expect("embedded Python source contains interior NUL");
    let filename = CString::new(filename).expect("Python filename contains interior NUL");
    let module_name = CString::new(module_name).expect("Python module name contains interior NUL");
    let module = PyModule::from_code(
        py,
        source.as_c_str(),
        filename.as_c_str(),
        module_name.as_c_str(),
    )
    .context("failed to compile embedded Python module")?
    .unbind();

    let _ = cache.set(module);
    Ok(cache
        .get()
        .expect("Python module cache must be initialized")
        .bind(py)
        .clone())
}

pub fn f64_array1<'py>(py: Python<'py>, values: &[f64]) -> Bound<'py, PyArray1<f64>> {
    let start = Instant::now();
    let array = PyArray1::from_slice(py, values);
    record_transfer(&RUST_TO_PYTHON_BYTES, values.len());
    record_elapsed(&RUST_TO_PYTHON_NANOS, start);
    array
}

pub fn f64_array2<'py>(py: Python<'py>, rows: &[Vec<f64>]) -> Result<Bound<'py, PyArray2<f64>>> {
    let start = Instant::now();
    let array = PyArray2::from_vec2(py, rows)
        .map_err(|error| anyhow::anyhow!("invalid 2D float array: {error}"))?;
    let value_count = rows
        .iter()
        .fold(0usize, |count, row| count.saturating_add(row.len()));
    record_transfer(&RUST_TO_PYTHON_BYTES, value_count);
    record_elapsed(&RUST_TO_PYTHON_NANOS, start);
    Ok(array)
}

pub fn f64_array3<'py>(
    py: Python<'py>,
    values: &[Vec<Vec<f64>>],
) -> Result<Bound<'py, PyArray3<f64>>> {
    let start = Instant::now();
    let array = PyArray3::from_vec3(py, values)
        .map_err(|error| anyhow::anyhow!("invalid 3D float array: {error}"))?;
    let value_count = values
        .iter()
        .flat_map(|channels| channels.iter())
        .fold(0usize, |count, row| count.saturating_add(row.len()));
    record_transfer(&RUST_TO_PYTHON_BYTES, value_count);
    record_elapsed(&RUST_TO_PYTHON_NANOS, start);
    Ok(array)
}

pub fn extract_f64_array1(value: &Bound<'_, PyAny>) -> Result<Vec<f64>> {
    let array = value.cast::<PyArray1<f64>>().map_err(|error| {
        anyhow::anyhow!("expected a one-dimensional NumPy float64 array: {error}")
    })?;
    let readonly = array.readonly();
    let values = readonly
        .as_slice()
        .context("expected a contiguous NumPy float64 array")?;
    let start = Instant::now();
    let output = values.to_vec();
    record_transfer(&PYTHON_TO_RUST_BYTES, output.len());
    record_elapsed(&PYTHON_TO_RUST_NANOS, start);
    Ok(output)
}

fn record_transfer(bytes: &AtomicU64, values: usize) {
    let byte_count = values.saturating_mul(std::mem::size_of::<f64>());
    bytes.fetch_add(
        u64::try_from(byte_count).unwrap_or(u64::MAX),
        Ordering::Relaxed,
    );
}

fn record_elapsed(nanoseconds: &AtomicU64, start: Instant) {
    nanoseconds.fetch_add(
        u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX),
        Ordering::Relaxed,
    );
}

#[cfg(test)]
mod tests {
    use super::{cached_module, extract_f64_array1, f64_array1, f64_array2, f64_array3};
    use numpy::{PyArrayMethods, PyUntypedArrayMethods};
    use pyo3::prelude::*;
    use pyo3::types::{PyModule, PySlice};
    use std::sync::OnceLock;

    static TEST_MODULE: OnceLock<Py<PyModule>> = OnceLock::new();

    #[test]
    fn f64_array_helpers_preserve_values_and_shape() {
        Python::attach(|py| {
            let one_dimensional = f64_array1(py, &[1.0, 2.0, 3.0]);
            assert_eq!(
                one_dimensional.readonly().as_slice().unwrap(),
                &[1.0, 2.0, 3.0]
            );

            let two_dimensional = f64_array2(py, &[vec![1.0, 2.0], vec![3.0, 4.0]]).unwrap();
            assert_eq!(two_dimensional.shape(), [2, 2]);
            assert_eq!(
                two_dimensional.readonly().as_slice().unwrap(),
                &[1.0, 2.0, 3.0, 4.0]
            );

            let three_dimensional =
                f64_array3(py, &[vec![vec![1.0, 2.0]], vec![vec![3.0, 4.0]]]).unwrap();
            assert_eq!(three_dimensional.shape(), [2, 1, 2]);

            let round_trip = extract_f64_array1(&one_dimensional.into_any()).unwrap();
            assert_eq!(round_trip, vec![1.0, 2.0, 3.0]);
        });
    }

    #[test]
    fn f64_array2_rejects_ragged_rows() {
        Python::attach(|py| {
            let result = f64_array2(py, &[vec![1.0], vec![2.0, 3.0]]);
            assert!(result.is_err());
        });
    }

    #[test]
    fn extract_f64_array1_rejects_wrong_dtype_and_non_contiguous_views() {
        Python::attach(|py| {
            let wrong_dtype = numpy::PyArray1::<f32>::from_slice(py, &[1.0, 2.0]);
            assert!(extract_f64_array1(&wrong_dtype.into_any()).is_err());

            let values = f64_array1(py, &[1.0, 2.0, 3.0, 4.0]);
            let every_other = values
                .call_method1("__getitem__", (PySlice::new(py, 0, 4, 2),))
                .unwrap();
            assert!(extract_f64_array1(&every_other).is_err());
        });
    }

    #[test]
    fn cached_module_reuses_the_compiled_module() {
        Python::attach(|py| {
            let first = cached_module(
                py,
                &TEST_MODULE,
                "value = 7",
                "test_module.py",
                "pmoke_python_cache_test",
            )
            .unwrap();
            let second = cached_module(
                py,
                &TEST_MODULE,
                "value = 7",
                "test_module.py",
                "pmoke_python_cache_test",
            )
            .unwrap();

            assert_eq!(first.as_ptr(), second.as_ptr());
            assert_eq!(first.getattr("value").unwrap().extract::<i32>().unwrap(), 7);
        });
    }
}
