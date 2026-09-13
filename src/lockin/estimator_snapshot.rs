//! Per-channel estimator snapshot (WP-5C/3a, INTERFACES 5.1a).
//!
//! `lockin/ch{channel}_estimator.json` freezes everything a staged rerun
//! needs to interpret or reconstruct uncertainty without refitting: solver
//! identity and effective tolerances, signal/noise model bytes (variances,
//! lags) plus the binding digest, reference, geometry/support, quality
//! summary, and the conditional covariance interpretation. It never claims
//! unconditional uncertainty: conditioning on the frozen reference,
//! calibration, and excluded phase/depth fits is stated explicitly.

use crate::config::{GlsCovarianceOutput, JointHarmonicGlsConfig};
use crate::lockin::joint::{ModelBinding, QualityRow, WindowStatus, gls_noise_mode_name};
use crate::lockin::lockin_params::LockinParams;
use anyhow::{Context, Result, bail};
use pmoke_analysis_core::joint::{JointSolverTolerances, NoiseModel};
use serde::Serialize;

/// Estimator snapshot schema version.
pub const ESTIMATOR_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SolverPolicy {
    pub id: String,
    pub rank_tol: f64,
    pub max_condition: f64,
    pub max_noise_condition: f64,
    pub max_jitter_v2: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SignalModelSnapshot {
    pub fit_harmonics: Vec<usize>,
    pub output_harmonics: Vec<usize>,
    pub envelope_degree: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NoiseModelSnapshot {
    pub mode: String,
    pub reference_variance_v2: f64,
    pub variance_bins_v2: Option<Vec<f64>>,
    pub correlation_lags: Option<Vec<f64>>,
    pub correlation_lag_step_s: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelReceipt {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReferenceSnapshot {
    pub frequency_hz: f64,
    pub phase_rad: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GeometrySnapshot {
    pub sample_interval_s: f64,
    pub half_window_taps: usize,
    pub stride_samples: usize,
    pub base_index_start: usize,
    pub base_index_end: usize,
    pub output_index_start: usize,
    pub output_index_end: usize,
    pub output_count: usize,
    pub first_time_s: f64,
    pub last_time_s: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct QualitySummary {
    pub windows: usize,
    pub ok: usize,
    pub warnings: usize,
    pub warning_centers: Vec<usize>,
    pub max_jitter_applied_v2: f64,
    pub max_scaled_design_condition: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CovarianceInterpretation {
    pub covariance_mode: String,
    pub serialization: String,
    pub conditioning: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct EstimatorSnapshot {
    pub schema_version: u32,
    pub channel: u8,
    pub solver: SolverPolicy,
    pub signal_model: SignalModelSnapshot,
    pub noise_model: NoiseModelSnapshot,
    pub model: ModelReceipt,
    pub reference: ReferenceSnapshot,
    pub geometry: GeometrySnapshot,
    pub quality_summary: QualitySummary,
    pub covariance: CovarianceInterpretation,
    /// Fatal windows abort publication, so a published snapshot carries no
    /// concealed errors; the empty list says so explicitly.
    pub errors: Vec<String>,
}

fn require_finite(name: &str, value: f64) -> Result<()> {
    if value.is_finite() {
        return Ok(());
    }
    bail!("estimator snapshot field {name} is non-finite: {value}")
}

/// Builds the per-channel snapshot from frozen execution context. Every
/// value comes from the executed run; nothing is re-derived or guessed.
#[allow(clippy::too_many_arguments)]
pub fn build_estimator_snapshot(
    channel: u8,
    gls: &JointHarmonicGlsConfig,
    noise: &NoiseModel,
    tolerances: &JointSolverTolerances,
    solver_id: &str,
    f_ref: f64,
    omega_tref: f64,
    sample_interval_s: f64,
    params: &LockinParams,
    binding: &ModelBinding,
    quality: &[QualityRow],
) -> Result<EstimatorSnapshot> {
    require_finite("reference_frequency_hz", f_ref)?;
    require_finite("reference_phase_rad", omega_tref)?;
    require_finite("sample_interval_s", sample_interval_s)?;
    require_finite(
        "noise_model.reference_variance_v2",
        noise.reference_variance_v2,
    )?;
    let mut warnings = 0usize;
    let mut warning_centers = Vec::new();
    let mut max_jitter = 0.0f64;
    let mut max_condition = 0.0f64;
    for row in quality {
        require_finite("quality.time_s", row.time_s)?;
        require_finite(
            "quality.scaled_design_condition",
            row.scaled_design_condition,
        )?;
        require_finite("quality.residual_rms_v", row.residual_rms_v)?;
        require_finite("quality.jitter_applied_v2", row.jitter_applied_v2)?;
        if row.status == WindowStatus::Warning {
            warnings += 1;
            warning_centers.push(row.original_center_index);
        }
        max_jitter = max_jitter.max(row.jitter_applied_v2);
        max_condition = max_condition.max(row.scaled_design_condition);
    }
    let (first_time_s, last_time_s) = match (quality.first(), quality.last()) {
        (Some(first), Some(last)) => (first.time_s, last.time_s),
        (None, None) => bail!("estimator snapshot needs at least one quality row"),
        _ => unreachable!("first/last come from the same slice"),
    };
    let outputs = params.i_end - params.i_start + 1;
    if quality.len() != outputs {
        bail!(
            "quality rows ({}) do not match the output grid ({outputs})",
            quality.len()
        );
    }
    let serialization = match gls.covariance_output {
        GlsCovarianceOutput::None => "none",
        GlsCovarianceOutput::Diagonal => "diagonal",
        GlsCovarianceOutput::Full => "full",
    };
    Ok(EstimatorSnapshot {
        schema_version: ESTIMATOR_SNAPSHOT_SCHEMA_VERSION,
        channel,
        solver: SolverPolicy {
            id: solver_id.to_string(),
            rank_tol: tolerances.rank_tol,
            max_condition: tolerances.max_condition,
            max_noise_condition: tolerances.max_noise_condition,
            max_jitter_v2: tolerances.max_jitter_v2,
        },
        signal_model: SignalModelSnapshot {
            fit_harmonics: gls.fit_harmonics.clone(),
            output_harmonics: gls.output_harmonics.clone(),
            envelope_degree: gls.envelope_degree,
        },
        noise_model: NoiseModelSnapshot {
            mode: gls_noise_mode_name(gls.noise_mode).to_string(),
            reference_variance_v2: noise.reference_variance_v2,
            variance_bins_v2: noise.variance_bins.clone(),
            correlation_lags: noise.correlation.as_ref().map(|kernel| kernel.lags.clone()),
            correlation_lag_step_s: noise.correlation.as_ref().map(|kernel| kernel.lag_step_s),
        },
        model: ModelReceipt {
            path: binding.path.clone(),
            sha256: binding.sha256.clone(),
        },
        reference: ReferenceSnapshot {
            frequency_hz: f_ref,
            phase_rad: omega_tref,
        },
        geometry: GeometrySnapshot {
            sample_interval_s,
            half_window_taps: params.n_half + 1,
            stride_samples: params.stride,
            base_index_start: params.i_start,
            base_index_end: params.i_end,
            output_index_start: params.i_start,
            output_index_end: params.i_end,
            output_count: outputs,
            first_time_s,
            last_time_s,
        },
        quality_summary: QualitySummary {
            windows: quality.len(),
            ok: quality.len() - warnings,
            warnings,
            warning_centers,
            max_jitter_applied_v2: max_jitter,
            max_scaled_design_condition: max_condition,
        },
        covariance: CovarianceInterpretation {
            covariance_mode: "design_model".to_string(),
            serialization: serialization.to_string(),
            conditioning: format!(
                "conditional design-model covariance: conditioned on frozen reference \
                 (f_ref={f_ref} Hz), calibration bytes (sha256={}), and the fitted \
                 global phase/depth excluded from this budget; unknown uncertainty \
                 is not zero",
                binding.sha256
            ),
        },
        errors: Vec::new(),
    })
}

/// Writes one snapshot as pretty JSON. Existing files are never
/// overwritten; non-finite values cannot serialize and abort instead.
pub fn write_estimator_json(path: &std::path::Path, snapshot: &EstimatorSnapshot) -> Result<()> {
    if path.exists() {
        bail!("estimator snapshot already exists: {}", path.display());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create estimator snapshot dir: {}",
                parent.display()
            )
        })?;
    }
    let text = serde_json::to_string_pretty(snapshot)
        .with_context(|| format!("failed to encode estimator snapshot: {}", path.display()))?;
    std::fs::write(path, format!("{text}\n"))
        .with_context(|| format!("failed to write estimator snapshot: {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
#[path = "estimator_snapshot_tests.rs"]
mod tests;
