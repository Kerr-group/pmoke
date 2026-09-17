//! PN-AT-020 known-truth fidelity controls (test-scoped).
//!
//! The compare lane never sees physical truth, so the known-truth fidelity
//! metrics live here and are exercised by the PN-M4 tests: injected
//! stationary harmonics, modulation sidebands, strong even harmonics and
//! rises at phase/bank boundaries are demodulated and compared against the
//! injected waveform. Gain, latency, overshoot, leakage and final
//! angle/field errors are reported at the actual output resolution; an
//! unknown physical tolerance leaves fidelity unverified and forbids a
//! scientific pass (PN-FR-028/030, PN-D-007).

/// A demodulated or injected series keyed by output center.
pub(crate) type OutputSeries = Vec<(u64, f64)>;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FidelityMetrics {
    pub gain: Option<f64>,
    /// Latency expressed in output-center coordinate units (raw samples when
    /// the output centers are raw sample indices).
    pub latency_centers: Option<f64>,
    pub latency_s: Option<f64>,
    pub overshoot_fraction: Option<f64>,
    pub leakage_rms: Option<f64>,
    pub final_value_error: Option<f64>,
    pub field_error: Option<f64>,
}

/// Explicit numeric tolerances: only a specified authority (PN-O-001) may
/// supply these; the recorded-data lane has none (PN-D-007).
#[derive(Debug, Clone, Copy)]
pub(crate) struct FidelityTolerance {
    pub gain_error_max: f64,
    pub latency_s_max: f64,
    pub overshoot_fraction_max: f64,
    pub leakage_rms_max: f64,
    pub final_value_error_max: f64,
}

