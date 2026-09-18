//! Phase-aware joint harmonic GLS execution (WP-5C).
//!
//! This module runs the joint estimator over the same support grid as the
//! legacy boxcar path and returns boxcar-shaped outputs so downstream
//! phase/MOKE stages work unchanged. Noise models arrive through a
//! [`NoiseModelSource`]; production uses the file-backed calibration loader
//! ([`crate::lockin::model_loading`]), tests use synthetic sources.

use crate::config::{GlsCovarianceOutput, GlsNoiseMode, JointHarmonicGlsConfig, Lockin};
use crate::lockin::estimator_snapshot::{EstimatorSnapshot, build_estimator_snapshot};
use crate::lockin::lockin_params::LockinParams;
use crate::lockin::provenance::LockinProvenance;
use crate::utils::time_axis::TimeAxisRef;
use anyhow::{Context, Result, bail};
use pmoke_analysis_core::{
    CorrelationKernel, HarmonicSignalModel, JointEstimate, JointHarmonicSettings,
    JointSolverTolerances, NoiseModel, estimate_joint,
};
use rayon::prelude::*;

/// Upper bound for temporary per-window allocations during native execution.
/// Final artifacts remain ordered and compatible, while a large record never
/// creates one temporary vector for the entire output grid.
const JOINT_OUTPUT_CHUNK: usize = 256;

/// Solver identity recorded in GLS provenance (FR-045).
pub const JOINT_SOLVER_ID: &str = "joint-direct-qr/1";

/// Immutable model binding receipt: which artifact bytes back a channel.
/// `bytes` carries the exact validated artifact bytes the loader hashed
/// (v4 retention); `None` for synthetic sources that hold no file bytes.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelBinding {
    pub channel: u8,
    pub path: String,
    pub sha256: String,
    pub noise_mode: GlsNoiseMode,
    pub bytes: Option<Vec<u8>>,
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
            bytes: None,
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

/// One quality row per executed window (INTERFACES section 5 contract).
///
/// The CSV column set is fixed: `original_center_index,time_s,
/// scaled_design_condition,residual_rms_v,rank,status`. `original_center_index`
/// is the original-sample center index (consistent with `time_s`); artifacts
/// written before this convention carry the decimated output counter instead
/// and must be interpreted under that old convention. `jitter_applied_v2`
/// and `noise_mode` ride along in memory for the estimator snapshot (next
/// slice) but are never CSV columns: a window that needed regularization is
/// flagged `warning`, and fatal windows abort publication instead of
/// becoming shortened tables.
#[derive(Debug, Clone, PartialEq)]
pub struct QualityRow {
    pub original_center_index: usize,
    pub time_s: f64,
    pub scaled_design_condition: f64,
    pub residual_rms_v: f64,
    pub rank: usize,
    pub status: WindowStatus,
    pub jitter_applied_v2: f64,
    pub noise_mode: &'static str,
}

/// Per-window execution status: `ok`, or `warning` when the solver applied
/// nonzero regularization jitter (the value itself is recorded, never
/// hidden, and lands in the estimator snapshot).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowStatus {
    Ok,
    Warning,
}

impl WindowStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            WindowStatus::Ok => "ok",
            WindowStatus::Warning => "warning",
        }
    }

    pub fn for_jitter(jitter_applied_v2: f64) -> Self {
        if jitter_applied_v2 > 0.0 {
            WindowStatus::Warning
        } else {
            WindowStatus::Ok
        }
    }
}

/// Legacy-order quadrature names `[X1,Y1,...,X6,Y6]` for covariance headers.
pub const QUADRATURE_NAMES: [&str; 12] = [
    "x1", "y1", "x2", "y2", "x3", "y3", "x4", "y4", "x5", "y5", "x6", "y6",
];

/// Per-quadrature columns over the output grid: `[x_h1, y_h1, ..., x_h6, y_h6]`.
pub type XyColumns = Vec<Vec<f64>>;
/// Per-output 12x12 design-model covariance matrices in XY order.
pub type XyCovariances = Vec<Vec<Vec<f64>>>;

