//! Phase-aware joint harmonic GLS execution (WP-5C).
//!
//! This module runs the joint estimator over the same support grid as the
//! legacy boxcar path and returns boxcar-shaped outputs so downstream
//! phase/MOKE stages work unchanged. Noise models arrive through a
//! [`NoiseModelSource`]; only synthetic sources exist in this slice, so the
//! file-backed calibration loader (WP-5B) plugs in without touching the
//! engine.

use crate::config::{GlsNoiseMode, JointHarmonicGlsConfig, Lockin};
use crate::lockin::lockin_params::LockinParams;
use crate::lockin::provenance::LockinProvenance;
use crate::utils::time_axis::TimeAxisRef;
use anyhow::{Context, Result, bail};
use pmoke_analysis_core::{
    CorrelationKernel, HarmonicSignalModel, JointEstimate, JointHarmonicSettings,
    JointSolverTolerances, NoiseModel, estimate_joint,
};

/// Solver identity recorded in GLS provenance (FR-045).
pub const JOINT_SOLVER_ID: &str = "joint-direct-qr/1";

/// Immutable model binding receipt: which artifact bytes back a channel.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelBinding {
    pub channel: u8,
    pub path: String,
    pub sha256: String,
    pub noise_mode: GlsNoiseMode,
}

/// Noise-model provider seam. The file-backed calibration loader arrives in
/// WP-5B; tests and calibration-free paths use synthetic sources.
pub trait NoiseModelSource {
    fn load(&self, channel: u8, noise_mode: GlsNoiseMode) -> Result<(NoiseModel, ModelBinding)>;
}

/// Synthetic in-memory noise models for tests and calibration-free checks.
/// Missing per-mode data is an explicit error, never an implicit fallback.
#[derive(Debug, Clone)]
pub struct SyntheticNoiseModelSource {
    pub reference_variance_v2: f64,
    pub variance_bins: Option<Vec<f64>>,
    pub correlation_lags: Option<Vec<f64>>,
    pub correlation_lag_step_s: f64,
    pub label: String,
}

impl SyntheticNoiseModelSource {
    pub fn identity(reference_variance_v2: f64) -> Self {
        Self {
            reference_variance_v2,
            variance_bins: None,
            correlation_lags: None,
            correlation_lag_step_s: 1.0,
            label: "synthetic-identity".to_string(),
        }
    }
}

impl NoiseModelSource for SyntheticNoiseModelSource {
    fn load(&self, channel: u8, noise_mode: GlsNoiseMode) -> Result<(NoiseModel, ModelBinding)> {
        if !self.reference_variance_v2.is_finite() || self.reference_variance_v2 <= 0.0 {
            bail!(
                "synthetic noise source '{}' has non-positive reference variance (got {})",
                self.label,
                self.reference_variance_v2
            );
        }
        let binding = ModelBinding {
            channel,
            path: format!("synthetic:{}:ch{channel}", self.label),
            sha256: "synthetic".to_string(),
            noise_mode,
        };
        let model = match noise_mode {
            GlsNoiseMode::Identity => NoiseModel {
                mode: pmoke_analysis_core::NoiseMode::Identity,
                reference_variance_v2: self.reference_variance_v2,
                variance_bins: None,
                correlation: None,
            },
            GlsNoiseMode::PhaseDiagonal => {
                let bins = self.variance_bins.clone().ok_or_else(|| {
                    anyhow::anyhow!(
                        "synthetic noise source '{}' has no variance bins for phase_diagonal mode",
                        self.label
                    )
                })?;
                NoiseModel {
                    mode: pmoke_analysis_core::NoiseMode::PhaseDiagonal,
                    reference_variance_v2: self.reference_variance_v2,
                    variance_bins: Some(bins),
                    correlation: None,
                }
            }
            GlsNoiseMode::StationaryCorrelated | GlsNoiseMode::PhaseCorrelated => {
                let lags = self.correlation_lags.clone().ok_or_else(|| {
                    anyhow::anyhow!(
                        "synthetic noise source '{}' has no correlation kernel for correlated mode",
                        self.label
                    )
                })?;
                let bins = if matches!(noise_mode, GlsNoiseMode::PhaseCorrelated) {
                    Some(self.variance_bins.clone().ok_or_else(|| {
                        anyhow::anyhow!(
                            "synthetic noise source '{}' has no variance bins for phase_correlated mode",
                            self.label
                        )
                    })?)
                } else {
                    None
                };
                NoiseModel {
                    mode: if matches!(noise_mode, GlsNoiseMode::StationaryCorrelated) {
                        pmoke_analysis_core::NoiseMode::StationaryCorrelated
                    } else {
                        pmoke_analysis_core::NoiseMode::PhaseCorrelated
                    },
                    reference_variance_v2: self.reference_variance_v2,
                    variance_bins: bins,
                    correlation: Some(CorrelationKernel {
                        lags,
                        lag_step_s: self.correlation_lag_step_s,
                    }),
                }
            }
        };
        Ok((model, binding))
    }
}

