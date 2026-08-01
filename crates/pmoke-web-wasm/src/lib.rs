use std::f64::consts::TAU;
use wasm_bindgen::prelude::*;

const MIN_SAMPLES: usize = 64;
const MAX_SAMPLES: usize = 4_096;

/// Generate interleaved time, source, in-phase, and quadrature preview samples.
#[wasm_bindgen]
pub fn generate_signal(requested_samples: usize, phase: f64) -> Box<[f64]> {
    let samples = requested_samples.clamp(MIN_SAMPLES, MAX_SAMPLES);
    let mut output = Vec::with_capacity(samples * 4);

    for index in 0..samples {
        let t = index as f64 / (samples - 1) as f64;
        let carrier = TAU * (7.0 * t + phase);
        let envelope = (-3.2 * (t - 0.52).powi(2)).exp();
        let pulse = (carrier.sin() + 0.18 * (3.0 * carrier + 0.4).sin()) * envelope;
        let in_phase = (TAU * 1.35 * t).sin() * 0.72 + 0.12 * pulse;
        let quadrature = (TAU * 1.35 * t + 1.12).sin() * 0.48;
        output.extend_from_slice(&[t, pulse, in_phase, quadrature]);
    }

    output.into_boxed_slice()
}

#[wasm_bindgen]
pub fn validate_config_toml(toml_str: &str) -> String {
    pmoke_config_core::validate_config_toml_json(toml_str)
}

#[wasm_bindgen]
pub fn build_info() -> String {
    format!("pmoke-web-wasm/{}", env!("CARGO_PKG_VERSION"))
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
            "version = 4\n[scope]\nmodel = 'dsox1204a'\nconnection = 'usbtmc://0x1/0x2/0x3'\n[data]\noutput = 'both'\ninput = 'fetch'\n[lockin]\nsignal_channels = [1]\nworkers = 4\nstride_samples = 1\nfilter = { kind = 'boxcar_legacy', half_window_cycles = 1.0 }",
        );
        assert!(json.contains("\"valid\": true"), "json: {json}");
    }
}
