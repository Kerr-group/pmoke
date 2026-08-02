use crate::{AnalysisError, DEFAULT_MAX_DEMO_SAMPLES, Result};
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

const HARMONIC_COEFFICIENTS: [f64; 6] = [
    0.581_864_936_842_083_3,
    0.315_745_306_087_972_3,
    0.104_537_902_479_595_42,
    0.025_139_158_519_404_087,
    0.004_762_786_735_204_94,
    0.000_745_551_998_014_054_3,
];

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SyntheticSignalSettings {
    pub samples: usize,
    pub sample_rate_hz: f64,
    pub reference_frequency_hz: f64,
    pub amplitude: f64,
    pub phase_rad: f64,
    pub noise_rms: f64,
    pub kerr_angle_rad: f64,
    pub seed: u64,
}

pub fn generate_synthetic_signal(settings: SyntheticSignalSettings) -> Result<Vec<f64>> {
    if !(64..=DEFAULT_MAX_DEMO_SAMPLES).contains(&settings.samples) {
        return Err(AnalysisError::new(
            "invalid_sample_count",
            format!("samples must be in 64..={DEFAULT_MAX_DEMO_SAMPLES}"),
        ));
    }
    for (name, value) in [
        ("sample_rate_hz", settings.sample_rate_hz),
        ("reference_frequency_hz", settings.reference_frequency_hz),
        ("amplitude", settings.amplitude),
        ("phase_rad", settings.phase_rad),
        ("noise_rms", settings.noise_rms),
        ("kerr_angle_rad", settings.kerr_angle_rad),
    ] {
        if !value.is_finite() {
            return Err(AnalysisError::new(
                "non_finite_parameter",
                format!("{name} must be finite"),
            ));
        }
    }
    if settings.sample_rate_hz <= 0.0
        || settings.reference_frequency_hz <= 0.0
        || settings.amplitude <= 0.0
        || settings.noise_rms < 0.0
    {
        return Err(AnalysisError::new(
            "invalid_parameter",
            "rates and amplitude must be positive; noise_rms must not be negative",
        ));
    }
    if 6.0 * settings.reference_frequency_hz >= 0.5 * settings.sample_rate_hz {
        return Err(AnalysisError::new(
            "nyquist_violation",
            "the sixth reference harmonic must remain below Nyquist",
        ));
    }
    if settings.kerr_angle_rad.abs() >= std::f64::consts::FRAC_PI_4 {
        return Err(AnalysisError::new(
            "invalid_kerr_angle",
            "kerr_angle_rad magnitude must be below pi/4",
        ));
    }

    let mut random = XorShift64::new(settings.seed);
    let mut signal = Vec::with_capacity(settings.samples);
    for index in 0..settings.samples {
        let time = index as f64 / settings.sample_rate_hz;
        let mut value = 0.0;
        for (offset, coefficient) in HARMONIC_COEFFICIENTS.iter().enumerate() {
            let harmonic = offset + 1;
            let kerr_term = if harmonic.is_multiple_of(2) {
                (2.0 * settings.kerr_angle_rad).cos()
            } else {
                (2.0 * settings.kerr_angle_rad).sin()
            };
            let harmonic_amplitude = settings.amplitude * coefficient * kerr_term;
            value += 2.0
                * harmonic_amplitude
                * (TAU * harmonic as f64 * settings.reference_frequency_hz * time
                    + settings.phase_rad)
                    .sin();
        }
        let noise = settings.noise_rms
            * (random.next_signed() + random.next_signed() + random.next_signed())
            / 3.0_f64.sqrt();
        signal.push(value + noise);
    }
    Ok(signal)
}

struct XorShift64(u64);

impl XorShift64 {
    fn new(seed: u64) -> Self {
        Self(if seed == 0 {
            0x9e37_79b9_7f4a_7c15
        } else {
            seed
        })
    }

    fn next_signed(&mut self) -> f64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        let unit = (value >> 11) as f64 / ((1_u64 << 53) as f64);
        2.0 * unit - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> SyntheticSignalSettings {
        SyntheticSignalSettings {
            samples: 10_000,
            sample_rate_hz: 100_000.0,
            reference_frequency_hz: 1_000.0,
            amplitude: 1.0,
            phase_rad: 0.2,
            noise_rms: 0.01,
            kerr_angle_rad: 0.01,
            seed: 42,
        }
    }

    #[test]
    fn generation_is_deterministic_and_finite() {
        let first = generate_synthetic_signal(settings()).unwrap();
        let second = generate_synthetic_signal(settings()).unwrap();
        assert_eq!(first, second);
        assert!(first.iter().all(|value| value.is_finite()));
    }

    #[test]
    fn generation_enforces_sample_and_nyquist_limits() {
        let mut invalid = settings();
        invalid.samples = DEFAULT_MAX_DEMO_SAMPLES + 1;
        assert_eq!(
            generate_synthetic_signal(invalid).unwrap_err().code(),
            "invalid_sample_count"
        );
        invalid = settings();
        invalid.sample_rate_hz = 10_000.0;
        assert_eq!(
            generate_synthetic_signal(invalid).unwrap_err().code(),
            "nyquist_violation"
        );
    }
}
