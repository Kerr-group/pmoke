use crate::config::{Lockin, LockinLpfKind};
use crate::lockin::lockin_params::LockinParams;
use anyhow::{Result, anyhow};
use num_complex::Complex64;
use std::f64::consts::PI;

#[derive(Clone, Copy)]
pub enum RefType {
    Sin,
    Cos,
}

pub struct LockinProcessor<'a> {
    t: &'a [f64],
    data: &'a [f64],
    omega_tref: f64,
    params: LockinParams,
    filter: Option<FilterDesign>,
}

#[derive(Debug, Clone)]
pub struct FilterDesign {
    pub taps: Vec<f64>,
    pub cutoff_hz: f64,
    pub cutoff_source: &'static str,
    pub estimated_enbw_hz: f64,
    pub legacy_boxcar_enbw_hz: Option<f64>,
    pub enbw_match_error_hz: Option<f64>,
    pub enbw_match_reachable: Option<bool>,
    pub user_cutoff_unused: bool,
    pub kaiser_beta: f64,
}

pub struct HarmonicLockinResult {
    pub li_x: Vec<f64>,
    pub li_y: Vec<f64>,
    pub mixed_signal: Option<Vec<Complex64>>,
}

impl HarmonicLockinResult {
    pub fn without_debug_data(mut self) -> Self {
        self.mixed_signal = None;
        self
    }
}

impl<'a> LockinProcessor<'a> {
    pub fn new(
        t: &'a [f64],
        data: &'a [f64],
        f_ref: f64,
        omega_tref: f64,
        lockin: &Lockin,
    ) -> Result<Self> {
        assert!(t.len() >= 2);
        assert_eq!(t.len(), data.len());

        let params = LockinParams::from_slice(t, f_ref, lockin)?;
        let filter = match params.lpf_kind {
            LockinLpfKind::FirZeroPhase | LockinLpfKind::FirBoxcarEnbw => {
                Some(design_filter(params, lockin)?)
            }
            LockinLpfKind::BoxcarLegacy => None,
        };

        Ok(Self {
            t,
            data,
            omega_tref,
            params,
            filter,
        })
    }

    fn ref_signal(&self, t: f64, harmonic: usize, ref_type: RefType) -> f64 {
        let arg = (harmonic as f64) * (self.params.omega * t - self.omega_tref);
        match ref_type {
            RefType::Sin => arg.sin(),
            RefType::Cos => arg.cos(),
        }
    }

    pub fn compute_harmonic_detailed(
        &self,
        harmonic: usize,
        include_debug_data: bool,
    ) -> HarmonicLockinResult {
        match self.params.lpf_kind {
            LockinLpfKind::FirZeroPhase | LockinLpfKind::FirBoxcarEnbw => {
                self.compute_fir_lockin(harmonic, include_debug_data)
            }
            LockinLpfKind::BoxcarLegacy => HarmonicLockinResult {
                li_x: self.compute_legacy_lockin(harmonic, RefType::Sin),
                li_y: self.compute_legacy_lockin(harmonic, RefType::Cos),
                mixed_signal: None,
            },
        }
    }

    pub fn output_times(&self) -> Vec<f64> {
        (self.params.i_start..=self.params.i_end)
            .map(|i_idx| self.t[i_idx * self.params.stride])
            .collect()
    }

    pub fn params(&self) -> LockinParams {
        self.params
    }

    pub fn filter_design(&self) -> Option<&FilterDesign> {
        self.filter.as_ref()
    }

    fn compute_fir_lockin(
        &self,
        harmonic: usize,
        include_debug_data: bool,
    ) -> HarmonicLockinResult {
        let mixed_signal = self.compute_complex_mixed_signal(harmonic);
        let filtered = self.apply_fir(&mixed_signal);

        let li_x: Vec<f64> = filtered.iter().map(|z| -z.im).collect();
        let li_y: Vec<f64> = filtered.iter().map(|z| z.re).collect();

        HarmonicLockinResult {
            li_x,
            li_y,
            mixed_signal: include_debug_data.then_some(mixed_signal),
        }
    }

    fn compute_complex_mixed_signal(&self, harmonic: usize) -> Vec<Complex64> {
        let harmonic = harmonic as f64;
        let phase0 = harmonic * (self.params.omega * self.t[0] - self.omega_tref);
        let step_phase = -harmonic * self.params.omega * self.params.dt;
        let step = Complex64::from_polar(1.0, step_phase);
        let mut osc = Complex64::from_polar(1.0, -phase0);

        let mut mixed = Vec::with_capacity(self.data.len());
        for (idx, &sample) in self.data.iter().enumerate() {
            if idx > 0 && idx % 4096 == 0 {
                let phase = -phase0 + (idx as f64) * step_phase;
                osc = Complex64::from_polar(1.0, phase);
            }

            mixed.push(sample * osc);
            osc *= step;
        }

        mixed
    }