/// Shared inputs for a joint lock-in execution (keeps the engine entry
/// points within the argument-count lint).
pub struct JointRunInputs<'a> {
    pub lockin: &'a Lockin,
    pub gls: &'a JointHarmonicGlsConfig,
    pub t: TimeAxisRef<'a>,
    pub f_ref: f64,
    pub omega_tref: f64,
    pub sample_rate: f64,
    /// Effective solver tolerances, recorded verbatim in the estimator
    /// snapshot (no hidden policy).
    pub tolerances: JointSolverTolerances,
}

/// Joint lock-in outputs in boxcar-compatible layout.
///
/// The estimator always computes the full design-model covariance; the
/// none/diagonal/full serialization modes land with the covariance artifacts
/// in the next slice, so no information is dropped here.
pub struct JointRunOutput {
    /// Per channel [`XyColumns`] over the shared output grid.
    pub result: Vec<XyColumns>,
    /// Per-channel quality rows aligned with the output grid.
    pub quality: Vec<Vec<QualityRow>>,
    /// Per-channel, per-output 12x12 design-model covariance in XY order
    /// (peak-amplitude quantities). `covariance_output=none` deliberately returns
    /// empty per-channel vectors after validating each solver result, so
    /// native memory does not retain an unused 144-f64 matrix per output.
    pub covariance: Vec<XyCovariances>,
    /// Per-channel frozen estimator snapshots for staged reruns.
    pub snapshots: Vec<EstimatorSnapshot>,
    /// Per-channel retained calibration bindings (exact validated bytes).
    pub bindings: Vec<ModelBinding>,
    pub provenance: LockinProvenance,
    pub base_index_range: (usize, usize),
    pub output_index_range: (usize, usize),
}

/// Writes per-window quality diagnostics as a typed heterogeneous CSV
/// (AT-025). CSV only: no NPY sidecar is ever written, and existing files
/// are never overwritten (a partial write must not masquerade as complete).
/// Non-finite diagnostics abort the write instead of publishing bad rows.
pub fn write_quality_csv(path: &std::path::Path, rows: &[QualityRow]) -> Result<()> {
    if path.exists() {
        bail!("joint quality output already exists: {}", path.display());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!("failed to create quality output dir: {}", parent.display())
        })?;
    }
    // Validate every row before creating the file: a bailed write must not
    // leave a header-only partial artifact behind.
    for row in rows {
        for (name, value) in [
            ("time_s", row.time_s),
            ("scaled_design_condition", row.scaled_design_condition),
            ("residual_rms_v", row.residual_rms_v),
        ] {
            if !value.is_finite() {
                bail!(
                    "joint quality row at center {} has non-finite {name}: {value}",
                    row.original_center_index
                );
            }
        }
    }
    let file = std::fs::File::create(path)
        .with_context(|| format!("failed to create quality output: {}", path.display()))?;
    let mut writer = csv::WriterBuilder::new()
        .has_headers(true)
        .from_writer(file);
    writer
        .write_record([
            "original_center_index",
            "time_s",
            "scaled_design_condition",
            "residual_rms_v",
            "rank",
            "status",
        ])
        .context("failed to write quality header")?;
    for row in rows {
        writer
            .write_record([
                row.original_center_index.to_string(),
                row.time_s.to_string(),
                row.scaled_design_condition.to_string(),
                row.residual_rms_v.to_string(),
                row.rank.to_string(),
                row.status.as_str().to_string(),
            ])
            .with_context(|| {
                format!(
                    "failed to write quality row at center {}",
                    row.original_center_index
                )
            })?;
    }
    writer.flush().context("failed to flush quality output")?;
    Ok(())
}

/// Covariance CSV column header for a serialization mode (INTERFACES
/// section 5, FR-041): `time_s` plus 12 diagonal `cov_<q>_<q>_v2` entries in
/// XY order, or 78 upper-triangle entries in lexicographic (row,column)
/// order with row<=column. `none` is rejected: it omits the artifact
/// entirely instead of writing an empty file.
pub fn covariance_csv_header(mode: GlsCovarianceOutput) -> Result<Vec<String>> {
    let mut header = vec!["time_s".to_string()];
    match mode {
        GlsCovarianceOutput::None => {
            bail!(
                "covariance_output=none omits the covariance artifact; do not write an empty file"
            )
        }
        GlsCovarianceOutput::Diagonal => {
            for name in QUADRATURE_NAMES {
                header.push(format!("cov_{name}_{name}_v2"));
            }
        }
        GlsCovarianceOutput::Full => {
            for (row, row_name) in QUADRATURE_NAMES.iter().enumerate() {
                for column_name in &QUADRATURE_NAMES[row..] {
                    header.push(format!("cov_{row_name}_{column_name}_v2"));
                }
            }
        }
    }
    Ok(header)
}

