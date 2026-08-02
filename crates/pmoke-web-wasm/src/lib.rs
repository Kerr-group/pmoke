use std::f64::consts::TAU;
use wasm_bindgen::prelude::*;

const MIN_SAMPLES: usize = 64;
const MAX_SAMPLES: usize = 4_096;
const LOCKIN_HEADER_VALUES: usize = 8;

/// Compute raw channel values at continuous time t in [0, 1].
pub fn sample_channels(t: f64, phase: f64) -> (f64, f64, f64) {
    let carrier = TAU * (7.0 * t + phase);
    let envelope = (std::f64::consts::PI * (t - 0.5)).cos().powi(2);
    let pulse = (carrier.sin() + 0.18 * (3.0 * carrier + 0.4).sin()) * envelope;
    let in_phase = (TAU * 2.0 * t).sin() * 0.72 + 0.12 * pulse;
    let quadrature = (TAU * 2.0 * t + 1.12).sin() * 0.48;
    (pulse, in_phase, quadrature)
}

/// Generate interleaved time, source, in-phase, and quadrature preview samples with C1 periodic closure.
#[wasm_bindgen]
pub fn generate_signal(requested_samples: usize, phase: f64) -> Box<[f64]> {
    let samples = requested_samples.clamp(MIN_SAMPLES, MAX_SAMPLES);
    let mut output = Vec::with_capacity(samples * 4);

    for index in 0..samples {
        let t = index as f64 / (samples - 1) as f64;
        let (pulse, in_phase, quadrature) = sample_channels(t, phase);
        output.extend_from_slice(&[t, pulse, in_phase, quadrature]);
    }

    output.into_boxed_slice()
}

#[wasm_bindgen]
pub fn validate_config_toml(toml_str: &str) -> String {
    pmoke_config_core::validate_config_toml_json(toml_str)
}

#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn generate_analysis_demo(
    samples: usize,
    sample_rate_hz: f64,
    reference_frequency_hz: f64,
    amplitude: f64,
    phase_rad: f64,
    noise_rms: f64,
    kerr_angle_rad: f64,
    seed: u32,
) -> Result<Box<[f64]>, JsError> {
    pmoke_analysis_core::generate_synthetic_signal(pmoke_analysis_core::SyntheticSignalSettings {
        samples,
        sample_rate_hz,
        reference_frequency_hz,
        amplitude,
        phase_rad,
        noise_rms,
        kerr_angle_rad,
        seed: u64::from(seed),
    })
    .map(Vec::into_boxed_slice)
    .map_err(analysis_error)
}

#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn analyze_boxcar_legacy_interleaved(
    signal: &[f64],
    start_time_s: f64,
    sample_rate_hz: f64,
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    half_window_cycles: f64,
    stride_samples: usize,
    harmonic: usize,
) -> Result<Box<[f64]>, JsError> {
    let output = pmoke_analysis_core::analyze_boxcar_legacy(
        signal,
        pmoke_analysis_core::BoxcarLegacySettings {
            start_time_s,
            sample_interval_s: 1.0 / sample_rate_hz,
            reference_frequency_hz,
            reference_phase_rad,
            half_window_cycles,
            stride_samples,
            harmonic,
        },
    )
    .map_err(analysis_error)?;
    let mut packed = Vec::with_capacity(LOCKIN_HEADER_VALUES + output.x.len() * 3);
    packed.extend_from_slice(&[
        output.metadata.output_samples as f64,
        output.metadata.sample_rate_hz,
        output.metadata.output_rate_hz,
        output.metadata.half_window_s,
        output.metadata.support_s,
        output.metadata.estimated_enbw_hz,
        output.metadata.first_input_index as f64,
        output.metadata.last_input_index as f64,
    ]);
    for ((time, x), y) in output.time_s.into_iter().zip(output.x).zip(output.y) {
        packed.extend_from_slice(&[time, x, y]);
    }
    Ok(packed.into_boxed_slice())
}

