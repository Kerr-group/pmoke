use crate::config::{Lockin, LockinLpfKind};
use crate::lockin::lockin_params::LockinParams;
use anyhow::Result;
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
    fir_taps: Option<Vec<f64>>,
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
        let fir_taps = match params.lpf_kind {
            LockinLpfKind::FirZeroPhase => Some(design_fir_taps(params)),
            LockinLpfKind::BoxcarLegacy => None,
        };

        Ok(Self {
            t,
            data,
            omega_tref,
            params,
            fir_taps,
        })
    }

    fn ref_signal(&self, t: f64, harmonic: usize, ref_type: RefType) -> f64 {
        let arg = (harmonic as f64) * (self.params.omega * t - self.omega_tref);
        match ref_type {
            RefType::Sin => arg.sin(),
            RefType::Cos => arg.cos(),
        }
    }

    pub fn compute_harmonic(&self, harmonic: usize) -> (Vec<f64>, Vec<f64>) {
        match self.params.lpf_kind {
            LockinLpfKind::FirZeroPhase => self.compute_fir_lockin(harmonic),
            LockinLpfKind::BoxcarLegacy => (
                self.compute_legacy_lockin(harmonic, RefType::Sin),
                self.compute_legacy_lockin(harmonic, RefType::Cos),
            ),
        }
    }

    fn compute_fir_lockin(&self, harmonic: usize) -> (Vec<f64>, Vec<f64>) {
        let mixed_signal = self.compute_complex_mixed_signal(harmonic);
        let filtered = self.apply_fir(&mixed_signal);

        let li_x: Vec<f64> = filtered.iter().map(|z| -z.im).collect();
        let li_y: Vec<f64> = filtered.iter().map(|z| z.re).collect();

        (li_x, li_y)
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
        let taps = self
            .fir_taps
            .as_ref()
            .expect("FIR taps must be present for fir_zero_phase");

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

fn design_fir_taps(params: LockinParams) -> Vec<f64> {
    let sample_rate = 1.0 / params.dt;
    let output_rate = sample_rate / params.stride as f64;
    let raw_cutoff_hz = 0.5 / params.t_half;
    let cutoff_hz = raw_cutoff_hz.min(0.45 * output_rate);
    let cutoff_cycles = (cutoff_hz / sample_rate).min(0.499_999);
    let beta = kaiser_beta(params.lpf_stopband_atten_db);
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