/// Packs one 12x12 XY covariance into a serialized row matching
/// [`covariance_csv_header`]. Values arrive in peak-amplitude XY units
/// from the core mapper; this function only selects and orders entries.
pub fn pack_covariance_row(
    covariance_xy: &[Vec<f64>],
    mode: GlsCovarianceOutput,
) -> Result<Vec<f64>> {
    if covariance_xy.len() != 12 || covariance_xy.iter().any(|row| row.len() != 12) {
        bail!("covariance packing needs a 12x12 XY matrix");
    }
    match mode {
        GlsCovarianceOutput::None => {
            bail!("covariance_output=none omits the covariance artifact; do not pack rows")
        }
        GlsCovarianceOutput::Diagonal => {
            Ok((0..12).map(|index| covariance_xy[index][index]).collect())
        }
        GlsCovarianceOutput::Full => {
            let mut packed = Vec::with_capacity(78);
            for (row, values) in covariance_xy.iter().enumerate() {
                packed.extend_from_slice(&values[row..]);
            }
            Ok(packed)
        }
    }
}

/// Writes the per-window conditional covariance artifact
/// (`lockin/ch{channel}_covariance.csv`, AT-025): `time_s` plus the selected
/// covariance columns. Existing files are never overwritten, and non-finite
/// entries abort instead of publishing unknown-as-zero (FR-041).
pub fn write_covariance_csv(
    path: &std::path::Path,
    mode: GlsCovarianceOutput,
    times: &[f64],
    covariances: &[Vec<Vec<f64>>],
) -> Result<()> {
    let header = covariance_csv_header(mode)?;
    write_covariance_table(path, &header, times, covariances, mode, false)
}

/// NPY mirror of [`write_covariance_csv`] with exactly the same
/// time-plus-covariance columns, C-order `<f8`, via the shared
/// [`crate::utils::csv::write_npy`] conventions. Quality CSVs stay
/// CSV-only; covariance is the numeric artifact with an NPY mirror.
pub fn write_covariance_npy(
    path: &std::path::Path,
    mode: GlsCovarianceOutput,
    times: &[f64],
    covariances: &[Vec<Vec<f64>>],
) -> Result<()> {
    let header = covariance_csv_header(mode)?;
    write_covariance_table(path, &header, times, covariances, mode, true)
}

fn write_covariance_table(
    path: &std::path::Path,
    header: &[String],
    times: &[f64],
    covariances: &[Vec<Vec<f64>>],
    mode: GlsCovarianceOutput,
    npy: bool,
) -> Result<()> {
    if times.len() != covariances.len() {
        bail!(
            "covariance times ({}) and matrices ({}) differ",
            times.len(),
            covariances.len()
        );
    }
    if path.exists() {
        bail!("joint covariance output already exists: {}", path.display());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create covariance output dir: {}",
                parent.display()
            )
        })?;
    }
    let mut columns: Vec<Vec<f64>> = vec![Vec::with_capacity(times.len()); header.len()];
    for (index, (time, covariance)) in times.iter().zip(covariances.iter()).enumerate() {
        if !time.is_finite() {
            bail!("joint covariance row {index} has non-finite time_s");
        }
        let packed = pack_covariance_row(covariance, mode)?;
        if packed.iter().any(|value| !value.is_finite()) {
            bail!("joint covariance row {index} has non-finite entries");
        }
        columns[0].push(*time);
        for (column, value) in columns.iter_mut().skip(1).zip(packed.iter()) {
            column.push(*value);
        }
    }
    if npy {
        crate::utils::csv::write_npy(path, &columns)
            .with_context(|| format!("failed to write covariance NPY: {}", path.display()))?;
    } else {
        let file = std::fs::File::create(path)
            .with_context(|| format!("failed to create covariance output: {}", path.display()))?;
        let mut writer = csv::WriterBuilder::new()
            .has_headers(true)
            .from_writer(file);
        writer
            .write_record(header)
            .context("failed to write covariance header")?;
        for index in 0..times.len() {
            let record: Vec<String> = columns
                .iter()
                .map(|column| column[index].to_string())
                .collect();
            writer
                .write_record(&record)
                .with_context(|| format!("failed to write covariance row {index}"))?;
        }
        writer
            .flush()
            .context("failed to flush covariance output")?;
    }
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