    fn apply_fir(&self, mixed_signal: &[Complex64]) -> Vec<Complex64> {
        let filter = self
            .filter
            .as_ref()
            .expect("FIR taps must be present for FIR lock-in");
        let taps = &filter.taps;

        let i_start = self.params.i_start;
        let i_end = self.params.i_end;
        let m = if i_end >= i_start {
            i_end - i_start + 1
        } else {
            0
        };
        let mut out = Vec::with_capacity(m);

        for i_idx in i_start..=i_end {
            let i_base = i_idx * self.params.stride;
            let base_start = i_base - self.params.n_half;
            let mut acc = Complex64::new(0.0, 0.0);

            for (tap_idx, &tap) in taps.iter().enumerate() {
                let sample_idx = base_start + tap_idx;
                acc += mixed_signal[sample_idx] * tap;
            }

            out.push(acc);
        }

        out
    }

    fn compute_legacy_lockin(&self, harmonic: usize, ref_type: RefType) -> Vec<f64> {
        let mixed_signal: Vec<f64> = self
            .t
            .iter()
            .zip(self.data.iter())
            .map(|(&t, &data)| data * self.ref_signal(t, harmonic, ref_type))
            .collect();

        let i_start = self.params.i_start;
        let i_end = self.params.i_end;
        let m = if i_end >= i_start {
            i_end - i_start + 1
        } else {
            0
        };
        let mut out = Vec::with_capacity(m);

        for k in 0..m {
            let i_idx = i_start + k;
            let i_base = i_idx * self.params.stride;

            let mut integ = 0.0;
            for j in 0..(2 * self.params.n_half) {
                let j0 = j as isize - self.params.n_half as isize;
                let j1 = j0 + 1;
                let idx0 = (i_base as isize + j0) as usize;
                let idx1 = (i_base as isize + j1) as usize;

                let f0 = mixed_signal[idx0];
                let f1 = mixed_signal[idx1];

                integ += 0.5 * (f0 + f1) * self.params.dt;
            }

            let neg_idx0 = i_base - self.params.n_half;
            let neg_idx1 = i_base - self.params.n_half - 1;

            let y0_neg = mixed_signal[neg_idx0];
            let y1_neg = mixed_signal[neg_idx1];

            let edge_dt = self.params.t_half - (self.params.n_half as f64) * self.params.dt;
            let ym_neg = (y1_neg * edge_dt + y0_neg * (self.params.dt - edge_dt)) / self.params.dt;
            let edge_neg = edge_dt * 0.5 * (y0_neg + ym_neg);

            let pos_idx0 = i_base + self.params.n_half;
            let pos_idx1 = i_base + self.params.n_half + 1;

            let y0_pos = mixed_signal[pos_idx0];
            let y1_pos = mixed_signal[pos_idx1];

            let ym_pos = (y1_pos * edge_dt + y0_pos * (self.params.dt - edge_dt)) / self.params.dt;
            let edge_pos = edge_dt * 0.5 * (y0_pos + ym_pos);

            let li = (integ + edge_neg + edge_pos) / (2.0 * self.params.t_half);
            out.push(li);
        }

        out
    }
}