/// One quality row per executed window (FR-044 diagnostics, AT-025 shape).
#[derive(Debug, Clone, PartialEq)]
pub struct QualityRow {
    pub time_s: f64,
    pub residual_rms: f64,
    pub rank: usize,
    pub condition: f64,
    pub jitter_applied_v2: f64,
    pub noise_mode: &'static str,
}

/// Shared inputs for a joint lock-in execution (keeps the engine entry
/// points within the argument-count lint).
pub struct JointRunInputs<'a> {
    pub lockin: &'a Lockin,
    pub gls: &'a JointHarmonicGlsConfig,
    pub t: TimeAxisRef<'a>,
    pub f_ref: f64,
    pub omega_tref: f64,
    pub sample_rate: f64,
}

/// Joint lock-in outputs in boxcar-compatible layout.
///
/// The estimator always computes the full design-model covariance; the
/// none/diagonal/full serialization modes land with the covariance artifacts
/// in the next slice, so no information is dropped here.
pub struct JointRunOutput {
    /// Per channel `[x_h1, y_h1, ..., x_h6, y_h6]` over the shared output grid.
    pub result: Vec<Vec<Vec<f64>>>,
    /// Per-channel quality rows aligned with the output grid.
    pub quality: Vec<Vec<QualityRow>>,
    pub provenance: LockinProvenance,
    pub base_index_range: (usize, usize),
    pub output_index_range: (usize, usize),
}

/// Writes per-window quality diagnostics as a typed heterogeneous CSV
/// (AT-025). CSV only: no NPY sidecar is ever written, and existing files
/// are never overwritten (a partial write must not masquerade as complete).
pub fn write_quality_csv(path: &std::path::Path, rows: &[QualityRow]) -> Result<()> {
    if path.exists() {
        bail!("joint quality output already exists: {}", path.display());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("failed to create quality output dir: {}", parent.display())
        })?;
    }
    let file = std::fs::File::create(path)
        .with_context(|| format!("failed to create quality output: {}", path.display()))?;
    let mut writer = csv::WriterBuilder::new()
        .has_headers(true)
        .from_writer(file);
    writer
        .write_record([
            "time_s",
            "residual_rms",
            "rank",
            "condition",
            "jitter_applied_v2",
            "noise_mode",
        ])
        .context("failed to write quality header")?;
    for row in rows {
        writer
            .write_record([
                row.time_s.to_string(),
                row.residual_rms.to_string(),
                row.rank.to_string(),
                row.condition.to_string(),
                row.jitter_applied_v2.to_string(),
                row.noise_mode.to_string(),
            ])
            .with_context(|| format!("failed to write quality row at t={}", row.time_s))?;
    }
    writer.flush().context("failed to flush quality output")?;
    Ok(())
}

/// Typed estimator failure context (FR-044): code, stage, channel, window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlsFailure {
    pub code: String,
    pub channel: u8,
    pub window_index: usize,
}

impl std::fmt::Display for GlsFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "joint_harmonic_gls estimation failed (code={}, stage=lockin, channel={}, window={})",
            self.code, self.channel, self.window_index
        )
    }
}

pub fn run_joint_li(
    inputs: &JointRunInputs<'_>,
    signal_ch: &[u8],
    signal_data: &[&[f64]],
    source: &dyn NoiseModelSource,
) -> Result<JointRunOutput> {
    let JointRunInputs {
        lockin,
        gls,
        t,
        f_ref,
        sample_rate: sample_rate_hz,
        ..
    } = *inputs;
    if signal_ch.len() != signal_data.len() {
        bail!(
            "signal channel count ({}) and signal data column count ({}) differ",
            signal_ch.len(),
            signal_data.len()
        );
    }
    if signal_data.is_empty() {
        bail!("no signal channels were available for joint lock-in processing");
    }
    let params = LockinParams::from_geometry(t.len(), sample_rate_hz.recip(), f_ref, lockin)
        .context("joint lock-in geometry failed")?;
    validate_joint_output_range(params, lockin)?;
    let model = HarmonicSignalModel {
        fit_harmonics: gls.fit_harmonics.clone(),
        output_harmonics: gls.output_harmonics.clone(),
        envelope_degree: gls.envelope_degree,
    };

    let mut result = Vec::with_capacity(signal_data.len());
    let mut quality = Vec::with_capacity(signal_data.len());
    let mut bindings = Vec::with_capacity(signal_data.len());
    for (&channel, signal) in signal_ch.iter().zip(signal_data.iter()) {
        let signal: &[f64] = signal;
        let (noise, binding) = source
            .load(channel, gls.noise_mode)
            .with_context(|| format!("joint lock-in model loading failed for channel {channel}"))?;
        bindings.push(binding);
        let settings = JointHarmonicSettings {
            model: model.clone(),
            noise,
            tolerances: JointSolverTolerances::default(),
        };
        let (columns, rows) = run_joint_channel(inputs, signal, channel, &params, &settings)?;
        result.push(columns);
        quality.push(rows);
    }

    let provenance =
        LockinProvenance::from_joint(params, gls.noise_mode, &bindings, JOINT_SOLVER_ID);
    Ok(JointRunOutput {
        result,
        quality,
        provenance,
        base_index_range: (params.i_start, params.i_end),
        output_index_range: (params.i_start, params.i_end),
    })
}

