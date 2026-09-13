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
use serde::{Deserialize, Serialize};

/// Estimator snapshot schema version.
pub const ESTIMATOR_SNAPSHOT_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolverPolicy {
    pub id: String,
    pub rank_tol: f64,
    pub max_condition: f64,
    pub max_noise_condition: f64,
    pub max_jitter_v2: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignalModelSnapshot {
    pub fit_harmonics: Vec<usize>,
    pub output_harmonics: Vec<usize>,
    pub envelope_degree: u8,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NoiseModelSnapshot {
    pub mode: String,
    pub reference_variance_v2: f64,
    pub variance_bins_v2: Option<Vec<f64>>,
    pub correlation_lags: Option<Vec<f64>>,
    pub correlation_lag_step_s: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelReceipt {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceSnapshot {
    pub frequency_hz: f64,
    pub phase_rad: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QualitySummary {
    pub windows: usize,
    pub ok: usize,
    pub warnings: usize,
    pub warning_centers: Vec<usize>,
    pub max_jitter_applied_v2: f64,
    pub max_scaled_design_condition: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CovarianceInterpretation {
    pub covariance_mode: String,
    pub serialization: String,
    pub conditioning: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

/// Validates a parsed estimator snapshot before manifest registration
/// (AT-025): JSON syntax is not enough — the snapshot must carry the
/// expected schema version and channel, complete solver/signal/noise
/// geometry records with finite in-domain values, a well-formed model
/// receipt, and internally consistent quality/covariance summaries.
/// Checksum verification stays a separate manifest concern; this decides
/// structural validity. A snapshot carrying errors is never registrable:
/// publication aborts on fatal windows, so errors must be empty here.
pub fn validate_estimator_snapshot(
    snapshot: &EstimatorSnapshot,
    expected_channel: u8,
) -> Result<()> {
    let here = |what: &str| format!("estimator snapshot for channel {expected_channel} has {what}");
    if snapshot.schema_version != ESTIMATOR_SNAPSHOT_SCHEMA_VERSION {
        bail!(
            "{}",
            here(&format!(
                "unsupported schema_version {} (expected {ESTIMATOR_SNAPSHOT_SCHEMA_VERSION})",
                snapshot.schema_version
            ))
        );
    }
    if snapshot.channel != expected_channel {
        bail!(
            "{}",
            here(&format!(
                "channel {} disagrees with the snapshot filename",
                snapshot.channel
            ))
        );
    }
    if snapshot.solver.id.is_empty() {
        bail!("{}", here("an empty solver id"));
    }
    for (name, value) in [
        ("rank_tol", snapshot.solver.rank_tol),
        ("max_condition", snapshot.solver.max_condition),
        ("max_noise_condition", snapshot.solver.max_noise_condition),
    ] {
        if !value.is_finite() || value <= 0.0 {
            bail!("{}", here(&format!("non-positive solver {name}")));
        }
    }
    if !snapshot.solver.max_jitter_v2.is_finite() || snapshot.solver.max_jitter_v2 < 0.0 {
        bail!("{}", here("negative solver max_jitter_v2"));
    }
    if snapshot.signal_model.fit_harmonics.is_empty()
        || snapshot.signal_model.fit_harmonics.iter().any(|h| *h == 0)
    {
        bail!("{}", here("an empty or zero-containing fit harmonic list"));
    }
    for output in snapshot.signal_model.output_harmonics.iter() {
        if !snapshot.signal_model.fit_harmonics.contains(output) {
            bail!(
                "{}",
                here(&format!("output harmonic {output} outside the fit list"))
            );
        }
    }
    if snapshot.signal_model.envelope_degree != 0 {
        bail!("{}", here("a nonzero envelope degree"));
    }
    const MODES: [&str; 4] = [
        "identity",
        "phase_diagonal",
        "stationary_correlated",
        "phase_correlated",
    ];
    if !MODES.contains(&snapshot.noise_model.mode.as_str()) {
        bail!(
            "{}",
            here(&format!(
                "unknown noise mode {:?}",
                snapshot.noise_model.mode
            ))
        );
    }
    if !snapshot.noise_model.reference_variance_v2.is_finite()
        || snapshot.noise_model.reference_variance_v2 <= 0.0
    {
        bail!("{}", here("a non-positive reference variance"));
    }
    let needs_bins = matches!(
        snapshot.noise_model.mode.as_str(),
        "phase_diagonal" | "phase_correlated"
    );
    match (&snapshot.noise_model.variance_bins_v2, needs_bins) {
        (Some(bins), true) => {
            if bins.len() < 2 || bins.iter().any(|v| !v.is_finite() || *v <= 0.0) {
                bail!("{}", here("an invalid phase variance table"));
            }
        }
        (None, false) => {}
        _ => bail!(
            "{}",
            here("a variance table inconsistent with the noise mode")
        ),
    }
    let needs_kernel = matches!(
        snapshot.noise_model.mode.as_str(),
        "stationary_correlated" | "phase_correlated"
    );
    match (
        &snapshot.noise_model.correlation_lags,
        &snapshot.noise_model.correlation_lag_step_s,
        needs_kernel,
    ) {
        (Some(lags), Some(step), true) => {
            if lags.is_empty()
                || lags.iter().any(|v| !v.is_finite())
                || (lags[0] - 1.0).abs() > 1e-9
                || !step.is_finite()
                || *step <= 0.0
            {
                bail!("{}", here("an invalid correlation kernel"));
            }
        }
        (None, None, false) => {}
        _ => bail!(
            "{}",
            here("a correlation kernel inconsistent with the noise mode")
        ),
    }
    if snapshot.model.path.is_empty() {
        bail!("{}", here("an empty model path"));
    }
    if snapshot.model.sha256.len() != 64
        || !snapshot
            .model
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        bail!("{}", here("a malformed model digest"));
    }
    if !snapshot.reference.frequency_hz.is_finite() || snapshot.reference.frequency_hz <= 0.0 {
        bail!("{}", here("a non-positive reference frequency"));
    }
    if !snapshot.reference.phase_rad.is_finite() {
        bail!("{}", here("a non-finite reference phase"));
    }
    let geometry = &snapshot.geometry;
    if !geometry.sample_interval_s.is_finite() || geometry.sample_interval_s <= 0.0 {
        bail!("{}", here("a non-positive sample interval"));
    }
    if geometry.stride_samples == 0 || geometry.half_window_taps == 0 {
        bail!("{}", here("a zero stride or window tap count"));
    }
    if geometry.base_index_end < geometry.base_index_start
        || geometry.output_index_end < geometry.output_index_start
    {
        bail!("{}", here("an inverted index range"));
    }
    if geometry.output_count == 0
        || geometry.output_count != geometry.output_index_end - geometry.output_index_start + 1
    {
        bail!(
            "{}",
            here("an output count inconsistent with the output range")
        );
    }
    if !geometry.first_time_s.is_finite()
        || !geometry.last_time_s.is_finite()
        || geometry.last_time_s < geometry.first_time_s
    {
        bail!("{}", here("a non-monotonic time span"));
    }
    let quality = &snapshot.quality_summary;
    if quality.windows == 0 || quality.ok + quality.warnings != quality.windows {
        bail!(
            "{}",
            here("a quality summary inconsistent with its window count")
        );
    }
    if !quality.max_jitter_applied_v2.is_finite()
        || !quality.max_scaled_design_condition.is_finite()
    {
        bail!("{}", here("non-finite quality maxima"));
    }
    if snapshot.covariance.covariance_mode.is_empty()
        || snapshot.covariance.serialization.is_empty()
        || snapshot.covariance.conditioning.is_empty()
    {
        bail!("{}", here("an empty covariance interpretation"));
    }
    if !snapshot.errors.is_empty() {
        bail!("{}", here("carried errors; only clean snapshots register"));
    }
    Ok(())
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
