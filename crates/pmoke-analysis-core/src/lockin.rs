use crate::{AnalysisError, Result};
use serde::{Deserialize, Serialize};
use std::f64::consts::TAU;

const PHASE_RESYNC_INTERVAL: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoxcarLegacySettings {
    pub start_time_s: f64,
    pub sample_interval_s: f64,
    pub reference_frequency_hz: f64,
    pub reference_phase_rad: f64,
    pub half_window_cycles: f64,
    pub stride_samples: usize,
    pub harmonic: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LockinMetadata {
    pub input_samples: usize,
    pub output_samples: usize,
    pub sample_rate_hz: f64,
    pub output_rate_hz: f64,
    pub half_window_s: f64,
    pub support_s: f64,
    pub estimated_enbw_hz: f64,
    pub first_input_index: usize,
    pub last_input_index: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoxcarLegacyOutput {
    pub time_s: Vec<f64>,
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub metadata: LockinMetadata,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoxcarLegacyPairOutput {
    pub x: Vec<f64>,
    pub y: Vec<f64>,
    pub metadata: LockinMetadata,
}

#[derive(Debug, Clone, Copy)]
pub struct FiniteSignal<'a>(&'a [f64]);

impl<'a> FiniteSignal<'a> {
    pub fn new(signal: &'a [f64]) -> Result<Self> {
        if let Some(index) = signal.iter().position(|value| !value.is_finite()) {
            return Err(AnalysisError::new(
                "non_finite_signal",
                format!("signal contains NaN or infinity at sample {index}"),
            ));
        }
        Ok(Self(signal))
    }

    pub fn as_slice(self) -> &'a [f64] {
        self.0
    }
}

pub fn analyze_boxcar_legacy(
    signal: &[f64],
    settings: BoxcarLegacySettings,
) -> Result<BoxcarLegacyOutput> {
    let pair = analyze_boxcar_legacy_pair(signal, settings)?;
    let time_s = (0..pair.metadata.output_samples)
        .map(|index| {
            let input_index = pair.metadata.first_input_index + index * settings.stride_samples;
            settings.start_time_s + input_index as f64 * settings.sample_interval_s
        })
        .collect();
    Ok(BoxcarLegacyOutput {
        time_s,
        x: pair.x,
        y: pair.y,
        metadata: pair.metadata,
    })
}

pub fn analyze_boxcar_legacy_pair(
    signal: &[f64],
    settings: BoxcarLegacySettings,
) -> Result<BoxcarLegacyPairOutput> {
    analyze_boxcar_legacy_pair_finite(FiniteSignal::new(signal)?, settings)
}

pub fn analyze_boxcar_legacy_pair_finite(
    signal: FiniteSignal<'_>,
    settings: BoxcarLegacySettings,
) -> Result<BoxcarLegacyPairOutput> {
    let signal = signal.as_slice();
    let geometry = Geometry::new(signal, settings)?;
    let output_samples = geometry.i_end - geometry.i_start + 1;
    let mut x = Vec::with_capacity(output_samples);
    let mut y = Vec::with_capacity(output_samples);
    let edge_dt =
        geometry.half_window_s - (geometry.half_window_samples as f64) * settings.sample_interval_s;
    let scale = 1.0 / (2.0 * geometry.half_window_s);

    let window_len = 2 * geometry.half_window_samples + 3;
    let first_center = geometry.i_start * settings.stride_samples;
    let last_center = geometry.i_end * settings.stride_samples;
    let raw_start = first_center - geometry.half_window_samples - 1;
    let raw_end = last_center + geometry.half_window_samples + 2;
    let mut window = RollingMixedWindow::new(window_len);
    let mut oscillator = MixedOscillator::new(settings, raw_start);
    let mut next_emit = first_center + geometry.half_window_samples + 1;

    for (offset, &sample) in signal[raw_start..raw_end].iter().enumerate() {
        let input_index = raw_start + offset;
        window.push(oscillator.mix(input_index, sample));
        if input_index != next_emit {
            continue;
        }

        debug_assert_eq!(window.len(), window_len);
        let outer_negative = window.get(0);
        let inner_negative = window.get(1);
        let inner_positive = window.get(window_len - 2);
        let outer_positive = window.get(window_len - 1);
        let integral_re = (window.sum_re()
            - outer_negative.0
            - outer_positive.0
            - 0.5 * inner_negative.0
            - 0.5 * inner_positive.0)
            * settings.sample_interval_s;
        let integral_im = (window.sum_im()
            - outer_negative.1
            - outer_positive.1
            - 0.5 * inner_negative.1
            - 0.5 * inner_positive.1)
            * settings.sample_interval_s;
        let edge_negative_re = edge_integral(
            inner_negative.0,
            outer_negative.0,
            edge_dt,
            settings.sample_interval_s,
        );
        let edge_positive_re = edge_integral(
            inner_positive.0,
            outer_positive.0,
            edge_dt,
            settings.sample_interval_s,
        );
        let edge_negative_im = edge_integral(
            inner_negative.1,
            outer_negative.1,
            edge_dt,
            settings.sample_interval_s,
        );
        let edge_positive_im = edge_integral(
            inner_positive.1,
            outer_positive.1,
            edge_dt,
            settings.sample_interval_s,
        );
        x.push(-(integral_im + edge_negative_im + edge_positive_im) * scale);
        y.push((integral_re + edge_negative_re + edge_positive_re) * scale);
        next_emit = next_emit.saturating_add(settings.stride_samples);
    }
    debug_assert_eq!(x.len(), output_samples);

    let weights = legacy_boxcar_weights(
        geometry.half_window_samples,
        geometry.half_window_s,
        settings.sample_interval_s,
    );
    let sample_rate_hz = 1.0 / settings.sample_interval_s;
    Ok(BoxcarLegacyPairOutput {
        metadata: LockinMetadata {
            input_samples: signal.len(),
            output_samples,
            sample_rate_hz,
            output_rate_hz: sample_rate_hz / settings.stride_samples as f64,
            half_window_s: geometry.half_window_s,
            support_s: 2.0 * geometry.half_window_s,
            estimated_enbw_hz: enbw_hz(&weights, sample_rate_hz),
            first_input_index: geometry.i_start * settings.stride_samples,
            last_input_index: geometry.i_end * settings.stride_samples,
        },
        x,
        y,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BoxcarMeanSettings {
    pub start_time_s: f64,
    pub sample_interval_s: f64,
    pub half_window_s: f64,
    pub stride_samples: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BoxcarMeanOutput {
    pub time_s: Vec<f64>,
    pub mean: Vec<f64>,
    pub metadata: LockinMetadata,
}

/// Trapezoidal boxcar mean over the lock-in support window.
///
/// Each output is the time average of the raw signal over
/// `[center - half_window_s, center + half_window_s]` with the same
/// trapezoidal weights and edge interpolation the lock-in uses, evaluated at
/// the same stride centers. The output grid therefore aligns sample-for-sample
/// with [`analyze_boxcar_legacy`] for identical length, stride, and window.
pub fn boxcar_mean(signal: &[f64], settings: BoxcarMeanSettings) -> Result<BoxcarMeanOutput> {
    let signal = FiniteSignal::new(signal)?.as_slice();
    require_finite("start_time_s", settings.start_time_s)?;
    require_positive_finite("sample_interval_s", settings.sample_interval_s)?;
    require_positive_finite("half_window_s", settings.half_window_s)?;
    let geometry = Geometry::from_half_window(
        signal.len(),
        settings.sample_interval_s,
        settings.half_window_s,
        settings.stride_samples,
    )?;
    let weights = legacy_boxcar_weights(
        geometry.half_window_samples,
        geometry.half_window_s,
        settings.sample_interval_s,
    );
    let window_len = weights.len();
    let output_samples = geometry.i_end - geometry.i_start + 1;
    let mut mean = Vec::with_capacity(output_samples);
    for index in geometry.i_start..=geometry.i_end {
        let center = index * settings.stride_samples;
        let raw_start = center - geometry.half_window_samples - 1;
        let window = &signal[raw_start..raw_start + window_len];
        mean.push(
            weights
                .iter()
                .zip(window.iter())
                .map(|(weight, sample)| weight * sample)
                .sum(),
        );
    }
    let time_s = (0..output_samples)
        .map(|index| {
            let input_index =
                geometry.i_start * settings.stride_samples + index * settings.stride_samples;
            settings.start_time_s + input_index as f64 * settings.sample_interval_s
        })
        .collect();
    let sample_rate_hz = 1.0 / settings.sample_interval_s;
    Ok(BoxcarMeanOutput {
        time_s,
        mean,
        metadata: LockinMetadata {
            input_samples: signal.len(),
            output_samples,
            sample_rate_hz,
            output_rate_hz: sample_rate_hz / settings.stride_samples as f64,
            half_window_s: geometry.half_window_s,
            support_s: 2.0 * geometry.half_window_s,
            estimated_enbw_hz: enbw_hz(&weights, sample_rate_hz),
            first_input_index: geometry.i_start * settings.stride_samples,
            last_input_index: geometry.i_end * settings.stride_samples,
        },
    })
}

pub fn boxcar_response_abs(half_window_s: f64, frequency_hz: f64) -> Result<f64> {
    require_positive_finite("half_window_s", half_window_s)?;
    require_nonnegative_finite("frequency_hz", frequency_hz)?;
    let argument = TAU * frequency_hz * half_window_s;
    Ok(if argument.abs() < 1.0e-12 {
        1.0
    } else {
        (argument.sin() / argument).abs()
    })
}

#[derive(Debug, Clone, Copy)]
struct Geometry {
    half_window_s: f64,
    half_window_samples: usize,
    i_start: usize,
    i_end: usize,
}

impl Geometry {
    fn new(signal: &[f64], settings: BoxcarLegacySettings) -> Result<Self> {
        if signal.len() < 2 {
            return Err(AnalysisError::new(
                "signal_too_short",
                "signal must contain at least two samples",
            ));
        }
        require_finite("start_time_s", settings.start_time_s)?;
        require_positive_finite("sample_interval_s", settings.sample_interval_s)?;
        require_positive_finite("reference_frequency_hz", settings.reference_frequency_hz)?;
        require_finite("reference_phase_rad", settings.reference_phase_rad)?;
        require_positive_finite("half_window_cycles", settings.half_window_cycles)?;
        if settings.stride_samples == 0 {
            return Err(AnalysisError::new(
                "invalid_stride",
                "stride_samples must be positive",
            ));
        }
        let half_window_s = settings.half_window_cycles / settings.reference_frequency_hz;
        Self::from_half_window(
            signal.len(),
            settings.sample_interval_s,
            half_window_s,
            settings.stride_samples,
        )
    }

    fn from_half_window(
        signal_len: usize,
        sample_interval_s: f64,
        half_window_s: f64,
        stride_samples: usize,
    ) -> Result<Self> {
        if signal_len < 2 {
            return Err(AnalysisError::new(
                "signal_too_short",
                "signal must contain at least two samples",
            ));
        }
        if stride_samples == 0 {
            return Err(AnalysisError::new(
                "invalid_stride",
                "stride_samples must be positive",
            ));
        }
        if !half_window_s.is_finite() || half_window_s < sample_interval_s {
            return Err(AnalysisError::new(
                "window_too_short",
                "half-window must be finite and at least one sample interval",
            ));
        }
        let half_window_samples = ((half_window_s / sample_interval_s).floor() as usize).max(1);
        let integration_points = ((signal_len - 1) / stride_samples) + 1;
        let i_start = 2 + (half_window_samples + 1) / stride_samples;
        let i_end = integration_points.saturating_sub(i_start);
        if i_end < i_start {
            return Err(AnalysisError::new(
                "signal_too_short",
                "signal does not contain a complete lock-in window",
            ));
        }
        let first_center = i_start * stride_samples;
        let last_center = i_end * stride_samples;
        if first_center <= half_window_samples
            || last_center
                .checked_add(half_window_samples + 1)
                .is_none_or(|index| index >= signal_len)
        {
            return Err(AnalysisError::new(
                "window_out_of_range",
                "lock-in window exceeds the signal bounds",
            ));
        }
        Ok(Self {
            half_window_s,
            half_window_samples,
            i_start,
            i_end,
        })
    }
}

struct MixedOscillator {
    phase_zero: f64,
    step_phase: f64,
    step_sin: f64,
    step_cos: f64,
    oscillator_re: f64,
    oscillator_im: f64,
}

impl MixedOscillator {
    fn new(settings: BoxcarLegacySettings, start: usize) -> Self {
        let harmonic = settings.harmonic as f64;
        let omega = TAU * settings.reference_frequency_hz;
        let step_phase = -harmonic * omega * settings.sample_interval_s;
        let (step_sin, step_cos) = step_phase.sin_cos();
        let phase_zero = -harmonic * (omega * settings.start_time_s - settings.reference_phase_rad);
        let anchor = start - start % PHASE_RESYNC_INTERVAL;
        let (mut oscillator_im, mut oscillator_re) =
            (phase_zero + anchor as f64 * step_phase).sin_cos();
        for _ in anchor..start {
            let next_re = oscillator_re * step_cos - oscillator_im * step_sin;
            let next_im = oscillator_re * step_sin + oscillator_im * step_cos;
            oscillator_re = next_re;
            oscillator_im = next_im;
        }
        Self {
            phase_zero,
            step_phase,
            step_sin,
            step_cos,
            oscillator_re,
            oscillator_im,
        }
    }

    fn mix(&mut self, input_index: usize, sample: f64) -> (f64, f64) {
        if input_index > 0 && input_index.is_multiple_of(PHASE_RESYNC_INTERVAL) {
            (self.oscillator_im, self.oscillator_re) =
                (self.phase_zero + input_index as f64 * self.step_phase).sin_cos();
        }
        let mixed = (sample * self.oscillator_re, sample * self.oscillator_im);
        let next_re = self.oscillator_re * self.step_cos - self.oscillator_im * self.step_sin;
        let next_im = self.oscillator_re * self.step_sin + self.oscillator_im * self.step_cos;
        self.oscillator_re = next_re;
        self.oscillator_im = next_im;
        mixed
    }
}

struct RollingMixedWindow {
    values: Vec<(f64, f64)>,
    cursor: usize,
    count: usize,
    sum_re: CompensatedSum,
    sum_im: CompensatedSum,
}

impl RollingMixedWindow {
    fn new(capacity: usize) -> Self {
        Self {
            values: vec![(0.0, 0.0); capacity],
            cursor: 0,
            count: 0,
            sum_re: CompensatedSum::default(),
            sum_im: CompensatedSum::default(),
        }
    }

    fn push(&mut self, value: (f64, f64)) {
        if self.count < self.values.len() {
            self.values[self.count] = value;
            self.count += 1;
        } else {
            let removed = self.values[self.cursor];
            self.sum_re.add(-removed.0);
            self.sum_im.add(-removed.1);
            self.values[self.cursor] = value;
            self.cursor = (self.cursor + 1) % self.values.len();
        }
        self.sum_re.add(value.0);
        self.sum_im.add(value.1);
    }

    fn len(&self) -> usize {
        self.count
    }

    fn get(&self, index: usize) -> (f64, f64) {
        debug_assert!(index < self.count);
        self.values[(self.cursor + index) % self.values.len()]
    }

    fn sum_re(&self) -> f64 {
        self.sum_re.value()
    }

    fn sum_im(&self) -> f64 {
        self.sum_im.value()
    }
}

#[derive(Default)]
struct CompensatedSum {
    sum: f64,
    correction: f64,
}

impl CompensatedSum {
    fn add(&mut self, value: f64) {
        let adjusted = value - self.correction;
        let next = self.sum + adjusted;
        self.correction = (next - self.sum) - adjusted;
        self.sum = next;
    }

    fn value(&self) -> f64 {
        self.sum
    }
}

fn edge_integral(y0: f64, y1: f64, edge_dt: f64, sample_interval_s: f64) -> f64 {
    let interpolated = (y1 * edge_dt + y0 * (sample_interval_s - edge_dt)) / sample_interval_s;
    edge_dt * 0.5 * (y0 + interpolated)
}

fn legacy_boxcar_weights(
    half_window_samples: usize,
    half_window_s: f64,
    sample_interval_s: f64,
) -> Vec<f64> {
    let len = 2 * half_window_samples + 3;
    let mut weights = vec![0.0; len];
    let normalization = 1.0 / (2.0 * half_window_s);
    for index in 0..(2 * half_window_samples) {
        weights[index + 1] += 0.5 * sample_interval_s * normalization;
        weights[index + 2] += 0.5 * sample_interval_s * normalization;
    }
    let edge_dt = half_window_s - (half_window_samples as f64) * sample_interval_s;
    if edge_dt > 0.0 {
        let inner = 0.5 * edge_dt * (2.0 - edge_dt / sample_interval_s) * normalization;
        let outer = 0.5 * edge_dt * (edge_dt / sample_interval_s) * normalization;
        weights[0] += outer;
        weights[1] += inner;
        weights[len - 2] += inner;
        weights[len - 1] += outer;
    }
    weights
}

fn enbw_hz(weights: &[f64], sample_rate_hz: f64) -> f64 {
    let sum = weights.iter().sum::<f64>();
    let sum_of_squares = weights.iter().map(|value| value * value).sum::<f64>();
    sample_rate_hz * sum_of_squares / (sum * sum)
}

fn require_finite(name: &str, value: f64) -> Result<()> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(AnalysisError::new(
            "non_finite_parameter",
            format!("{name} must be finite"),
        ))
    }
}

fn require_positive_finite(name: &str, value: f64) -> Result<()> {
    require_finite(name, value)?;
    if value > 0.0 {
        Ok(())
    } else {
        Err(AnalysisError::new(
            "invalid_parameter",
            format!("{name} must be positive"),
        ))
    }
}

fn require_nonnegative_finite(name: &str, value: f64) -> Result<()> {
    require_finite(name, value)?;
    if value >= 0.0 {
        Ok(())
    } else {
        Err(AnalysisError::new(
            "invalid_parameter",
            format!("{name} must not be negative"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mean_settings() -> BoxcarMeanSettings {
        BoxcarMeanSettings {
            start_time_s: -0.01,
            sample_interval_s: 1.0e-5,
            half_window_s: 1.0e-3,
            stride_samples: 20,
        }
    }

    #[test]
    fn boxcar_mean_recovers_constant_and_ramp() {
        let settings = mean_settings();
        let constant = vec![2.5; 20_000];
        let result = boxcar_mean(&constant, settings).unwrap();
        assert!(result.mean.iter().all(|value| (value - 2.5).abs() < 1.0e-9));
        let ramp = (0..20_000)
            .map(|index| settings.start_time_s + index as f64 * settings.sample_interval_s)
            .collect::<Vec<_>>();
        let result = boxcar_mean(&ramp, settings).unwrap();
        assert_eq!(result.time_s.len(), result.mean.len());
        for (time, mean) in result.time_s.iter().zip(result.mean.iter()) {
            assert!(
                (mean - time).abs() < 1.0e-9,
                "ramp mean {mean} differs from center time {time}"
            );
        }
    }

    #[test]
    fn boxcar_mean_shares_the_lockin_output_grid() {
        let legacy = settings();
        let half_window_s = legacy.half_window_cycles / legacy.reference_frequency_hz;
        let signal = (0..20_000)
            .map(|index| (index as f64 * 0.001).sin())
            .collect::<Vec<_>>();
        let lockin = analyze_boxcar_legacy(&signal, legacy).unwrap();
        let mean = boxcar_mean(
            &signal,
            BoxcarMeanSettings {
                start_time_s: legacy.start_time_s,
                sample_interval_s: legacy.sample_interval_s,
                half_window_s,
                stride_samples: legacy.stride_samples,
            },
        )
        .unwrap();
        assert_eq!(mean.time_s, lockin.time_s);
        assert_eq!(
            mean.metadata.first_input_index,
            lockin.metadata.first_input_index
        );
        assert_eq!(
            mean.metadata.last_input_index,
            lockin.metadata.last_input_index
        );
    }

    #[test]
    fn boxcar_mean_rejects_degenerate_inputs() {
        let settings = mean_settings();
        assert!(boxcar_mean(&[], settings).is_err());
        assert!(boxcar_mean(&[1.0, f64::NAN], settings).is_err());
        assert!(
            boxcar_mean(
                &[1.0; 100],
                BoxcarMeanSettings {
                    half_window_s: f64::NAN,
                    ..settings
                },
            )
            .is_err()
        );
        assert!(
            boxcar_mean(
                &[1.0; 100],
                BoxcarMeanSettings {
                    stride_samples: 0,
                    ..settings
                },
            )
            .is_err()
        );
    }

    fn settings() -> BoxcarLegacySettings {
        BoxcarLegacySettings {
            start_time_s: -0.01,
            sample_interval_s: 1.0e-5,
            reference_frequency_hz: 1_000.0,
            reference_phase_rad: 0.2,
            half_window_cycles: 1.0,
            stride_samples: 20,
            harmonic: 1,
        }
    }

    #[test]
    fn recovers_known_amplitude_and_phase() {
        let settings = settings();
        let amplitude = 0.8;
        let signal_phase = 0.3;
        let signal = (0..20_000)
            .map(|index| {
                let time = settings.start_time_s + index as f64 * settings.sample_interval_s;
                amplitude * (TAU * settings.reference_frequency_hz * time + signal_phase).sin()
            })
            .collect::<Vec<_>>();
        let result = analyze_boxcar_legacy(&signal, settings).unwrap();
        let expected_phase = signal_phase + settings.reference_phase_rad;
        let expected_x = 0.5 * amplitude * expected_phase.cos();
        let expected_y = 0.5 * amplitude * expected_phase.sin();
        assert!(
            result
                .x
                .iter()
                .all(|value| (value - expected_x).abs() < 1.0e-12)
        );
        assert!(
            result
                .y
                .iter()
                .all(|value| (value - expected_y).abs() < 1.0e-12)
        );
    }

    #[test]
    fn rejects_invalid_and_non_finite_inputs() {
        let mut invalid = settings();
        invalid.stride_samples = 0;
        assert_eq!(
            analyze_boxcar_legacy(&[0.0; 1_000], invalid)
                .unwrap_err()
                .code(),
            "invalid_stride"
        );
        assert_eq!(
            analyze_boxcar_legacy(&[0.0, f64::NAN], settings())
                .unwrap_err()
                .code(),
            "non_finite_signal"
        );
        assert_eq!(
            analyze_boxcar_legacy(&[], settings()).unwrap_err().code(),
            "signal_too_short"
        );
        assert_eq!(
            analyze_boxcar_legacy(&[0.0, f64::INFINITY], settings())
                .unwrap_err()
                .code(),
            "non_finite_signal"
        );
        let mut oversized_window = settings();
        oversized_window.half_window_cycles = 10_000.0;
        assert_eq!(
            analyze_boxcar_legacy(&[0.0; 1_000], oversized_window)
                .unwrap_err()
                .code(),
            "signal_too_short"
        );
    }

    #[test]
    fn response_has_unity_dc_and_first_null() {
        let half_window = 0.001;
        assert_eq!(boxcar_response_abs(half_window, 0.0).unwrap(), 1.0);
        assert!(boxcar_response_abs(half_window, 500.0).unwrap() < 1.0e-15);
    }

    #[test]
    fn geometry_golden_matches_independent_grid() {
        #[derive(serde::Deserialize)]
        struct Payload {
            schema_version: u32,
            cases: Vec<Case>,
        }
        #[derive(serde::Deserialize)]
        struct Case {
            name: String,
            length: usize,
            dt: f64,
            stride: usize,
            f_ref: f64,
            cycles: f64,
            t_start: f64,
            expected_error: Option<String>,
            n_half: Option<usize>,
            integration_points: Option<usize>,
            i_start: Option<usize>,
            i_end: Option<usize>,
            row_count: Option<usize>,
            first_center: Option<usize>,
            last_center: Option<usize>,
            first_time: Option<f64>,
            last_time: Option<f64>,
            first_support: Option<[usize; 2]>,
            last_support: Option<[usize; 2]>,
        }
        let payload: Payload =
            serde_json::from_str(include_str!("../tests/fixtures/joint-gls/geometry.json"))
                .unwrap();
        assert_eq!(payload.schema_version, 1);
        assert!(!payload.cases.is_empty());
        for case in &payload.cases {
            let settings = BoxcarLegacySettings {
                start_time_s: case.t_start,
                sample_interval_s: case.dt,
                reference_frequency_hz: case.f_ref,
                reference_phase_rad: 0.0,
                half_window_cycles: case.cycles,
                stride_samples: case.stride,
                harmonic: 1,
            };
            let signal = vec![0.0; case.length];
            if let Some(expected_code) = &case.expected_error {
                assert_eq!(
                    analyze_boxcar_legacy(&signal, settings).unwrap_err().code(),
                    expected_code,
                    "case {}",
                    case.name
                );
                continue;
            }
            let geometry = Geometry::new(&signal, settings).unwrap();
            let (n_half, i_start, i_end) = (
                case.n_half.unwrap(),
                case.i_start.unwrap(),
                case.i_end.unwrap(),
            );
            assert_eq!(geometry.half_window_samples, n_half, "case {}", case.name);
            assert_eq!(geometry.i_start, i_start, "case {}", case.name);
            assert_eq!(geometry.i_end, i_end, "case {}", case.name);
            let output = analyze_boxcar_legacy(&signal, settings).unwrap();
            assert_eq!(output.metadata.output_samples, case.row_count.unwrap());
            assert_eq!(
                output.metadata.first_input_index,
                case.first_center.unwrap()
            );
            assert_eq!(output.metadata.last_input_index, case.last_center.unwrap());
            let (first_center, last_center) =
                (case.first_center.unwrap(), case.last_center.unwrap());
            assert_eq!(
                case.first_support.unwrap(),
                [first_center - n_half - 1, first_center + n_half + 1]
            );
            assert_eq!(
                case.last_support.unwrap(),
                [last_center - n_half - 1, last_center + n_half + 1]
            );
            let time_tolerance = 1.0e-12;
            assert!(
                (output.time_s[0] - case.first_time.unwrap()).abs() < time_tolerance,
                "case {}",
                case.name
            );
            assert!(
                (*output.time_s.last().unwrap() - case.last_time.unwrap()).abs() < time_tolerance,
                "case {}",
                case.name
            );
            assert_eq!(
                case.integration_points.unwrap(),
                (case.length - 1) / case.stride + 1
            );
        }
    }
}