fn run_joint_channel(
    inputs: &JointRunInputs<'_>,
    signal: &[f64],
    channel: u8,
    params: &LockinParams,
    settings: &JointHarmonicSettings,
) -> Result<(Vec<Vec<f64>>, Vec<QualityRow>)> {
    let JointRunInputs {
        gls,
        t,
        f_ref,
        omega_tref,
        sample_rate: sample_rate_hz,
        ..
    } = *inputs;
    let outputs = params.i_end - params.i_start + 1;
    let mut columns: Vec<Vec<f64>> = (0..12).map(|_| Vec::with_capacity(outputs)).collect();
    let mut rows = Vec::with_capacity(outputs);
    // Same tap support as the legacy boxcar (edge legacy_trim): centers on
    // the strided grid, samples [center - n_half - 1, center + n_half + 1].
    let half_taps = params.n_half + 1;
    for (output_index, center_k) in (params.i_start..=params.i_end).enumerate() {
        let center = center_k * params.stride;
        let lo = center.checked_sub(half_taps).ok_or_else(|| {
            anyhow::anyhow!("joint lock-in window underflows for output {output_index}")
        })?;
        let hi = center + half_taps;
        let window_times: Vec<f64> = (lo..=hi).map(|index| t.value_at(index)).collect();
        let window_signal: Vec<f64> = signal
            .get(lo..=hi)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "joint lock-in window [{lo}..={hi}] exceeds signal length {}",
                    signal.len()
                )
            })?
            .to_vec();
        let estimate: JointEstimate = estimate_joint(
            &window_times,
            &window_signal,
            f_ref,
            omega_tref,
            sample_rate_hz,
            settings,
        )
        .map_err(|error| {
            anyhow::anyhow!(GlsFailure {
                code: error.code().to_string(),
                channel,
                window_index: output_index,
            })
        })
        .with_context(|| {
            format!(
                "joint_harmonic_gls estimation failed (stage=lockin, channel={channel}, window={output_index}); refusing to fall back"
            )
        })?;
        if estimate.xy.len() != 12 {
            bail!(
                "joint estimator returned {} quadratures, expected 12 (channel={channel}, window={output_index})",
                estimate.xy.len()
            );
        }
        for (harmonic, column_pair) in columns.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            column_pair[0].push(estimate.xy[2 * harmonic]);
            column_pair[1].push(estimate.xy[2 * harmonic + 1]);
        }
        rows.push(QualityRow {
            time_s: t.value_at(center),
            residual_rms: estimate.residual_rms,
            rank: estimate.rank,
            condition: estimate.condition,
            jitter_applied_v2: estimate.jitter_applied_v2,
            noise_mode: gls_noise_mode_name(gls.noise_mode),
        });
    }
    Ok((columns, rows))
}

/// Canonical GLS noise-mode name for quality rows and provenance.
pub(crate) fn gls_noise_mode_name(mode: GlsNoiseMode) -> &'static str {
    match mode {
        GlsNoiseMode::Identity => "identity",
        GlsNoiseMode::PhaseDiagonal => "phase_diagonal",
        GlsNoiseMode::StationaryCorrelated => "stationary_correlated",
        GlsNoiseMode::PhaseCorrelated => "phase_correlated",
    }
}

#[cfg(test)]
#[path = "joint_tests.rs"]
mod tests;

fn validate_joint_output_range(params: LockinParams, lockin: &Lockin) -> Result<()> {
    let (base_start, base_end) = (params.i_start, params.i_end);
    if base_start <= base_end {
        return Ok(());
    }
    Err(anyhow::anyhow!(
        "joint lock-in output range is empty after edge trimming: base_index_range=({base_start}, {base_end}); reduce lockin.window.half_window_cycles or use a longer trace (half_window_cycles={})",
        lockin.window.half_window_cycles
    ))
}
