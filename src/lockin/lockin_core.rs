use crate::config::{Lockin, LockinLpfKind};
use crate::lockin::lockin_params::LockinParams;
use crate::ui;
use crate::utils::time_axis::TimeAxisRef;
use anyhow::{Result, anyhow};
use num_complex::Complex64;
use std::collections::VecDeque;
use std::f64::consts::PI;

const MAX_SYNC_AVERAGE_SAMPLES: usize = 1_000_000;
const PHASE_RESYNC_INTERVAL: usize = 4096;

#[cfg(test)]
#[derive(Clone, Copy)]
pub enum RefType {
    Sin,
    Cos,
}

pub struct LockinProcessor<'a> {
    t: TimeAxisRef<'a>,
    data: &'a [f64],
    omega_tref: f64,
    params: LockinParams,
    filter: Option<FilterDesign>,
}

#[derive(Debug, Clone)]
pub struct FilterDesign {
    pub taps: Vec<f64>,
    pub sos: Vec<Biquad>,
    pub cutoff_hz: f64,
    pub design_cutoff_hz: f64,
    pub cutoff_source: &'static str,
    pub estimated_enbw_hz: f64,
    pub legacy_boxcar_enbw_hz: Option<f64>,
    pub enbw_match_error_hz: Option<f64>,
    pub enbw_match_reachable: Option<bool>,
    pub user_cutoff_unused: bool,
    pub kaiser_beta: f64,
    pub sync_average_samples: Option<usize>,
    pub iir_order: Option<usize>,
    pub settling_samples: usize,
}

#[derive(Debug, Clone, Copy)]
pub struct Biquad {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

impl FilterDesign {
    pub fn response_abs(&self, sample_rate: f64, freq_hz: f64) -> f64 {
        if self.sos.is_empty() {
            fir_response_abs(&self.taps, sample_rate, freq_hz)
        } else {
            fir_response_abs(&self.taps, sample_rate, freq_hz)
                * iir_response_abs(&self.sos, sample_rate, freq_hz).powi(2)
        }
    }
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
        t: impl Into<TimeAxisRef<'a>>,
        data: &'a [f64],
        f_ref: f64,
        omega_tref: f64,
        lockin: &Lockin,
    ) -> Result<Self> {
        let t = t.into();
        assert!(t.len() >= 2);
        assert_eq!(t.len(), data.len());

        let params = LockinParams::from_geometry(
            t.len(),
            t.dt().expect("time axis has at least two samples"),
            f_ref,
            lockin,
        )?;
        let filter = match params.lpf_kind {
            LockinLpfKind::FirZeroPhase
            | LockinLpfKind::FirBoxcarEnbw
            | LockinLpfKind::SyncIirZeroPhase => Some(design_filter(params, lockin)?),
            LockinLpfKind::BoxcarLegacy => None,
        };
        validate_output_index_range(params, filter.as_ref())?;

        Ok(Self {
            t,
            data,
            omega_tref,
            params,
            filter,
        })
    }

