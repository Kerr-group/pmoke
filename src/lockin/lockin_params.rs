use crate::config::{Lockin, LockinLpfKind};
use crate::ui;
use anyhow::{Result, anyhow, bail};
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy)]
pub struct LockinParams {
    pub dt: f64,
    pub sample_rate: f64,
    pub output_rate: f64,
    pub stride: usize,
    #[allow(dead_code)]
    pub length: usize,
    #[allow(dead_code)]
    pub f_ref: f64,
    pub lpf_kind: LockinLpfKind,
    pub lpf_stopband_atten_db: f64,
    pub lpf_sync_average_cycles: f64,
    pub lpf_iir_order: usize,
    pub cutoff_hz: Option<f64>,
    pub cutoff_source: CutoffSource,
    pub fallback_used: bool,

    pub omega: f64,
    pub t_half: f64,
    pub n_half: usize,
    pub i_start: usize,
    pub i_end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutoffSource {
    ExplicitHz,
    ReferenceRatio,
    Fallback,
    EnbwMatch,
    None,
}

impl CutoffSource {
    pub fn as_str(self) -> &'static str {
        match self {
            CutoffSource::ExplicitHz => "explicit_hz",
            CutoffSource::ReferenceRatio => "reference_ratio",
            CutoffSource::Fallback => "fallback",
            CutoffSource::EnbwMatch => "enbw_match",
            CutoffSource::None => "none",
        }
    }
}

impl LockinParams {
    fn init(dt: f64, length: usize, f_ref: f64, lockin: &Lockin) -> Result<Self> {
        if !dt.is_finite() || dt <= 0.0 {
            bail!("lockin dt must be positive and finite (got {dt})");
        }
        if !f_ref.is_finite() || f_ref <= 0.0 {
            bail!("lockin f_ref must be positive and finite (got {f_ref})");
        }
        let stride = lockin.stride_samples;
        let sample_rate = 1.0 / dt;
        let output_rate = sample_rate / stride as f64;
        let half_window_cycles = lockin.lpf_half_window_cycles;
        let omega = 2.0 * PI * f_ref;
        let t_half = half_window_cycles / f_ref;
        if !t_half.is_finite() || t_half < dt {
            bail!(
                "lockin half-window ({t_half}) must be finite and >= dt ({dt}); increase lockin.lpf_half_window_cycles or sampling resolution"
            );
        }
        let n_half = ((t_half / dt).floor() as usize).max(1);
        let n_int = ((length - 1) / stride) + 1;
        let i_start = 2 + (n_half + 1) / stride;
        let i_end = n_int.saturating_sub(i_start);
        let (cutoff_hz, cutoff_source, fallback_used) = resolve_cutoff_hz(lockin, f_ref, t_half)?;

        if let Some(cutoff_hz) = cutoff_hz {
            if cutoff_hz >= 0.45 * output_rate {
                bail!(
                    "lockin cutoff_hz ({}) must be < 0.45 * output_rate ({})",
                    cutoff_hz,
                    0.45 * output_rate
                );
            }
            if cutoff_hz >= 0.4 * output_rate {
                ui::warn(format!(
                    "lockin cutoff_hz ({cutoff_hz}) is close to output Nyquist; output_rate={output_rate}"
                ));
            }
            if lockin.lpf_kind == LockinLpfKind::SyncIirZeroPhase {
                let design_cutoff_hz =
                    zero_phase_butterworth_design_cutoff(cutoff_hz, lockin.lpf_iir_order);
                if design_cutoff_hz >= 0.45 * sample_rate {
                    bail!(
                        "sync_iir_zero_phase design_cutoff_hz ({}) must be < 0.45 * sample_rate ({})",
                        design_cutoff_hz,
                        0.45 * sample_rate
                    );
                }
            }
        }

        Ok(Self {
            dt,
            sample_rate,
            output_rate,
            stride,
            length,
            f_ref,
            lpf_kind: lockin.lpf_kind,
            lpf_stopband_atten_db: lockin.lpf_stopband_atten_db,
            lpf_sync_average_cycles: lockin.lpf_sync_average_cycles,
            lpf_iir_order: lockin.lpf_iir_order,
            cutoff_hz,
            cutoff_source,
            fallback_used,
            omega,
            t_half,
            n_half,
            i_start,
            i_end,
        })
    }

    pub fn from_slice(t: &[f64], f_ref: f64, lockin: &Lockin) -> Result<Self> {
        if t.len() < 2 {
            return Err(anyhow!("Time slice 't' must have at least 2 elements"));
        }
        let dt = t[1] - t[0];
        let length = t.len();

        Self::from_geometry(length, dt, f_ref, lockin)
    }

    pub(crate) fn from_geometry(
        length: usize,
        dt: f64,
        f_ref: f64,
        lockin: &Lockin,
    ) -> Result<Self> {
        if length < 2 {
            return Err(anyhow!("Time axis must have at least 2 elements"));
        }
        Self::init(dt, length, f_ref, lockin)
    }
}

#[cfg(test)]
#[path = "lockin_params_tests.rs"]
mod tests;

fn zero_phase_butterworth_design_cutoff(target_cutoff_hz: f64, order: usize) -> f64 {
    target_cutoff_hz / (2.0_f64.sqrt() - 1.0).powf(1.0 / (2.0 * order as f64))
}

fn resolve_cutoff_hz(
    lockin: &Lockin,
    f_ref: f64,
    t_half: f64,
) -> Result<(Option<f64>, CutoffSource, bool)> {
    match lockin.lpf_kind {
        LockinLpfKind::BoxcarLegacy => Ok((None, CutoffSource::None, false)),
        LockinLpfKind::FirBoxcarEnbw => Ok((None, CutoffSource::EnbwMatch, false)),
        LockinLpfKind::FirZeroPhase | LockinLpfKind::SyncIirZeroPhase => {
            if let Some(cutoff_hz) = lockin.lpf_cutoff_hz {
                Ok((Some(cutoff_hz), CutoffSource::ExplicitHz, false))
            } else if let Some(ratio) = lockin.lpf_cutoff_ref_ratio {
                Ok((Some(ratio * f_ref), CutoffSource::ReferenceRatio, false))
            } else {
                Ok((Some(0.5 / t_half), CutoffSource::Fallback, true))
            }
        }
    }
}