/// Immutable native execution plan. Geometry and resource bounds are
/// prepared once per run and shared by every signal channel; per-window
/// scratch remains bounded by `chunk_size`.
#[derive(Debug, Clone, Copy)]
struct PreparedJointPlan {
    params: LockinParams,
    workers: usize,
    chunk_size: usize,
}

impl PreparedJointPlan {
    fn prepare(params: LockinParams, lockin: &Lockin) -> Result<Self> {
        validate_joint_output_range(params, lockin)?;
        let output_count = params
            .i_end
            .checked_sub(params.i_start)
            .and_then(|span| span.checked_add(1))
            .context("joint lock-in output count overflow")?;
        if output_count == 0 {
            bail!("joint lock-in prepared plan has no output windows");
        }
        Ok(Self {
            params,
            workers: lockin.workers.max(1),
            chunk_size: JOINT_OUTPUT_CHUNK,
        })
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
        tolerances,
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
    let plan = PreparedJointPlan::prepare(params, lockin)?;
    let model = HarmonicSignalModel {
        fit_harmonics: gls.fit_harmonics.clone(),
        output_harmonics: gls.output_harmonics.clone(),
        envelope_degree: gls.envelope_degree,
    };

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(plan.workers)
        .build()
        .context("failed to build joint GLS worker pool")?;

    let mut result = Vec::with_capacity(signal_data.len());
    let mut quality = Vec::with_capacity(signal_data.len());
    let mut covariance = Vec::with_capacity(signal_data.len());
    let mut snapshots = Vec::with_capacity(signal_data.len());
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
            tolerances,
        };
        let (columns, rows, covariances) =
            run_joint_channel(inputs, signal, channel, &plan, &settings, &pool)?;
        result.push(columns);
        quality.push(rows);
        covariance.push(
            if matches!(gls.covariance_output, GlsCovarianceOutput::None) {
                Vec::new()
            } else {
                covariances
            },
        );
        snapshots.push(
            build_estimator_snapshot(
                channel,
                gls,
                &settings.noise,
                &tolerances,
                JOINT_SOLVER_ID,
                f_ref,
                inputs.omega_tref,
                sample_rate_hz.recip(),
                &plan.params,
                bindings.last().expect("binding just pushed"),
                quality.last().expect("quality just pushed"),
            )
            .with_context(|| format!("joint estimator snapshot failed for channel {channel}"))?,
        );
    }

    let provenance =
        LockinProvenance::from_joint(plan.params, gls.noise_mode, &bindings, JOINT_SOLVER_ID);
    Ok(JointRunOutput {
        result,
        quality,
        covariance,
        snapshots,
        bindings,
        provenance,
        base_index_range: (plan.params.i_start, plan.params.i_end),
        output_index_range: (plan.params.i_start, plan.params.i_end),
    })
}