    #[cfg(test)]
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
            LockinLpfKind::SyncIirZeroPhase => {
                self.compute_sync_iir_lockin(harmonic, include_debug_data)
            }
            LockinLpfKind::BoxcarLegacy => self.compute_legacy_lockin_pair(harmonic),
        }
    }

    pub fn output_times(&self) -> Vec<f64> {
        let (i_start, i_end) = self.output_index_range();
        (i_start..=i_end)
            .map(|i_idx| self.t.value_at(i_idx * self.params.stride))
            .collect()
    }

    pub fn params(&self) -> LockinParams {
        self.params
    }

    pub fn filter_design(&self) -> Option<&FilterDesign> {
        self.filter.as_ref()
    }

    pub fn base_index_range(&self) -> (usize, usize) {
        (self.params.i_start, self.params.i_end)
    }

    pub fn output_index_range(&self) -> (usize, usize) {
        output_index_range_for(self.params, self.filter.as_ref())
    }

    pub fn summary_lines(&self) -> Vec<String> {
        let mut lines = Vec::new();
        lines.push(format!("lpf_kind={:?}", self.params.lpf_kind));
        lines.push(format!("f_ref={:.6e} Hz", self.params.f_ref));
        lines.push(format!(
            "half_window={:.6e} s, support={:.6e} s, tap_count={}",
            self.params.t_half,
            2.0 * self.params.t_half,
            2 * self.params.n_half + 1
        ));
        lines.push(format!(
            "sample_rate={:.6e} Hz, output_rate={:.6e} Hz, stride_samples={}",
            self.params.sample_rate, self.params.output_rate, self.params.stride
        ));
        match &self.filter {
            Some(filter) => {
                lines.push(format!(
                    "cutoff={:.6e} Hz ({})",
                    filter.cutoff_hz, filter.cutoff_source
                ));
                lines.push(format!(
                    "estimated_enbw={:.6e} Hz",
                    filter.estimated_enbw_hz
                ));
                if let Some(legacy_enbw) = filter.legacy_boxcar_enbw_hz {
                    lines.push(format!("legacy_boxcar_enbw={legacy_enbw:.6e} Hz"));
                }
                if let Some(error) = filter.enbw_match_error_hz {
                    lines.push(format!("enbw_match_error={error:.6e} Hz"));
                }
                if let Some(samples) = filter.sync_average_samples {
                    lines.push(format!(
                        "sync_average_samples={}, sync_average_cycles={:.6e}",
                        samples, self.params.lpf_sync_average_cycles
                    ));
                }
                if let Some(order) = filter.iir_order {
                    lines.push(format!("iir_order={order}"));
                }
                if filter.design_cutoff_hz != filter.cutoff_hz {
                    lines.push(format!(
                        "design_cutoff={:.6e} Hz, requested_zero_phase_cutoff={:.6e} Hz",
                        filter.design_cutoff_hz, filter.cutoff_hz
                    ));
                }
                lines.push(format!("settling_samples={}", filter.settling_samples));
                if self.params.fallback_used {
                    lines.push("cutoff_fallback_used=true".to_string());
                }
            }
            None => {
                lines.push("cutoff=none".to_string());
                lines.push("estimated_enbw=legacy_boxcar".to_string());
            }
        }
        lines
    }

    fn compute_fir_lockin(
        &self,
        harmonic: usize,
        include_debug_data: bool,
    ) -> HarmonicLockinResult {
        if !include_debug_data {
            let (raw_start, raw_end) = self.fir_required_raw_range();
            let mixed_signal =
                self.compute_complex_mixed_signal_range(harmonic, raw_start, raw_end);
            let (li_x, li_y) = self.apply_fir_to_li(&mixed_signal, raw_start);
            return HarmonicLockinResult {
                li_x,
                li_y,
                mixed_signal: None,
            };
        }

        let mixed_signal = self.compute_complex_mixed_signal(harmonic);
        let (li_x, li_y) = self.apply_fir_to_li(&mixed_signal, 0);
        HarmonicLockinResult {
            li_x,
            li_y,
            mixed_signal: Some(mixed_signal),
        }
    }

    fn compute_sync_iir_lockin(
        &self,
        harmonic: usize,
        include_debug_data: bool,
    ) -> HarmonicLockinResult {
        let filter = self
            .filter
            .as_ref()
            .expect("IIR design must be present for sync_iir_zero_phase lock-in");
        let sync_len = filter.sync_average_samples.unwrap_or(1);

        let (i_start, i_end) = self.output_index_range();
        let m = if i_end >= i_start {
            i_end - i_start + 1
        } else {
            0
        };

        if !include_debug_data {
            let (mut re, mut im) = self.compute_sync_averaged_real_imag_signal(harmonic, sync_len);
            apply_sos_filtfilt_pair_in_place(&mut re, &mut im, &filter.sos);

            let mut li_x = Vec::with_capacity(m);
            let mut li_y = Vec::with_capacity(m);
            for i_idx in i_start..=i_end {
                let sample_idx = i_idx * self.params.stride;
                li_x.push(-im[sample_idx]);
                li_y.push(re[sample_idx]);
            }

            return HarmonicLockinResult {
                li_x,
                li_y,
                mixed_signal: None,
            };
        }

        let mixed_signal = self.compute_complex_mixed_signal(harmonic);
        let debug_mixed_signal = mixed_signal.clone();
        let mut filtered = mixed_signal;
        apply_centered_moving_average_complex_in_place(&mut filtered, sync_len);
        apply_sos_filtfilt_in_place(&mut filtered, &filter.sos);

        let mut li_x = Vec::with_capacity(m);
        let mut li_y = Vec::with_capacity(m);
        for i_idx in i_start..=i_end {
            let z = filtered[i_idx * self.params.stride];
            li_x.push(-z.im);
            li_y.push(z.re);
        }

        HarmonicLockinResult {
            li_x,
            li_y,
            mixed_signal: Some(debug_mixed_signal),
        }
    }

    fn compute_complex_mixed_signal(&self, harmonic: usize) -> Vec<Complex64> {
        self.compute_complex_mixed_signal_range(harmonic, 0, self.data.len())
    }

    fn compute_complex_mixed_signal_range(
        &self,
        harmonic: usize,
        start: usize,
        end: usize,
    ) -> Vec<Complex64> {
        let harmonic = harmonic as f64;
        let step_phase = -harmonic * self.params.omega * self.params.dt;
        let step = Complex64::from_polar(1.0, step_phase);
        let anchor = start - (start % PHASE_RESYNC_INTERVAL);
        let anchor_phase =
            -harmonic * (self.params.omega * self.t.value_at(anchor) - self.omega_tref);
        let mut osc = Complex64::from_polar(1.0, anchor_phase);
        for _ in anchor..start {
            osc *= step;
        }

        let mut mixed = Vec::with_capacity(end - start);
        for (idx, &sample) in self.data[start..end].iter().enumerate() {
            let raw_idx = start + idx;
            if idx > 0 && raw_idx.is_multiple_of(PHASE_RESYNC_INTERVAL) {
                let phase =
                    -harmonic * (self.params.omega * self.t.value_at(raw_idx) - self.omega_tref);
                osc = Complex64::from_polar(1.0, phase);
            }

            mixed.push(sample * osc);
            osc *= step;
        }

        mixed
    }

    fn compute_real_imag_mixed_signal(&self, harmonic: usize) -> (Vec<f64>, Vec<f64>) {
        let harmonic = harmonic as f64;
        let phase0 = harmonic * (self.params.omega * self.t.value_at(0) - self.omega_tref);
        let step_phase = -harmonic * self.params.omega * self.params.dt;
        let step = Complex64::from_polar(1.0, step_phase);
        let mut osc = Complex64::from_polar(1.0, -phase0);

        let mut re = Vec::with_capacity(self.data.len());
        let mut im = Vec::with_capacity(self.data.len());
        for (idx, &sample) in self.data.iter().enumerate() {
            if idx > 0 && idx.is_multiple_of(PHASE_RESYNC_INTERVAL) {
                let phase = -phase0 + (idx as f64) * step_phase;
                osc = Complex64::from_polar(1.0, phase);
            }

            re.push(sample * osc.re);
            im.push(sample * osc.im);
            osc *= step;
        }

        (re, im)
    }

    fn compute_real_imag_mixed_signal_range(
        &self,
        harmonic: usize,
        start: usize,
        end: usize,
    ) -> (Vec<f64>, Vec<f64>) {
        let harmonic = harmonic as f64;
        let phase0 = harmonic * (self.params.omega * self.t.value_at(0) - self.omega_tref);
        let step_phase = -harmonic * self.params.omega * self.params.dt;
        let step = Complex64::from_polar(1.0, step_phase);
        let anchor = start - (start % PHASE_RESYNC_INTERVAL);
        let anchor_phase = -phase0 + (anchor as f64) * step_phase;
        let mut osc = Complex64::from_polar(1.0, anchor_phase);
        for _ in anchor..start {
            osc *= step;
        }

        let mut re = Vec::with_capacity(end - start);
        let mut im = Vec::with_capacity(end - start);
        for (idx, &sample) in self.data[start..end].iter().enumerate() {
            let raw_idx = start + idx;
            if idx > 0 && raw_idx.is_multiple_of(PHASE_RESYNC_INTERVAL) {
                let phase = -phase0 + (raw_idx as f64) * step_phase;
                osc = Complex64::from_polar(1.0, phase);
            }

            re.push(sample * osc.re);
            im.push(sample * osc.im);
            osc *= step;
        }

        (re, im)
    }

    fn compute_sync_averaged_real_imag_signal(
        &self,
        harmonic: usize,
        sync_len: usize,
    ) -> (Vec<f64>, Vec<f64>) {
        let sync_len = sync_len.max(1);
        if sync_len == 1 {
            return self.compute_real_imag_mixed_signal(harmonic);
        }

        let harmonic = harmonic as f64;
        let step_phase = -harmonic * self.params.omega * self.params.dt;
        let step = Complex64::from_polar(1.0, step_phase);
        let phase0 = harmonic * (self.params.omega * self.t.value_at(0) - self.omega_tref);
        let mut osc = Complex64::from_polar(1.0, -phase0);

        let left = (sync_len - 1) / 2;
        let right = sync_len - left;
        let mut start = 0usize;
        let mut end = 0usize;
        let mut sum_re = 0.0;
        let mut sum_im = 0.0;
        let mut window = VecDeque::with_capacity(sync_len.min(self.data.len()));
        let mut out_re = Vec::with_capacity(self.data.len());
        let mut out_im = Vec::with_capacity(self.data.len());

        for idx in 0..self.data.len() {
            let desired_start = idx.saturating_sub(left);
            let desired_end = (idx + right).min(self.data.len());
            while start < desired_start {
                let (value_re, value_im) =
                    window.pop_front().expect("moving average window underflow");
                sum_re -= value_re;
                sum_im -= value_im;
                start += 1;
            }
            while end < desired_end {
                if end > 0 && end.is_multiple_of(PHASE_RESYNC_INTERVAL) {
                    let phase =
                        -harmonic * (self.params.omega * self.t.value_at(end) - self.omega_tref);
                    osc = Complex64::from_polar(1.0, phase);
                }
                let sample = self.data[end];
                let value_re = sample * osc.re;
                let value_im = sample * osc.im;
                window.push_back((value_re, value_im));
                sum_re += value_re;
                sum_im += value_im;
                osc *= step;
                end += 1;
            }
            let scale = 1.0 / window.len() as f64;
            out_re.push(sum_re * scale);
            out_im.push(sum_im * scale);
        }

        (out_re, out_im)
    }

    fn fir_required_raw_range(&self) -> (usize, usize) {
        let (i_start, i_end) = self.output_index_range();
        let raw_start = i_start * self.params.stride - self.params.n_half;
        let raw_end = i_end * self.params.stride + self.params.n_half + 1;
        (raw_start, raw_end)
    }

    fn apply_fir_to_li(
        &self,
        mixed_signal: &[Complex64],
        raw_start: usize,
    ) -> (Vec<f64>, Vec<f64>) {
        let filter = self
            .filter
            .as_ref()
            .expect("FIR taps must be present for FIR lock-in");
        let taps = &filter.taps;

        let (i_start, i_end) = self.output_index_range();
        let m = if i_end >= i_start {
            i_end - i_start + 1
        } else {
            0
        };
        let mut li_x = Vec::with_capacity(m);
        let mut li_y = Vec::with_capacity(m);

        for i_idx in i_start..=i_end {
            let i_base = i_idx * self.params.stride;
            let base_start = i_base - self.params.n_half - raw_start;
            let mut acc_re = 0.0;
            let mut acc_im = 0.0;

            for (tap_idx, &tap) in taps.iter().enumerate() {
                let sample_idx = base_start + tap_idx;
                let sample = mixed_signal[sample_idx];
                acc_re += sample.re * tap;
                acc_im += sample.im * tap;
            }

            li_x.push(-acc_im);
            li_y.push(acc_re);
        }

        (li_x, li_y)
    }

    fn compute_legacy_lockin_pair(&self, harmonic: usize) -> HarmonicLockinResult {
        let (i_start, i_end) = (self.params.i_start, self.params.i_end);
        let raw_start = i_start * self.params.stride - self.params.n_half - 1;
        let raw_end = i_end * self.params.stride + self.params.n_half + 2;
        let (mixed_re, mixed_im) =
            self.compute_real_imag_mixed_signal_range(harmonic, raw_start, raw_end);
        let prefix_re = prefix_sum(&mixed_re);
        let prefix_im = prefix_sum(&mixed_im);

        let m = if i_end >= i_start {
            i_end - i_start + 1
        } else {
            0
        };
        let mut li_x = Vec::with_capacity(m);
        let mut li_y = Vec::with_capacity(m);

        for k in 0..m {
            let i_idx = i_start + k;
            let i_base = i_idx * self.params.stride;
            let neg_idx0 = i_base - self.params.n_half - raw_start;
            let pos_idx0 = i_base + self.params.n_half - raw_start;
            let integ_re =
                trapezoid_integral_from_prefix(&mixed_re, &prefix_re, neg_idx0, pos_idx0)
                    * self.params.dt;
            let integ_im =
                trapezoid_integral_from_prefix(&mixed_im, &prefix_im, neg_idx0, pos_idx0)
                    * self.params.dt;

            let neg_idx1 = i_base - self.params.n_half - 1 - raw_start;
            let pos_idx1 = i_base + self.params.n_half + 1 - raw_start;
            let edge_dt = self.params.t_half - (self.params.n_half as f64) * self.params.dt;

            let edge_neg_re = legacy_edge_integral(
                mixed_re[neg_idx0],
                mixed_re[neg_idx1],
                edge_dt,
                self.params.dt,
            );
            let edge_pos_re = legacy_edge_integral(
                mixed_re[pos_idx0],
                mixed_re[pos_idx1],
                edge_dt,
                self.params.dt,
            );
            let edge_neg_im = legacy_edge_integral(
                mixed_im[neg_idx0],
                mixed_im[neg_idx1],
                edge_dt,
                self.params.dt,
            );
            let edge_pos_im = legacy_edge_integral(
                mixed_im[pos_idx0],
                mixed_im[pos_idx1],
                edge_dt,
                self.params.dt,
            );

            let scale = 1.0 / (2.0 * self.params.t_half);
            li_x.push(-(integ_im + edge_neg_im + edge_pos_im) * scale);
            li_y.push((integ_re + edge_neg_re + edge_pos_re) * scale);
        }

        HarmonicLockinResult {
            li_x,
            li_y,
            mixed_signal: None,
        }
    }
}