fn design_filter(params: LockinParams, lockin: &Lockin) -> Result<FilterDesign> {
    let beta = kaiser_beta(params.lpf_stopband_atten_db);
    match params.lpf_kind {
        LockinLpfKind::FirZeroPhase => {
            let cutoff_hz = params
                .cutoff_hz
                .ok_or_else(|| anyhow!("fir_zero_phase requires a resolved cutoff"))?;
            let taps = design_kaiser_lowpass_taps(params, cutoff_hz, beta);
            Ok(FilterDesign {
                estimated_enbw_hz: enbw_hz(&taps, params.sample_rate),
                taps,
                cutoff_hz,
                cutoff_source: params.cutoff_source.as_str(),
                legacy_boxcar_enbw_hz: None,
                enbw_match_error_hz: None,
                enbw_match_reachable: None,
                user_cutoff_unused: false,
                kaiser_beta: beta,
            })
        }
        LockinLpfKind::FirBoxcarEnbw => {
            let legacy_weights = legacy_boxcar_weights(params);
            let target_enbw = enbw_hz(&legacy_weights, params.sample_rate);
            let matched = match_boxcar_enbw_cutoff(params, beta, target_enbw);
            if !matched.reachable {
                eprintln!(
                    "⚠️ fir_boxcar_enbw target ENBW ({}) is outside the reachable FIR range; using nearest cutoff {}",
                    target_enbw, matched.cutoff_hz
                );
            }
            let taps = design_kaiser_lowpass_taps(params, matched.cutoff_hz, beta);
            let fir_enbw = enbw_hz(&taps, params.sample_rate);
            Ok(FilterDesign {
                taps,
                cutoff_hz: matched.cutoff_hz,
                cutoff_source: "enbw_match",
                estimated_enbw_hz: fir_enbw,
                legacy_boxcar_enbw_hz: Some(target_enbw),
                enbw_match_error_hz: Some((fir_enbw - target_enbw).abs()),
                enbw_match_reachable: Some(matched.reachable),
                user_cutoff_unused: lockin.lpf_cutoff_hz.is_some()
                    || lockin.lpf_cutoff_ref_ratio.is_some(),
                kaiser_beta: beta,
            })
        }
        LockinLpfKind::BoxcarLegacy => Err(anyhow!("boxcar_legacy does not use FIR design")),
    }
}

fn design_kaiser_lowpass_taps(params: LockinParams, cutoff_hz: f64, beta: f64) -> Vec<f64> {
    let cutoff_cycles = (cutoff_hz / params.sample_rate).min(0.499_999);
    let denom = bessel_i0(beta);

    let len = 2 * params.n_half + 1;
    let center = params.n_half as f64;
    let mut taps = Vec::with_capacity(len);

    for tap_idx in 0..len {
        let m = tap_idx as f64 - center;
        let ideal = if m == 0.0 {
            2.0 * cutoff_cycles
        } else {
            (2.0 * PI * cutoff_cycles * m).sin() / (PI * m)
        };

        let ratio = if center > 0.0 { m / center } else { 0.0 };
        let window = bessel_i0(beta * (1.0 - ratio * ratio).max(0.0).sqrt()) / denom;

        taps.push(ideal * window);
    }

    let sum: f64 = taps.iter().sum();
    if sum != 0.0 {
        for tap in &mut taps {
            *tap /= sum;
        }
    }

    taps
}

struct EnbwMatch {
    cutoff_hz: f64,
    reachable: bool,
}

fn match_boxcar_enbw_cutoff(params: LockinParams, beta: f64, target_enbw: f64) -> EnbwMatch {
    let min_cutoff = (params.sample_rate / 1.0e12).max(f64::MIN_POSITIVE);
    let max_cutoff = 0.45 * params.output_rate;
    let enbw_at = |cutoff_hz: f64| {
        let taps = design_kaiser_lowpass_taps(params, cutoff_hz, beta);
        enbw_hz(&taps, params.sample_rate)
    };

    let min_enbw = enbw_at(min_cutoff);
    let max_enbw = enbw_at(max_cutoff);
    if target_enbw <= min_enbw {
        return EnbwMatch {
            cutoff_hz: min_cutoff,
            reachable: false,
        };
    }
    if target_enbw >= max_enbw {
        return EnbwMatch {
            cutoff_hz: max_cutoff,
            reachable: false,
        };
    }

    let mut lo = min_cutoff;
    let mut hi = max_cutoff;
    for _ in 0..64 {
        let mid = 0.5 * (lo + hi);
        if enbw_at(mid) < target_enbw {
            lo = mid;
        } else {
            hi = mid;
        }
    }

    EnbwMatch {
        cutoff_hz: 0.5 * (lo + hi),
        reachable: true,
    }
}

fn enbw_hz(weights: &[f64], sample_rate: f64) -> f64 {
    let sum: f64 = weights.iter().sum();
    let sum_sq: f64 = weights.iter().map(|w| w * w).sum();
    if sum == 0.0 {
        f64::NAN
    } else {
        sample_rate * sum_sq / (sum * sum)
    }
}

fn legacy_boxcar_weights(params: LockinParams) -> Vec<f64> {
    let n = params.n_half;
    let len = 2 * n + 3;
    let mut weights = vec![0.0; len];
    let norm = 1.0 / (2.0 * params.t_half);

    for j in 0..(2 * n) {
        weights[j + 1] += 0.5 * params.dt * norm;
        weights[j + 2] += 0.5 * params.dt * norm;
    }

    let edge_dt = params.t_half - (n as f64) * params.dt;
    if edge_dt > 0.0 {
        let inner_coeff = 0.5 * edge_dt * (2.0 - edge_dt / params.dt) * norm;
        let outer_coeff = 0.5 * edge_dt * (edge_dt / params.dt) * norm;
        weights[0] += outer_coeff;
        weights[1] += inner_coeff;
        weights[len - 2] += inner_coeff;
        weights[len - 1] += outer_coeff;
    }

    weights
}