fn mean(values: &[f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    Some(values.iter().sum::<f64>() / values.len() as f64)
}

/// First 50%-crossing center of one series between the initial and final
/// levels, linearly interpolated between adjacent output centers. This is an
/// output-resolution estimate: it never claims sub-sample timing precision.
fn crossing_center(series: &[(u64, f64)], initial: f64, step: f64) -> Option<f64> {
    if !step.is_finite() || step == 0.0 {
        return None;
    }
    let half = initial + 0.5 * step;
    let rising = step > 0.0;
    for window in series.windows(2) {
        let (left_center, left_value) = window[0];
        let (right_center, right_value) = window[1];
        let reached = if rising {
            left_value < half && right_value >= half
        } else {
            left_value > half && right_value <= half
        };
        if reached {
            let denominator = right_value - left_value;
            if denominator.abs() < f64::EPSILON {
                return Some(right_center as f64);
            }
            let fraction = ((half - left_value) / denominator).clamp(0.0, 1.0);
            return Some(left_center as f64 + fraction * (right_center - left_center) as f64);
        }
    }
    None
}

/// Known-truth error metrics. `initial` and `steady` are half-open center
/// ranges before and after the injected rise; both series must share the
/// same centers.
pub(crate) fn assess(
    truth: &OutputSeries,
    recovered: &OutputSeries,
    initial: (u64, u64),
    steady: (u64, u64),
    sample_interval_s: f64,
    field_factor: f64,
) -> Result<FidelityMetrics, String> {
    if truth.len() != recovered.len() || truth.is_empty() {
        return Err("truth and recovered series must pair one to one".to_string());
    }
    if truth.iter().zip(recovered.iter()).any(|(t, r)| t.0 != r.0) {
        return Err("truth and recovered series must share output centers".to_string());
    }
    let initial_truth = mean(
        &truth
            .iter()
            .filter(|(center, _)| *center >= initial.0 && *center < initial.1)
            .map(|(_, value)| *value)
            .collect::<Vec<_>>(),
    )
    .ok_or_else(|| "no truth samples in the initial window".to_string())?;
    let final_truth = mean(
        &truth
            .iter()
            .filter(|(center, _)| *center >= steady.0 && *center < steady.1)
            .map(|(_, value)| *value)
            .collect::<Vec<_>>(),
    )
    .ok_or_else(|| "no truth samples in the steady window".to_string())?;
    let final_recovered = mean(
        &recovered
            .iter()
            .filter(|(center, _)| *center >= steady.0 && *center < steady.1)
            .map(|(_, value)| *value)
            .collect::<Vec<_>>(),
    );
    let step = final_truth - initial_truth;
    let gain = match final_recovered {
        Some(recovered_mean) if final_truth.abs() > 0.0 && final_truth.is_finite() => {
            Some(recovered_mean / final_truth)
        }
        _ => None,
    };
    let truth_crossing = crossing_center(truth, initial_truth, step);
    let recovered_crossing = crossing_center(recovered, initial_truth, step);
    let latency_centers = match (truth_crossing, recovered_crossing) {
        (Some(truth_crossing), Some(recovered_crossing)) => {
            Some(recovered_crossing - truth_crossing)
        }
        _ => None,
    };
    let latency_s = latency_centers.map(|centers| centers * sample_interval_s);
    let overshoot_fraction = if step.abs() > 0.0 {
        let peak = recovered
            .iter()
            .filter(|(center, _)| *center >= steady.0)
            .map(|(_, value)| *value)
            .fold(f64::NEG_INFINITY, f64::max);
        if peak.is_finite() {
            Some((peak - final_truth) / step.abs())
        } else {
            None
        }
    } else {
        None
    };
    let leakage_rms = {
        let squares: f64 = truth
            .iter()
            .zip(recovered.iter())
            .map(|((_, truth_value), (_, recovered_value))| {
                (recovered_value - truth_value) * (recovered_value - truth_value)
            })
            .sum();
        Some((squares / truth.len() as f64).sqrt())
    };
    let final_value_error = match (truth.last(), recovered.last()) {
        (Some((_, truth_value)), Some((_, recovered_value))) => Some(recovered_value - truth_value),
        _ => None,
    };
    let field_error = final_value_error.map(|error| error * field_factor);
    Ok(FidelityMetrics {
        gain,
        latency_centers,
        latency_s,
        overshoot_fraction,
        leakage_rms,
        final_value_error,
        field_error,
    })
}

/// `pass` only when every metric is inside an explicitly specified
/// tolerance; an uncomputed metric can never pass.
pub(crate) fn verdict(metrics: &FidelityMetrics, tolerance: &FidelityTolerance) -> &'static str {
    let gain_ok = metrics
        .gain
        .is_some_and(|gain| (gain - 1.0).abs() <= tolerance.gain_error_max);
    let latency_ok = metrics
        .latency_s
        .is_some_and(|latency| latency.abs() <= tolerance.latency_s_max);
    let overshoot_ok = metrics
        .overshoot_fraction
        .is_some_and(|overshoot| overshoot.abs() <= tolerance.overshoot_fraction_max);
    let leakage_ok = metrics
        .leakage_rms
        .is_some_and(|leakage| leakage <= tolerance.leakage_rms_max);
    let final_ok = metrics
        .final_value_error
        .is_some_and(|error| error.abs() <= tolerance.final_value_error_max);
    if gain_ok && latency_ok && overshoot_ok && leakage_ok && final_ok {
        "pass"
    } else {
        "fail"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRIDE: u64 = 100;
    const RISE_START: usize = 100;
    const RISE_END: usize = 140;

    /// Injected truth: flat 1.0 until center index 100, a linear rise to 1.5
    /// over 40 output centers, then flat 1.5 (a known rise at a known
    /// output resolution).
    fn truth_series(samples: usize) -> OutputSeries {
        (0..samples)
            .map(|index| {
                let value = if index < RISE_START {
                    1.0
                } else if index < RISE_END {
                    1.0 + 0.5 * (index - RISE_START) as f64 / (RISE_END - RISE_START) as f64
                } else {
                    1.5
                };
                (index as u64 * STRIDE, value)
            })
            .collect()
    }

    fn with_noise(series: &OutputSeries, amplitude: f64) -> OutputSeries {
        series
            .iter()
            .enumerate()
            .map(|(index, (center, value))| {
                // Deterministic bounded pseudo-noise: no RNG dependency.
                let unit = ((index * 7919) % 13) as f64 / 13.0 - 0.5;
                (*center, *value + amplitude * unit)
            })
            .collect()
    }

    /// Delayed, scaled and overshooting recovery of a known truth series.
    fn disturbed_series(
        truth: &OutputSeries,
        gain: f64,
        shift: usize,
        overshoot: f64,
    ) -> OutputSeries {
        let values: Vec<f64> = truth.iter().map(|(_, value)| *value).collect();
        truth
            .iter()
            .enumerate()
            .map(|(index, (center, _))| {
                let source = values[index.saturating_sub(shift)];
                let mut value = gain * source;
                if (RISE_END + 2..=RISE_END + 6).contains(&index) {
                    value += overshoot;
                }
                (*center, value)
            })
            .collect()
    }

    fn roughness(series: &OutputSeries) -> f64 {
        let squares: f64 = series
            .windows(2)
            .map(|pair| {
                let delta = pair[1].1 - pair[0].1;
                delta * delta
            })
            .sum();
        squares.sqrt()
    }

    #[test]
    fn known_injections_recover_gain_latency_overshoot_and_field_error() {
        let truth = truth_series(200);
        let steady = (RISE_END as u64 * STRIDE, 200 * STRIDE);
        let initial = (0, RISE_START as u64 * STRIDE);
        // A delayed and overshooting recovery keeps the gain at 1: the
        // 50% crossing then moves by exactly the injected delay (three
        // output-series positions = 3 x STRIDE raw samples).
        let shifted = disturbed_series(&truth, 1.0, 3, 0.05);
        let metrics = assess(&truth, &shifted, initial, steady, 1.0e-4, 2.0).unwrap();
        let gain = metrics.gain.unwrap();
        assert!(
            (gain - 1.0).abs() < 0.01,
            "gain {gain} (the injected overshoot shifts the steady-window mean)"
        );
        let latency = metrics.latency_centers.unwrap();
        let expected = 3.0 * STRIDE as f64;
        assert!(
            (latency - expected).abs() <= STRIDE as f64,
            "latency must be recovered at output resolution (expected about {expected}), got {latency}"
        );
        assert!((metrics.latency_s.unwrap() - expected * 1.0e-4).abs() <= 1.0e-2);
        let overshoot = metrics.overshoot_fraction.unwrap();
        assert!(
            (overshoot - 0.1).abs() < 0.02,
            "overshoot fraction {overshoot}"
        );
        // A scaled recovery shows as a gain error, a final-value error and a
        // field error (field factor 2).
        let scaled = disturbed_series(&truth, 0.95, 0, 0.0);
        let metrics = assess(&truth, &scaled, initial, steady, 1.0e-4, 2.0).unwrap();
        let gain = metrics.gain.unwrap();
        assert!((gain - 0.95).abs() < 0.01, "gain {gain}");
        assert!((metrics.final_value_error.unwrap() + 0.075).abs() < 0.01);
        assert!((metrics.field_error.unwrap() + 0.15).abs() < 0.02);
        assert!(metrics.leakage_rms.unwrap() > 0.0);
    }

    #[test]
    fn mismatched_series_are_refused_instead_of_guessed() {
        let truth = truth_series(20);
        let mut recovered = truth.clone();
        recovered[5].0 += 1;
        assert!(assess(&truth, &recovered, (0, 500), (1_000, 2_000), 1.0e-4, 1.0).is_err());
        assert!(
            assess(
                &truth,
                &truth[..10].to_vec(),
                (0, 500),
                (1_000, 2_000),
                1.0e-4,
                1.0
            )
            .is_err()
        );
    }

    #[test]
    fn damaged_truth_cannot_pass_the_tolerance_even_when_the_trace_is_smoother() {
        let truth = truth_series(200);
        let noisy = with_noise(&truth, 0.02);
        // A smoother trace that damages the truth: a 9-center moving average
        // that also attenuates the plateau, so the rise is delayed and the
        // final level is wrong.
        let damaged: OutputSeries = (0..noisy.len())
            .map(|index| {
                let lo = index.saturating_sub(4);
                let hi = (index + 4).min(noisy.len() - 1);
                let values: Vec<f64> = noisy[lo..=hi]
                    .iter()
                    .map(|(_, value)| 1.0 + 0.8 * (value - 1.0))
                    .collect();
                (
                    noisy[index].0,
                    values.iter().sum::<f64>() / values.len() as f64,
                )
            })
            .collect();
        let tolerance = FidelityTolerance {
            gain_error_max: 0.02,
            latency_s_max: 1.0e-3,
            overshoot_fraction_max: 0.05,
            leakage_rms_max: 0.02,
            final_value_error_max: 0.01,
        };
        let noisy_metrics = assess(
            &truth,
            &noisy,
            (0, RISE_START as u64 * STRIDE),
            (RISE_END as u64 * STRIDE, 200 * STRIDE),
            1.0e-4,
            1.0,
        )
        .unwrap();
        let damaged_metrics = assess(
            &truth,
            &damaged,
            (0, RISE_START as u64 * STRIDE),
            (RISE_END as u64 * STRIDE, 200 * STRIDE),
            1.0e-4,
            1.0,
        )
        .unwrap();
        assert_eq!(verdict(&noisy_metrics, &tolerance), "pass");
        assert_eq!(verdict(&damaged_metrics, &tolerance), "fail");
        assert!(
            damaged_metrics.final_value_error.unwrap().abs() > 0.05,
            "attenuated truth must show as a final-value error"
        );
        assert!(
            damaged_metrics.latency_centers.unwrap() > 100.0,
            "smoothed rise must show as a latency error at output resolution"
        );
        assert!(
            roughness(&damaged) < roughness(&noisy),
            "the damaged trace is smoother; a numeric scatter improvement cannot earn a pass"
        );
    }
}