fn prefix_sum(values: &[f64]) -> Vec<f64> {
    let mut prefix = Vec::with_capacity(values.len() + 1);
    prefix.push(0.0);
    let mut acc = 0.0;
    for &value in values {
        acc += value;
        prefix.push(acc);
    }
    prefix
}

fn trapezoid_integral_from_prefix(values: &[f64], prefix: &[f64], start: usize, end: usize) -> f64 {
    debug_assert!(start < end);
    debug_assert_eq!(prefix.len(), values.len() + 1);
    0.5 * values[start] + (prefix[end] - prefix[start + 1]) + 0.5 * values[end]
}

fn legacy_edge_integral(y0: f64, y1: f64, edge_dt: f64, dt: f64) -> f64 {
    let ym = (y1 * edge_dt + y0 * (dt - edge_dt)) / dt;
    edge_dt * 0.5 * (y0 + ym)
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
                sos: Vec::new(),
                cutoff_hz,
                design_cutoff_hz: cutoff_hz,
                cutoff_source: params.cutoff_source.as_str(),
                legacy_boxcar_enbw_hz: None,
                enbw_match_error_hz: None,
                enbw_match_reachable: None,
                user_cutoff_unused: false,
                kaiser_beta: beta,
                sync_average_samples: None,
                iir_order: None,
                settling_samples: params.n_half,
            })
        }
        LockinLpfKind::FirBoxcarEnbw => {
            let legacy_weights = legacy_boxcar_weights(params);
            let target_enbw = enbw_hz(&legacy_weights, params.sample_rate);
            let matched = match_boxcar_enbw_cutoff(params, beta, target_enbw);
            if !matched.reachable {
                ui::warn(format!(
                    "fir_boxcar_enbw target ENBW ({target_enbw}) is outside the reachable FIR range; using nearest cutoff {}",
                    matched.cutoff_hz
                ));
            }
            let taps = design_kaiser_lowpass_taps(params, matched.cutoff_hz, beta);
            let fir_enbw = enbw_hz(&taps, params.sample_rate);
            Ok(FilterDesign {
                taps,
                sos: Vec::new(),
                cutoff_hz: matched.cutoff_hz,
                design_cutoff_hz: matched.cutoff_hz,
                cutoff_source: "enbw_match",
                estimated_enbw_hz: fir_enbw,
                legacy_boxcar_enbw_hz: Some(target_enbw),
                enbw_match_error_hz: Some((fir_enbw - target_enbw).abs()),
                enbw_match_reachable: Some(matched.reachable),
                user_cutoff_unused: lockin.lpf_cutoff_hz.is_some()
                    || lockin.lpf_cutoff_ref_ratio.is_some(),
                kaiser_beta: beta,
                sync_average_samples: None,
                iir_order: None,
                settling_samples: params.n_half,
            })
        }
        LockinLpfKind::SyncIirZeroPhase => {
            let cutoff_hz = params
                .cutoff_hz
                .ok_or_else(|| anyhow!("sync_iir_zero_phase requires a resolved cutoff"))?;
            let sync_average_samples = sync_average_samples(params)?;
            let taps = moving_average_taps(sync_average_samples);
            let design_cutoff_hz =
                zero_phase_butterworth_design_cutoff(cutoff_hz, params.lpf_iir_order);
            let sos = design_butterworth_lowpass_sos(
                params.lpf_iir_order,
                design_cutoff_hz,
                params.sample_rate,
            )?;
            let estimated_enbw_hz =
                estimate_response_enbw_hz(&taps, &sos, params.sample_rate, true);
            let settling_samples =
                sync_average_samples / 2 + iir_settling_samples(params, design_cutoff_hz);
            Ok(FilterDesign {
                taps,
                sos,
                cutoff_hz,
                design_cutoff_hz,
                cutoff_source: params.cutoff_source.as_str(),
                estimated_enbw_hz,
                legacy_boxcar_enbw_hz: None,
                enbw_match_error_hz: None,
                enbw_match_reachable: None,
                user_cutoff_unused: false,
                kaiser_beta: f64::NAN,
                sync_average_samples: Some(sync_average_samples),
                iir_order: Some(params.lpf_iir_order),
                settling_samples,
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

fn output_index_range_for(params: LockinParams, filter: Option<&FilterDesign>) -> (usize, usize) {
    let mut i_start = params.i_start;
    let mut i_end = params.i_end;
    if let Some(filter) = filter
        && filter.settling_samples > params.n_half
    {
        let extra = filter.settling_samples - params.n_half;
        let extra_idx = extra.div_ceil(params.stride);
        i_start = i_start.saturating_add(extra_idx);
        i_end = i_end.saturating_sub(extra_idx);
    }
    (i_start, i_end)
}

fn validate_output_index_range(params: LockinParams, filter: Option<&FilterDesign>) -> Result<()> {
    let (base_start, base_end) = (params.i_start, params.i_end);
    let (output_start, output_end) = output_index_range_for(params, filter);
    if output_start <= output_end {
        return Ok(());
    }

    let settling_samples = filter
        .map(|filter| filter.settling_samples.to_string())
        .unwrap_or_else(|| "none".to_string());
    Err(anyhow!(
        "lock-in output range is empty after LPF edge trimming: base_index_range=({base_start}, {base_end}), output_index_range=({output_start}, {output_end}), settling_samples={settling_samples}; reduce LPF settling by increasing cutoff, reducing lpf_iir_order/lpf_sync_average_cycles, reducing lpf_half_window_cycles, or using a longer trace"
    ))
}

fn sync_average_samples(params: LockinParams) -> Result<usize> {
    let samples = (params.lpf_sync_average_cycles * params.sample_rate / params.f_ref).round();
    if !samples.is_finite() || samples <= 0.0 || samples > MAX_SYNC_AVERAGE_SAMPLES as f64 {
        return Err(anyhow!(
            "sync average samples must be finite and <= {MAX_SYNC_AVERAGE_SAMPLES} (got {samples})"
        ));
    }
    Ok(samples as usize)
}

fn moving_average_taps(len: usize) -> Vec<f64> {
    let len = len.max(1);
    vec![1.0 / len as f64; len]
}

fn apply_centered_moving_average_complex_in_place(values: &mut [Complex64], len: usize) {
    let len = len.max(1);
    if len == 1 || values.is_empty() {
        return;
    }

    let left = (len - 1) / 2;
    let right = len - left;
    let mut start = 0usize;
    let mut end = 0usize;
    let mut sum = Complex64::new(0.0, 0.0);
    let mut window = VecDeque::with_capacity(len.min(values.len()));

    for idx in 0..values.len() {
        let desired_start = idx.saturating_sub(left);
        let desired_end = (idx + right).min(values.len());
        while start < desired_start {
            let value = window.pop_front().expect("moving average window underflow");
            sum -= value;
            start += 1;
        }
        while end < desired_end {
            let value = values[end];
            window.push_back(value);
            sum += value;
            end += 1;
        }
        values[idx] = sum / window.len() as f64;
    }
}

fn zero_phase_butterworth_design_cutoff(target_cutoff_hz: f64, order: usize) -> f64 {
    target_cutoff_hz / (2.0_f64.sqrt() - 1.0).powf(1.0 / (2.0 * order as f64))
}

fn iir_settling_samples(params: LockinParams, cutoff_hz: f64) -> usize {
    let ratio = (cutoff_hz / params.sample_rate).clamp(f64::MIN_POSITIVE, 0.49);
    let tau_samples = 1.0 / (2.0 * PI * ratio);
    (8.0 * tau_samples).ceil() as usize
}

fn design_butterworth_lowpass_sos(
    order: usize,
    cutoff_hz: f64,
    sample_rate: f64,
) -> Result<Vec<Biquad>> {
    if order == 0 || !order.is_multiple_of(2) || order > 8 {
        return Err(anyhow!(
            "sync_iir_zero_phase lpf_iir_order must be one of 2, 4, 6, or 8 (got {order})"
        ));
    }
    if cutoff_hz <= 0.0 || cutoff_hz >= 0.5 * sample_rate {
        return Err(anyhow!(
            "sync_iir_zero_phase cutoff_hz ({cutoff_hz}) must be between 0 and sample Nyquist ({})",
            0.5 * sample_rate
        ));
    }

    let warped = 2.0 * sample_rate * (PI * cutoff_hz / sample_rate).tan();
    let bilinear_scale = 2.0 * sample_rate;
    let mut sos = Vec::with_capacity(order / 2);

    for pair_idx in 0..(order / 2) {
        let theta = PI * (2.0 * pair_idx as f64 + 1.0 + order as f64) / (2.0 * order as f64);
        let analog_pole = Complex64::from_polar(warped, theta);
        let digital_pole = (bilinear_scale + analog_pole) / (bilinear_scale - analog_pole);
        let a1 = -2.0 * digital_pole.re;
        let a2 = digital_pole.norm_sqr();
        let gain = (1.0 + a1 + a2) / 4.0;
        sos.push(Biquad {
            b0: gain,
            b1: 2.0 * gain,
            b2: gain,
            a1,
            a2,
        });
    }

    Ok(sos)
}

fn apply_sos_filtfilt_in_place(values: &mut [Complex64], sos: &[Biquad]) {
    if values.is_empty() || sos.is_empty() {
        return;
    }

    apply_sos_forward_in_place(values, sos);
    values.reverse();
    apply_sos_forward_in_place(values, sos);
    values.reverse();
}

fn apply_sos_filtfilt_pair_in_place(re: &mut [f64], im: &mut [f64], sos: &[Biquad]) {
    debug_assert_eq!(re.len(), im.len());
    if re.is_empty() || sos.is_empty() {
        return;
    }

    apply_sos_forward_pair_in_place(re, im, sos);
    re.reverse();
    im.reverse();
    apply_sos_forward_pair_in_place(re, im, sos);
    re.reverse();
    im.reverse();
}

fn apply_sos_forward_in_place(values: &mut [Complex64], sos: &[Biquad]) {
    for section in sos {
        apply_biquad_forward_in_place(values, *section);
    }
}

fn apply_sos_forward_pair_in_place(re: &mut [f64], im: &mut [f64], sos: &[Biquad]) {
    for section in sos {
        apply_biquad_forward_pair_in_place(re, im, *section);
    }
}

fn apply_biquad_forward_in_place(values: &mut [Complex64], section: Biquad) {
    if values.is_empty() {
        return;
    }

    let steady = values[0];
    let mut z1 = steady * (1.0 - section.b0);
    let mut z2 = steady * (1.0 - section.b0 - section.b1 + section.a1);

    for value in values {
        let x = *value;
        let y = section.b0 * x + z1;
        z1 = section.b1 * x - section.a1 * y + z2;
        z2 = section.b2 * x - section.a2 * y;
        *value = y;
    }
}

fn apply_biquad_forward_pair_in_place(re: &mut [f64], im: &mut [f64], section: Biquad) {
    debug_assert_eq!(re.len(), im.len());
    if re.is_empty() {
        return;
    }

    let steady_re = re[0];
    let steady_im = im[0];
    let mut z1_re = steady_re * (1.0 - section.b0);
    let mut z2_re = steady_re * (1.0 - section.b0 - section.b1 + section.a1);
    let mut z1_im = steady_im * (1.0 - section.b0);
    let mut z2_im = steady_im * (1.0 - section.b0 - section.b1 + section.a1);

    for (value_re, value_im) in re.iter_mut().zip(im.iter_mut()) {
        let x_re = *value_re;
        let y_re = section.b0 * x_re + z1_re;
        z1_re = section.b1 * x_re - section.a1 * y_re + z2_re;
        z2_re = section.b2 * x_re - section.a2 * y_re;
        *value_re = y_re;

        let x_im = *value_im;
        let y_im = section.b0 * x_im + z1_im;
        z1_im = section.b1 * x_im - section.a1 * y_im + z2_im;
        z2_im = section.b2 * x_im - section.a2 * y_im;
        *value_im = y_im;
    }
}

fn fir_response_abs(taps: &[f64], sample_rate: f64, freq_hz: f64) -> f64 {
    if taps.is_empty() {
        return 1.0;
    }
    let omega = 2.0 * PI * freq_hz / sample_rate;
    let center = (taps.len() / 2) as isize;
    let mut acc = Complex64::new(0.0, 0.0);
    for (idx, &tap) in taps.iter().enumerate() {
        let n = idx as isize - center;
        acc += tap * Complex64::from_polar(1.0, -omega * n as f64);
    }
    acc.norm()
}

fn iir_response_abs(sos: &[Biquad], sample_rate: f64, freq_hz: f64) -> f64 {
    let omega = 2.0 * PI * freq_hz / sample_rate;
    let z1 = Complex64::from_polar(1.0, -omega);
    let z2 = Complex64::from_polar(1.0, -2.0 * omega);
    sos.iter().fold(1.0, |acc, section| {
        let numerator = section.b0 + section.b1 * z1 + section.b2 * z2;
        let denominator = Complex64::new(1.0, 0.0) + section.a1 * z1 + section.a2 * z2;
        acc * (numerator / denominator).norm()
    })
}

fn estimate_response_enbw_hz(
    taps: &[f64],
    sos: &[Biquad],
    sample_rate: f64,
    iir_forward_backward: bool,
) -> f64 {
    const BINS: usize = 16_384;
    let df = 0.5 * sample_rate / BINS as f64;
    let mut integral = 0.0;
    for idx in 0..=BINS {
        let freq = idx as f64 * df;
        let fir = fir_response_abs(taps, sample_rate, freq);
        let iir = iir_response_abs(sos, sample_rate, freq);
        let response_abs = if iir_forward_backward {
            fir * iir.powi(2)
        } else {
            fir * iir
        };
        let weight = if idx == 0 || idx == BINS { 0.5 } else { 1.0 };
        integral += weight * response_abs * response_abs;
    }
    2.0 * integral * df
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
        0.5842 * (stopband_atten_db - 21.0).powf(0.4) + 0.07886 * (stopband_atten_db - 21.0)
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
#[path = "lockin_core_tests.rs"]
mod tests;