fn kaiser_beta(stopband_atten_db: f64) -> f64 {
    if stopband_atten_db > 50.0 {
        0.1102 * (stopband_atten_db - 8.7)
    } else if stopband_atten_db >= 21.0 {
        0.5842 * (stopband_atten_db - 21.0).powf(0.4)
            + 0.07886 * (stopband_atten_db - 21.0)
    } else {
        0.0
    }
}

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let y = x * x / 4.0;

    for k in 1..=24 {
        let kf = k as f64;
        term *= y / (kf * kf);
        sum += term;

        if term.abs() < 1e-16 * sum.abs() {
            break;
        }
    }

    sum
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::LockinLpfKind;
    use crate::lockin::lockin_params::CutoffSource;

    fn test_params() -> LockinParams {
        LockinParams {
            dt: 1.0e-4,
            sample_rate: 10_000.0,
            output_rate: 10_000.0,
            stride: 1,
            length: 20_000,
            f_ref: 1_000.0,
            lpf_kind: LockinLpfKind::FirBoxcarEnbw,
            lpf_stopband_atten_db: 60.0,
            cutoff_hz: None,
            cutoff_source: CutoffSource::EnbwMatch,
            fallback_used: false,
            omega: 2.0 * PI * 1_000.0,
            t_half: 0.001,
            n_half: 10,
            i_start: 13,
            i_end: 19_987,
        }
    }

    #[test]
    fn legacy_boxcar_weights_have_unit_dc_gain() {
        let weights = legacy_boxcar_weights(test_params());
        let sum: f64 = weights.iter().sum();
        assert!((sum - 1.0).abs() < 1.0e-12);
    }

    #[test]
    fn fir_boxcar_enbw_search_matches_reachable_target() {
        let params = test_params();
        let beta = kaiser_beta(params.lpf_stopband_atten_db);
        let min_cutoff = (params.sample_rate / 1.0e12).max(f64::MIN_POSITIVE);
        let max_cutoff = 0.45 * params.output_rate;
        let min_enbw = enbw_hz(
            &design_kaiser_lowpass_taps(params, min_cutoff, beta),
            params.sample_rate,
        );
        let max_enbw = enbw_hz(
            &design_kaiser_lowpass_taps(params, max_cutoff, beta),
            params.sample_rate,
        );
        let target = 0.5 * (min_enbw + max_enbw);
        let matched = match_boxcar_enbw_cutoff(params, beta, target);
        assert!(matched.reachable);

        let taps = design_kaiser_lowpass_taps(params, matched.cutoff_hz, beta);
        let actual = enbw_hz(&taps, params.sample_rate);
        let rel_err = (actual - target).abs() / target;
        assert!(rel_err < 1.0e-6, "relative error: {rel_err}");
    }

    #[test]
    fn enbw_matches_white_noise_variance_reduction() {
        let params = test_params();
        let weights = legacy_boxcar_weights(params);
        let enbw = enbw_hz(&weights, params.sample_rate);
        let expected_ratio = enbw / params.sample_rate;

        let mut state = 0x1234_5678_9abc_def0_u64;
        let mut input = Vec::with_capacity(50_000);
        for _ in 0..50_000 {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let unit = ((state >> 11) as f64) / ((1_u64 << 53) as f64);
            input.push((unit - 0.5) * 12.0_f64.sqrt());
        }

        let input_var = population_variance(&input);
        let mut output = Vec::new();
        for idx in 0..(input.len() - weights.len()) {
            let y = weights
                .iter()
                .zip(input[idx..].iter())
                .map(|(w, x)| w * x)
                .sum::<f64>();
            output.push(y);
        }
        let output_var = population_variance(&output);
        let actual_ratio = output_var / input_var;

        assert!(
            (actual_ratio - expected_ratio).abs() / expected_ratio < 0.15,
            "actual={actual_ratio}, expected={expected_ratio}"
        );
    }

    fn population_variance(values: &[f64]) -> f64 {
        let mean = values.iter().sum::<f64>() / values.len() as f64;
        values
            .iter()
            .map(|value| {
                let diff = value - mean;
                diff * diff
            })
            .sum::<f64>()
            / values.len() as f64
    }
}