#[wasm_bindgen]
pub fn rotate_phase_interleaved(
    x: &[f64],
    y: &[f64],
    delta_rad: f64,
) -> Result<Box<[f64]>, JsError> {
    if x.len() != y.len() {
        return Err(JsError::new(
            "length_mismatch: x and y must have equal lengths",
        ));
    }
    if !delta_rad.is_finite() || x.iter().chain(y).any(|value| !value.is_finite()) {
        return Err(JsError::new(
            "non_finite_phase: x, y, and delta must be finite",
        ));
    }
    let (in_phase, out_of_phase) = pmoke_analysis_core::rotate_phase(x, y, delta_rad);
    let mut packed = Vec::with_capacity(x.len() * 2);
    for (in_phase, out_of_phase) in in_phase.into_iter().zip(out_of_phase) {
        packed.extend_from_slice(&[in_phase, out_of_phase]);
    }
    Ok(packed.into_boxed_slice())
}

#[wasm_bindgen]
pub fn calculate_harmonics_kerr_packed(
    a2: &[f64],
    a3: &[f64],
    a4: &[f64],
    a6: &[f64],
    factor: f64,
) -> Result<Box<[f64]>, JsError> {
    let output = pmoke_analysis_core::calculate_harmonics_kerr(a2, a3, a4, a6, factor)
        .map_err(analysis_error)?;
    let mut packed = Vec::with_capacity(output.values_rad.len() + 1);
    packed.push(output.representative_modulation_depth);
    packed.extend(output.values_rad);
    Ok(packed.into_boxed_slice())
}

#[wasm_bindgen]
pub fn boxcar_response_interleaved(
    half_window_s: f64,
    max_frequency_hz: f64,
    points: usize,
) -> Result<Box<[f64]>, JsError> {
    if !(16..=2_048).contains(&points) {
        return Err(JsError::new(
            "invalid_points: response points must be in 16..=2048",
        ));
    }
    if !max_frequency_hz.is_finite() || max_frequency_hz <= 0.0 {
        return Err(JsError::new(
            "invalid_frequency: max frequency must be positive and finite",
        ));
    }
    let mut output = Vec::with_capacity(points * 2);
    for index in 0..points {
        let frequency_hz = max_frequency_hz * index as f64 / (points - 1) as f64;
        let response = pmoke_analysis_core::boxcar_response_abs(half_window_s, frequency_hz)
            .map_err(analysis_error)?;
        output.extend_from_slice(&[frequency_hz, response]);
    }
    Ok(output.into_boxed_slice())
}

#[wasm_bindgen]
pub fn analysis_limits_json() -> String {
    format!(
        r#"{{"max_demo_samples":{},"max_upload_samples":{},"max_upload_bytes":{},"max_total_harmonic_points":{},"lockin_header_values":{}}}"#,
        pmoke_analysis_core::DEFAULT_MAX_DEMO_SAMPLES,
        pmoke_analysis_core::MAX_UPLOAD_SAMPLES,
        pmoke_analysis_core::MAX_UPLOAD_BYTES,
        pmoke_analysis_core::MAX_TOTAL_HARMONIC_POINTS,
        LOCKIN_HEADER_VALUES,
    )
}

#[wasm_bindgen]
pub fn build_info() -> String {
    format!(
        "pmoke-web-wasm/{}; pmoke-analysis-core/{}",
        env!("CARGO_PKG_VERSION"),
        pmoke_analysis_core::VERSION
    )
}