fn run_joint_channel(
    inputs: &JointRunInputs<'_>,
    signal: &[f64],
    channel: u8,
    plan: &PreparedJointPlan,
    settings: &JointHarmonicSettings,
    pool: &rayon::ThreadPool,
) -> Result<(XyColumns, Vec<QualityRow>, XyCovariances)> {
    let params = plan.params;
    let JointRunInputs {
        gls,
        t,
        f_ref,
        omega_tref,
        sample_rate: sample_rate_hz,
        ..
    } = *inputs;
    let outputs = params.i_end - params.i_start + 1;
    let mut columns: XyColumns = (0..12).map(|_| Vec::with_capacity(outputs)).collect();
    let mut rows = Vec::with_capacity(outputs);
    let mut covariances = Vec::with_capacity(outputs);
    // Same tap support as the legacy boxcar (edge legacy_trim): centers on
    // the strided grid, samples [center - n_half - 1, center + n_half + 1].
    let half_taps = params.n_half + 1;
    for chunk_start in (0..outputs).step_by(plan.chunk_size) {
        let chunk_end = (chunk_start + plan.chunk_size).min(outputs);
        let estimates: Vec<(usize, Result<JointEstimate>)> = pool.install(|| {
            (chunk_start..chunk_end)
                .into_par_iter()
                .map(|output_index| {
                    let center_k = params.i_start + output_index;
                    let center = center_k.checked_mul(params.stride).ok_or_else(|| {
                        anyhow::anyhow!("joint lock-in center index overflow for output {output_index}")
                    });
                    let result = center.and_then(|center| {
                        let lo = center.checked_sub(half_taps).ok_or_else(|| {
                            anyhow::anyhow!(
                                "joint lock-in window underflows for output {output_index}"
                            )
                        })?;
                        let hi = center.checked_add(half_taps).ok_or_else(|| {
                            anyhow::anyhow!(
                                "joint lock-in window overflows for output {output_index}"
                            )
                        })?;
                        if hi >= t.len() {
                            bail!(
                                "joint lock-in window [{lo}..={hi}] exceeds time length {}",
                                t.len()
                            );
                        }
                        let window_times: Vec<f64> =
                            (lo..=hi).map(|index| t.value_at(index)).collect();
                        let window_signal = signal.get(lo..=hi).ok_or_else(|| {
                            anyhow::anyhow!(
                                "joint lock-in window [{lo}..={hi}] exceeds signal length {}",
                                signal.len()
                            )
                        })?;
                        estimate_joint(
                            &window_times,
                            window_signal,
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
                        })
                    });
                    (output_index, result)
                })
                .collect()
        });

        // Rayon returns the chunk in index order, but sort explicitly so the
        // output contract stays deterministic if the execution backend changes.
        let mut estimates = estimates;
        estimates.sort_by_key(|(output_index, _)| *output_index);
        for (output_index, estimate) in estimates {
            let estimate = estimate?;
            if estimate.xy.len() != 12 {
                bail!(
                    "joint estimator returned {} quadratures, expected 12 (channel={channel}, window={output_index})",
                    estimate.xy.len()
                );
            }
            require_xy_covariance(&estimate.covariance_xy, channel, output_index)?;
            for (harmonic, column_pair) in columns.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                column_pair[0].push(estimate.xy[2 * harmonic]);
                column_pair[1].push(estimate.xy[2 * harmonic + 1]);
            }
            let center = (params.i_start + output_index)
                .checked_mul(params.stride)
                .context("joint lock-in center index overflow after estimation")?;
            rows.push(QualityRow {
                // Original-sample center index, consistent with time_s above:
                // the decimated output counter (center_k) is a grid namespace,
                // not an original-sample index. Pre-R0 artifacts recorded
                // center_k here; readers must interpret those under the old
                // convention (see the quality CSV contract docs).
                original_center_index: center,
                time_s: t.value_at(center),
                scaled_design_condition: estimate.condition,
                residual_rms_v: estimate.residual_rms,
                rank: estimate.rank,
                status: WindowStatus::for_jitter(estimate.jitter_applied_v2),
                jitter_applied_v2: estimate.jitter_applied_v2,
                noise_mode: gls_noise_mode_name(gls.noise_mode),
            });
            covariances.push(estimate.covariance_xy);
        }
    }
    Ok((columns, rows, covariances))
}

/// Rejects malformed XY covariance before it can reach an artifact: exactly
/// 12x12, finite everywhere (unknown uncertainty is never silently zero,
/// FR-041).
fn require_xy_covariance(
    covariance_xy: &[Vec<f64>],
    channel: u8,
    output_index: usize,
) -> Result<()> {
    if covariance_xy.len() != 12 || covariance_xy.iter().any(|row| row.len() != 12) {
        bail!(
            "joint estimator returned non-12x12 XY covariance (channel={channel}, window={output_index})"
        );
    }
    for (row, values) in covariance_xy.iter().enumerate() {
        for (column, value) in values.iter().enumerate() {
            if !value.is_finite() {
                bail!(
                    "joint estimator returned non-finite XY covariance at ({row},{column}) (channel={channel}, window={output_index})"
                );
            }
        }
    }
    Ok(())
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
