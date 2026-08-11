use crate::config::{Lockin, LockinLpfKind};
use crate::lockin::lockin_params::LockinParams;
use crate::utils::time_axis::TimeAxisRef;
use anyhow::{Result, anyhow, bail};
use num_complex::Complex64;

pub struct LockinProcessor<'a> {
    t: TimeAxisRef<'a>,
    finite_data: pmoke_analysis_core::FiniteSignal<'a>,
    omega_tref: f64,
    params: LockinParams,
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
        if t.len() < 2 {
            bail!("lock-in time axis must contain at least two samples");
        }
        if t.len() != data.len() {
            bail!(
                "lock-in time length ({}) and signal length ({}) differ",
                t.len(),
                data.len()
            );
        }
        if !t.value_at(0).is_finite() {
            bail!("lock-in start time must be finite");
        }
        if !omega_tref.is_finite() {
            bail!("lock-in reference phase must be finite");
        }
        if !matches!(lockin.lpf_kind, LockinLpfKind::BoxcarLegacy) {
            bail!("the active runtime supports only the boxcar_legacy LPF");
        }
        let finite_data = pmoke_analysis_core::FiniteSignal::new(data)?;
        let params = LockinParams::from_geometry(
            t.len(),
            t.dt()
                .ok_or_else(|| anyhow!("lock-in time axis must contain at least two samples"))?,
            f_ref,
            lockin,
        )?;
        validate_output_index_range(params)?;

        Ok(Self {
            t,
            finite_data,
            omega_tref,
            params,
        })
    }

    pub fn compute_harmonic_detailed(
        &self,
        harmonic: usize,
        _include_debug_data: bool,
    ) -> HarmonicLockinResult {
        self.compute_legacy_lockin_pair(harmonic)
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

    pub fn base_index_range(&self) -> (usize, usize) {
        (self.params.i_start, self.params.i_end)
    }

    pub fn output_index_range(&self) -> (usize, usize) {
        (self.params.i_start, self.params.i_end)
    }

    pub fn summary_lines(&self) -> Vec<String> {
        vec![
            format!("lpf_kind={:?}", self.params.lpf_kind),
            format!("f_ref={:.6e} Hz", self.params.f_ref),
            format!(
                "half_window={:.6e} s, support={:.6e} s, tap_count={}",
                self.params.t_half,
                2.0 * self.params.t_half,
                2 * self.params.n_half + 1
            ),
            format!(
                "sample_rate={:.6e} Hz, output_rate={:.6e} Hz, stride_samples={}",
                self.params.sample_rate, self.params.output_rate, self.params.stride
            ),
            "cutoff=none".to_string(),
            "estimated_enbw=legacy_boxcar".to_string(),
        ]
    }

    fn compute_legacy_lockin_pair(&self, harmonic: usize) -> HarmonicLockinResult {
        let output = pmoke_analysis_core::analyze_boxcar_legacy_pair_finite(
            self.finite_data,
            pmoke_analysis_core::BoxcarLegacySettings {
                start_time_s: self.t.value_at(0),
                sample_interval_s: self.params.dt,
                reference_frequency_hz: self.params.f_ref,
                reference_phase_rad: self.omega_tref,
                half_window_cycles: self.params.t_half * self.params.f_ref,
                stride_samples: self.params.stride,
                harmonic,
            },
        )
        .expect("boxcar settings and waveform are validated by LockinProcessor::new");
        debug_assert_eq!(
            output.metadata.first_input_index,
            self.params.i_start * self.params.stride
        );
        debug_assert_eq!(
            output.metadata.last_input_index,
            self.params.i_end * self.params.stride
        );
        HarmonicLockinResult {
            li_x: output.x,
            li_y: output.y,
            mixed_signal: None,
        }
    }
}

fn validate_output_index_range(params: LockinParams) -> Result<()> {
    let (base_start, base_end) = (params.i_start, params.i_end);
    if base_start <= base_end {
        return Ok(());
    }
    Err(anyhow!(
        "lock-in output range is empty after boxcar edge trimming: base_index_range=({base_start}, {base_end}); reduce lpf_half_window_cycles or use a longer trace"
    ))
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

pub(crate) fn legacy_boxcar_enbw_hz(params: LockinParams) -> f64 {
    enbw_hz(&legacy_boxcar_weights(params), params.sample_rate)
}

pub(crate) fn legacy_boxcar_response_abs(params: LockinParams, freq_hz: f64) -> f64 {
    let weights = legacy_boxcar_weights(params);
    let center = (weights.len() / 2) as isize;
    let omega = 2.0 * std::f64::consts::PI * freq_hz / params.sample_rate;
    let mut response = Complex64::new(0.0, 0.0);
    for (index, weight) in weights.iter().enumerate() {
        let offset = index as isize - center;
        response += *weight * Complex64::from_polar(1.0, -omega * offset as f64);
    }
    response.norm()
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

#[cfg(test)]
#[path = "lockin_core_tests.rs"]
mod tests;