fn analysis_error(error: pmoke_analysis_core::AnalysisError) -> JsError {
    JsError::new(&format!("{}: {}", error.code(), error.message()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_bounded_and_finite() {
        let output = generate_signal(usize::MAX, 0.25);
        assert_eq!(output.len(), MAX_SAMPLES * 4);
        assert!(output.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn signal_has_c1_periodic_closure() {
        let phase = 0.17;
        let (p0, i0, q0) = sample_channels(0.0, phase);
        let (p1, i1, q1) = sample_channels(1.0, phase);

        assert!((p0 - p1).abs() <= 1e-9);
        assert!((i0 - i1).abs() <= 1e-9);
        assert!((q0 - q1).abs() <= 1e-9);

        // Analytic derivatives at t=0 and t=1
        // d(envelope)/dt = pi * sin(2*pi*t), so at t=0 and t=1, d(envelope)/dt = 0
        // envelope(0) = envelope(1) = 0
        // Therefore d(pulse)/dt = 0 at both endpoints!
        // d(in_phase)/dt = 2*pi*2.0*cos(0) * 0.72 + 0.12 * d(pulse)/dt
        // d(quadrature)/dt = 2*pi*2.0*cos(1.12) * 0.48
        let eps = 1e-7;
        let (_p0_plus, i0_plus, q0_plus) = sample_channels(eps, phase);
        let (_p0_minus, i0_minus, q0_minus) = sample_channels(-eps, phase);
        let (_p1_plus, i1_plus, q1_plus) = sample_channels(1.0 + eps, phase);
        let (_p1_minus, i1_minus, q1_minus) = sample_channels(1.0 - eps, phase);

        let d_i0 = (i0_plus - i0_minus) / (2.0 * eps);
        let d_i1 = (i1_plus - i1_minus) / (2.0 * eps);
        let d_q0 = (q0_plus - q0_minus) / (2.0 * eps);
        let d_q1 = (q1_plus - q1_minus) / (2.0 * eps);

        assert!((d_i0 - d_i1).abs() <= 1e-6);
        assert!((d_q0 - d_q1).abs() <= 1e-6);
    }

    #[test]
    fn output_is_deterministic() {
        assert_eq!(generate_signal(128, 0.5), generate_signal(128, 0.5));
    }

    #[test]
    fn sample_count_has_a_renderable_minimum() {
        assert_eq!(generate_signal(1, 0.0).len(), MIN_SAMPLES * 4);
    }

    #[test]
    fn wasm_config_validation_returns_valid_json() {
        let json = validate_config_toml(
            r#"version = 4
[scope]
model = "DHO5108"
connection = "tcp://192.0.2.10:55255"
[data]
output = "raw"
input = "raw"
[[sensors]]
channel = 1
scale = { factor = 1.0 }
label = "field"
unit = "T"
[pulse]
background_before = { start = -0.005, end = -0.001 }
background_after = { start = 0.01, end = 0.02 }
[reference]
channel = 2
fft_window = { start = 0.0, end = 0.005 }
stride_samples = 100
window_samples = 1000
[lockin]
signal_channels = [3]
workers = 2
stride_samples = 100
filter = { kind = "boxcar_legacy", half_window_cycles = 1.0 }
[phase]
offsets = [0, 0, 0, 0, 0, 0]
[kerr]
sensor = 1
method = "harmonics"
factor = -1.0
"#,
        );
        let report: pmoke_config_core::ValidationReport = serde_json::from_str(&json).unwrap();
        assert!(report.valid, "json: {json}");
        assert_eq!(report.summary.unwrap().signal_channels, vec![3]);
    }

    #[test]
    fn analysis_demo_and_wasm_adapter_match_shared_core() {
        let signal =
            generate_analysis_demo(20_000, 100_000.0, 1_000.0, 1.0, 0.2, 0.0, 0.01, 7).unwrap();
        let packed =
            analyze_boxcar_legacy_interleaved(&signal, 0.0, 100_000.0, 1_000.0, 0.0, 1.0, 20, 1)
                .unwrap();
        let expected = pmoke_analysis_core::analyze_boxcar_legacy(
            &signal,
            pmoke_analysis_core::BoxcarLegacySettings {
                start_time_s: 0.0,
                sample_interval_s: 1.0e-5,
                reference_frequency_hz: 1_000.0,
                reference_phase_rad: 0.0,
                half_window_cycles: 1.0,
                stride_samples: 20,
                harmonic: 1,
            },
        )
        .unwrap();
        assert_eq!(packed[0] as usize, expected.x.len());
        for (index, ((time, x), y)) in expected
            .time_s
            .iter()
            .zip(&expected.x)
            .zip(&expected.y)
            .enumerate()
        {
            let offset = LOCKIN_HEADER_VALUES + index * 3;
            assert_eq!(packed[offset], *time);
            assert_eq!(packed[offset + 1], *x);
            assert_eq!(packed[offset + 2], *y);
        }
    }

    #[test]
    fn analysis_limits_are_machine_readable() {
        let limits: serde_json::Value = serde_json::from_str(&analysis_limits_json()).unwrap();
        assert_eq!(
            limits["max_upload_samples"].as_u64(),
            Some(pmoke_analysis_core::MAX_UPLOAD_SAMPLES as u64)
        );
        assert_eq!(limits["lockin_header_values"].as_u64(), Some(8));
    }
}
