use std::f64::consts::TAU;
use std::mem::{size_of, size_of_val};
use wasm_bindgen::prelude::*;

const MIN_SAMPLES: usize = 64;
const MAX_SAMPLES: usize = 4_096;
const LOCKIN_HEADER_VALUES: usize = 8;
/// WASM joint-window caps (NFR-008): at most 2047 window samples and 25
/// design parameters (1 + 2 * fit harmonics) per call.
pub const MAX_JOINT_WINDOW_SAMPLES: usize = 2_047;
pub const MAX_JOINT_PARAMETERS: usize = 25;
/// WASM joint-model byte cap per channel (NFR-008/AT-029): 1 MiB across
/// all supplied model components (variance table, correlation lags,
/// harmonic lists), enforced before any cloning or solving.
pub const MAX_JOINT_MODEL_BYTES: usize = 1_048_576;

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
    angle_rad: f64,
    seed: u32,
) -> Result<Box<[f64]>, JsError> {
    pmoke_analysis_core::generate_synthetic_signal(pmoke_analysis_core::SyntheticSignalSettings {
        samples,
        sample_rate_hz,
        reference_frequency_hz,
        amplitude,
        phase_rad,
        noise_rms,
        angle_rad,
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
    let (in_phase, out_of_phase) =
        pmoke_analysis_core::rotate_phase(x, y, delta_rad).map_err(analysis_error)?;
    let mut packed = Vec::with_capacity(x.len() * 2);
    for (in_phase, out_of_phase) in in_phase.into_iter().zip(out_of_phase) {
        packed.extend_from_slice(&[in_phase, out_of_phase]);
    }
    Ok(packed.into_boxed_slice())
}

#[wasm_bindgen]
pub fn calculate_harmonics_moke_packed(
    a2: &[f64],
    a3: &[f64],
    a4: &[f64],
    a6: &[f64],
    factor: f64,
) -> Result<Box<[f64]>, JsError> {
    let output = pmoke_analysis_core::calculate_harmonics_moke(a2, a3, a4, a6, factor)
        .map_err(analysis_error)?;
    let mut packed = Vec::with_capacity(output.values_rad.len() + 1);
    packed.push(output.representative_modulation_depth);
    packed.extend(output.values_rad);
    Ok(packed.into_boxed_slice())
}

#[wasm_bindgen]
pub fn calculate_harmonics_vm_packed(
    second: &[f64],
    third: &[f64],
    modulation_depth: f64,
) -> Result<Box<[f64]>, JsError> {
    let values = pmoke_analysis_core::calculate_harmonics_vm(second, third, modulation_depth)
        .map_err(analysis_error)?;
    Ok(values.into_boxed_slice())
}

#[wasm_bindgen]
pub fn boxcar_mean_packed(
    signal: &[f64],
    start_time_s: f64,
    sample_rate_hz: f64,
    half_window_s: f64,
    stride_samples: usize,
) -> Result<Box<[f64]>, JsError> {
    let output = pmoke_analysis_core::boxcar_mean(
        signal,
        pmoke_analysis_core::BoxcarMeanSettings {
            start_time_s,
            sample_interval_s: 1.0 / sample_rate_hz,
            half_window_s,
            stride_samples,
        },
    )
    .map_err(analysis_error)?;
    Ok(output.mean.into_boxed_slice())
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
        r#"{{"max_demo_samples":{},"max_upload_samples":{},"max_upload_bytes":{},"max_total_harmonic_points":{},"lockin_header_values":{},"max_joint_window_samples":{},"max_joint_parameters":{},"max_joint_model_bytes":{}}}"#,
        pmoke_analysis_core::DEFAULT_MAX_DEMO_SAMPLES,
        pmoke_analysis_core::MAX_UPLOAD_SAMPLES,
        pmoke_analysis_core::MAX_UPLOAD_BYTES,
        pmoke_analysis_core::MAX_TOTAL_HARMONIC_POINTS,
        LOCKIN_HEADER_VALUES,
        MAX_JOINT_WINDOW_SAMPLES,
        MAX_JOINT_PARAMETERS,
        MAX_JOINT_MODEL_BYTES,
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

/// Joint harmonic GLS estimate for one window through the shared core
/// solver (AT-028 parity: the same `estimate_joint` the native engine
/// calls). All four noise modes are executable; unsupported combinations
/// fail explicitly with the core's typed codes.
///
/// Layout: `[xy (2 * outputs) | residual_rms | rank | condition |
/// jitter_applied_v2 | covariance diagonal (2 * outputs)]`. The covariance
/// is the peak-amplitude design-model XY covariance (FR-040).
/// `residual_rms` is the standardized per-unit-noise value (dimensionless;
/// numerically equal to the raw volts RMS for Identity noise when the
/// reference variance is 1 V^2, and dimensionless in that coincidence).
/// Native-testable core of [`analyze_joint_window_packed`]: identical logic,
/// `String` errors (JsError messages are unreadable off-wasm targets).
#[allow(clippy::too_many_arguments)]
fn joint_window_inner(
    signal: &[f64],
    start_time_s: f64,
    sample_interval_s: f64,
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    fit_harmonics: &[u32],
    output_harmonics: &[u32],
    noise_mode: &str,
    reference_variance_v2: f64,
    variance_bins: &[f64],
    correlation_lags: &[f64],
    correlation_lag_step_s: f64,
) -> Result<Vec<f64>, String> {
    use pmoke_analysis_core::{
        CorrelationKernel, HarmonicSignalModel, JointHarmonicSettings, JointSolverTolerances,
        NoiseMode, NoiseModel, estimate_joint,
    };
    if signal.len() > MAX_JOINT_WINDOW_SAMPLES {
        return Err(format!(
            "resource_limit_exceeded: joint window holds {} samples above cap {MAX_JOINT_WINDOW_SAMPLES}",
            signal.len()
        ));
    }
    if signal.len() < 3 {
        return Err("insufficient_support: joint window needs at least 3 samples".to_string());
    }
    // Model-byte preflight before any cloning or solving: a sample-count
    // cap is not a model-size cap, and unused oversized components must
    // not reach the solver either.
    let model_bytes = size_of_val(variance_bins)
        + size_of_val(correlation_lags)
        + (fit_harmonics.len() + output_harmonics.len()) * size_of::<u32>();
    if model_bytes > MAX_JOINT_MODEL_BYTES {
        return Err(format!(
            "resource_limit_exceeded: joint model holds {model_bytes} bytes above cap {MAX_JOINT_MODEL_BYTES}",
        ));
    }
    let fit: Vec<usize> = fit_harmonics
        .iter()
        .map(|harmonic| *harmonic as usize)
        .collect();
    let outputs: Vec<usize> = output_harmonics
        .iter()
        .map(|harmonic| *harmonic as usize)
        .collect();
    if 1 + 2 * fit.len() > MAX_JOINT_PARAMETERS {
        return Err(format!(
            "resource_limit_exceeded: joint design holds {} parameters above cap {MAX_JOINT_PARAMETERS}",
            1 + 2 * fit.len()
        ));
    }
    let mode = match noise_mode {
        "identity" => NoiseMode::Identity,
        "phase_diagonal" => NoiseMode::PhaseDiagonal,
        "stationary_correlated" => NoiseMode::StationaryCorrelated,
        "phase_correlated" => NoiseMode::PhaseCorrelated,
        _ => {
            return Err(format!(
                "unsupported_estimator_mode: unknown joint noise mode {noise_mode:?}"
            ));
        }
    };
    let noise = NoiseModel {
        mode,
        reference_variance_v2,
        variance_bins: if variance_bins.is_empty() {
            None
        } else {
            Some(variance_bins.to_vec())
        },
        correlation: if correlation_lags.is_empty() {
            None
        } else {
            Some(CorrelationKernel {
                lags: correlation_lags.to_vec(),
                lag_step_s: correlation_lag_step_s,
            })
        },
    };
    let times: Vec<f64> = (0..signal.len())
        .map(|index| start_time_s + index as f64 * sample_interval_s)
        .collect();
    let sample_rate_hz = 1.0 / sample_interval_s;
    let estimate = estimate_joint(
        &times,
        signal,
        reference_frequency_hz,
        reference_phase_rad,
        sample_rate_hz,
        &JointHarmonicSettings {
            model: HarmonicSignalModel {
                fit_harmonics: fit,
                output_harmonics: outputs.clone(),
                envelope_degree: 0,
            },
            noise,
            tolerances: JointSolverTolerances::default(),
        },
    )
    .map_err(|error| format!("{}: {}", error.code(), error.message()))?;
    if estimate.xy.len() != 2 * outputs.len() {
        return Err("non_finite_output: joint solver returned no quadratures".to_string());
    }
    let mut packed = Vec::with_capacity(2 * estimate.xy.len() + 4);
    packed.extend_from_slice(&estimate.xy);
    packed.extend_from_slice(&[
        estimate.residual_rms,
        estimate.rank as f64,
        estimate.condition,
        estimate.jitter_applied_v2,
    ]);
    for (index, row) in estimate.covariance_xy.iter().enumerate() {
        packed.push(row[index]);
    }
    if !packed.iter().all(|value| value.is_finite()) {
        return Err("non_finite_output: joint window produced non-finite values".to_string());
    }
    Ok(packed)
}

#[wasm_bindgen]
#[allow(clippy::too_many_arguments)]
pub fn analyze_joint_window_packed(
    signal: &[f64],
    start_time_s: f64,
    sample_interval_s: f64,
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    fit_harmonics: &[u32],
    output_harmonics: &[u32],
    noise_mode: &str,
    reference_variance_v2: f64,
    variance_bins: &[f64],
    correlation_lags: &[f64],
    correlation_lag_step_s: f64,
) -> Result<Box<[f64]>, JsError> {
    joint_window_inner(
        signal,
        start_time_s,
        sample_interval_s,
        reference_frequency_hz,
        reference_phase_rad,
        fit_harmonics,
        output_harmonics,
        noise_mode,
        reference_variance_v2,
        variance_bins,
        correlation_lags,
        correlation_lag_step_s,
    )
    .map(Vec::into_boxed_slice)
    .map_err(|message| JsError::new(&message))
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

    #[cfg(target_arch = "wasm32")]
    #[test]
    fn phase_rotation_preserves_shared_error_codes() {
        let length_error = rotate_phase_interleaved(&[1.0, 2.0], &[3.0], 0.2).unwrap_err();
        assert!(
            JsValue::from(length_error)
                .as_string()
                .unwrap()
                .starts_with("length_mismatch:")
        );

        for (x, y, delta) in [
            (vec![f64::NAN], vec![1.0], 0.2),
            (vec![1.0], vec![f64::INFINITY], 0.2),
            (vec![1.0], vec![2.0], f64::NAN),
        ] {
            let error = rotate_phase_interleaved(&x, &y, delta).unwrap_err();
            assert!(
                JsValue::from(error)
                    .as_string()
                    .unwrap()
                    .starts_with("non_finite_phase:")
            );
        }
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
            r#"version = 6
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
channels = [3]
workers = 2
stride_samples = 100
filter = { kind = "boxcar_legacy", half_window_cycles = 1.0 }
[phase]
offsets = [0, 0, 0, 0, 0, 0]
[moke]
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
    fn wasm_config_validation_accepts_v7_legacy_and_gls() {
        let header = r#"version = 7
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
channels = [3]
workers = 2
stride_samples = 100
[lockin.window]
kind = "reference_cycles"
half_window_cycles = 1.0
edge_policy = "legacy_trim"
"#;
        let footer = r#"[phase]
offsets = [0, 0, 0, 0, 0, 0]
[moke]
sensor = 1
method = "harmonics"
factor = -1.0
"#;
        let legacy = format!("{header}[lockin.estimator]\nkind = \"boxcar_legacy\"\n{footer}");
        let report: pmoke_config_core::ValidationReport =
            serde_json::from_str(&validate_config_toml(&legacy)).unwrap();
        assert!(report.valid, "json: {}", validate_config_toml(&legacy));
        assert_eq!(report.schema_version, Some(7));

        let gls = format!(
            "{header}[lockin.estimator]\nkind = \"joint_harmonic_gls\"\nfit_harmonics = [1, 2, 3, 4, 5, 6]\noutput_harmonics = [1, 2, 3, 4, 5, 6]\nnoise_mode = \"identity\"\n[[lockin.estimator.calibrations]]\nchannel = 3\npath = \"calibration/ch3.json\"\nsha256 = \"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\"\n{footer}"
        );
        let report: pmoke_config_core::ValidationReport =
            serde_json::from_str(&validate_config_toml(&gls)).unwrap();
        assert!(report.valid, "json: {}", validate_config_toml(&gls));
        assert_eq!(report.summary.unwrap().lockin_filter, "joint_harmonic_gls");
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

#[cfg(test)]
mod joint_window_tests {
    use super::*;

    const FIT: &[u32] = &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    const OUTPUTS: &[u32] = &[1, 2, 3, 4, 5, 6];

    fn tone_window() -> (Vec<f64>, f64, f64) {
        let dt = 1.0e-5;
        let f_ref = 1_000.0;
        let signal: Vec<f64> = (0..203)
            .map(|index| (TAU * f_ref * index as f64 * dt).sin())
            .collect();
        (signal, dt, f_ref)
    }

    #[test]
    fn joint_window_recovers_tone_in_all_modes() {
        let (signal, dt, f_ref) = tone_window();
        // Identity and stationary-correlated share the scalar path shape;
        // phase modes need bins; correlated modes need lags.
        let bins = vec![0.01; 16];
        let lags = vec![1.0, 0.5, 0.25];
        for (mode, bins, lags) in [
            ("identity", [].as_slice(), [].as_slice()),
            ("phase_diagonal", bins.as_slice(), [].as_slice()),
            ("stationary_correlated", [].as_slice(), lags.as_slice()),
            ("phase_correlated", bins.as_slice(), lags.as_slice()),
        ] {
            let packed = joint_window_inner(
                &signal, 0.0, dt, f_ref, 0.0, FIT, OUTPUTS, mode, 0.01, bins, lags, dt,
            )
            .unwrap();
            // 12 xy + 4 diagnostics + 12 diagonal = 28 values.
            assert_eq!(packed.len(), 28, "mode {mode}");
            assert!(packed.iter().all(|value| value.is_finite()), "mode {mode}");
            // Phase-zero unit tone: X1 is the peak amplitude 1.0.
            assert!((packed[0] - 1.0).abs() < 1e-9, "mode {mode}: {}", packed[0]);
            assert!(packed[1].abs() < 1e-9, "mode {mode}: {}", packed[1]);
            // Rank is full (25) and no jitter was needed on clean data.
            assert_eq!(packed[13], 25.0, "mode {mode}");
            assert_eq!(packed[15], 0.0, "mode {mode}");
            // Diagonal variances are positive (unknown is never zero).
            assert!(packed[16] > 0.0, "mode {mode}");
        }
    }

    #[test]
    fn joint_window_matches_direct_core_call() {
        // Parity: the packed boundary round-trips the shared core call
        // exactly (AT-028 same-core execution).
        use pmoke_analysis_core::{
            CorrelationKernel, HarmonicSignalModel, JointHarmonicSettings, JointSolverTolerances,
            NoiseMode, NoiseModel, estimate_joint,
        };
        let (signal, dt, f_ref) = tone_window();
        let times: Vec<f64> = (0..signal.len()).map(|index| index as f64 * dt).collect();
        let direct = estimate_joint(
            &times,
            &signal,
            f_ref,
            0.0,
            1.0 / dt,
            &JointHarmonicSettings {
                model: HarmonicSignalModel {
                    fit_harmonics: (1..=12).collect(),
                    output_harmonics: (1..=6).collect(),
                    envelope_degree: 0,
                },
                noise: NoiseModel {
                    mode: NoiseMode::Identity,
                    reference_variance_v2: 0.01,
                    variance_bins: None,
                    correlation: None,
                },
                tolerances: JointSolverTolerances::default(),
            },
        )
        .unwrap();
        let packed = joint_window_inner(
            &signal,
            0.0,
            dt,
            f_ref,
            0.0,
            FIT,
            OUTPUTS,
            "identity",
            0.01,
            &[],
            &[],
            dt,
        )
        .unwrap();
        assert_eq!(&packed[0..12], direct.xy.as_slice());
        assert_eq!(packed[12], direct.residual_rms);
        assert_eq!(packed[13], direct.rank as f64);
        assert_eq!(packed[14], direct.condition);
        assert_eq!(packed[15], direct.jitter_applied_v2);
        for index in 0..12 {
            assert_eq!(packed[16 + index], direct.covariance_xy[index][index]);
        }
        let _ = CorrelationKernel {
            lags: vec![1.0],
            lag_step_s: dt,
        };
    }

    #[test]
    fn joint_window_enforces_caps_and_modes() {
        let (signal, dt, f_ref) = tone_window();
        // Oversized window.
        let big = vec![0.0; MAX_JOINT_WINDOW_SAMPLES + 1];
        let error = joint_window_inner(
            &big,
            0.0,
            dt,
            f_ref,
            0.0,
            FIT,
            OUTPUTS,
            "identity",
            0.01,
            &[],
            &[],
            dt,
        )
        .unwrap_err();
        assert!(error.contains("resource_limit_exceeded"));
        // Too many parameters.
        let fit: Vec<u32> = (1..=13).collect();
        let error = joint_window_inner(
            &signal,
            0.0,
            dt,
            f_ref,
            0.0,
            &fit,
            OUTPUTS,
            "identity",
            0.01,
            &[],
            &[],
            dt,
        )
        .unwrap_err();
        assert!(error.contains("resource_limit_exceeded"));
        // Unknown mode.
        let error = joint_window_inner(
            &signal,
            0.0,
            dt,
            f_ref,
            0.0,
            FIT,
            OUTPUTS,
            "fourier",
            0.01,
            &[],
            &[],
            dt,
        )
        .unwrap_err();
        assert!(error.contains("unsupported_estimator_mode"));
        // Missing per-mode data surfaces the core's typed codes.
        let error = joint_window_inner(
            &signal,
            0.0,
            dt,
            f_ref,
            0.0,
            FIT,
            OUTPUTS,
            "phase_diagonal",
            0.01,
            &[],
            &[],
            dt,
        )
        .unwrap_err();
        assert!(error.contains("missing_noise_component"));
        // Degenerate window.
        let error = joint_window_inner(
            &[0.0, 0.0],
            0.0,
            dt,
            f_ref,
            0.0,
            FIT,
            OUTPUTS,
            "identity",
            0.01,
            &[],
            &[],
            dt,
        )
        .unwrap_err();
        assert!(error.contains("insufficient_support"));
    }

    #[test]
    fn joint_limits_are_advertised() {
        let limits = analysis_limits_json();
        assert!(limits.contains("\"max_joint_window_samples\":2047"));
        assert!(limits.contains("\"max_joint_parameters\":25"));
        assert!(limits.contains("\"max_joint_model_bytes\":1048576"));
    }

    #[test]
    fn joint_window_enforces_model_byte_cap() {
        let (signal, dt, f_ref) = tone_window();
        let harmonic_bytes = (FIT.len() + OUTPUTS.len()) * size_of::<u32>();
        // Exactly at the 1 MiB cap: variance table sized so the total of
        // all components equals the cap.
        let at_cap = vec![1.0; (MAX_JOINT_MODEL_BYTES - harmonic_bytes) / size_of::<f64>()];
        // Exactly at the cap succeeds: the cap is inclusive.
        let packed = joint_window_inner(
            &signal,
            0.0,
            dt,
            f_ref,
            0.0,
            FIT,
            OUTPUTS,
            "phase_diagonal",
            0.01,
            &at_cap,
            &[],
            dt,
        )
        .unwrap();
        assert!(!packed.is_empty());
        // One entry over the cap.
        let over_cap = vec![1.0; (MAX_JOINT_MODEL_BYTES - harmonic_bytes) / size_of::<f64>() + 1];
        let error = joint_window_inner(
            &signal,
            0.0,
            dt,
            f_ref,
            0.0,
            FIT,
            OUTPUTS,
            "phase_diagonal",
            0.01,
            &over_cap,
            &[],
            dt,
        )
        .unwrap_err();
        assert!(error.contains("resource_limit_exceeded"), "{error}");
        // The review repro: 150,000 f64 entries (1.2 MB) rejected.
        let review_case = vec![1.0; 150_000];
        let error = joint_window_inner(
            &signal,
            0.0,
            dt,
            f_ref,
            0.0,
            FIT,
            OUTPUTS,
            "phase_diagonal",
            0.01,
            &review_case,
            &[],
            dt,
        )
        .unwrap_err();
        assert!(error.contains("resource_limit_exceeded"), "{error}");
        // Oversized components are rejected even when the mode ignores
        // them: identity mode with a huge unused variance table.
        let error = joint_window_inner(
            &signal,
            0.0,
            dt,
            f_ref,
            0.0,
            FIT,
            OUTPUTS,
            "identity",
            0.01,
            &over_cap,
            &[],
            dt,
        )
        .unwrap_err();
        assert!(error.contains("resource_limit_exceeded"), "{error}");
    }
}
