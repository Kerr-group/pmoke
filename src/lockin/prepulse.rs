//! Pre-pulse-derived calibration for the joint-harmonic GLS estimator
//! (Issue #258, REQUIREMENTS v0.4 FR-01..09).
//!
//! When `lockin.estimator.calibration_source` is `prepulse`, the joint-GLS
//! entry path ([`crate::lockin::li_process`]) derives the per-channel noise
//! model once per LI run from `pulse.background_before` instead of loading
//! file artifacts, then runs the LI unchanged. No artifact files are
//! created; derivation provenance is recorded as `prepulse_calibration_digest`
//! on the LI provenance record.
//!
//! The derivation reuses the recorded-only calibration pipeline
//! (`pmoke-analysis-core::calibration`, the same kernels behind
//! `pmoke calibrate`) with a fixed short-interval policy documented below,
//! and reduces the built in-memory artifact through the same
//! artifact-to-model gate as the file loader
//! ([`crate::lockin::model_loading::noise_model_from_artifact`]).
//! Derivation failure (empty interval, too few training blocks, SCS adequacy
//! mismatch, non-finite input) aborts the run with a named error, never a
//! silent fallback (FR-06).

use crate::config::{GlsNoiseMode, JointHarmonicGlsConfig, Window};
use crate::lockin::joint::{ModelBinding, NoiseModelSource, gls_noise_mode_name};
use crate::lockin::model_loading::noise_model_from_artifact;
use crate::utils::time_axis::TimeAxisRef;
use anyhow::{Context, Result, bail};
use pmoke_analysis_core::calibration::{
    AcquisitionMeta, AdequacyGroup, AdequacyPolicy, ArtifactRequest, BlockPlanRequest,
    CALIBRATION_PHASE_CONVENTION, CalibrationRole, CorrelationRecipe, HeldoutReport,
    ModelBinding as CoreBinding, PhaseVarianceRecipe, RoleInterval, TuningMode, assemble_samples,
    build_artifact, estimate_correlation, estimate_phase_variance, fit_nuisance, plan_blocks,
    scs_adequacy,
};
use pmoke_analysis_core::joint::{JointSolverTolerances, NoiseModel};
use std::collections::{BTreeMap, HashMap, HashSet};

/// Fixed seed recorded on derived artifacts. The builders are deterministic
/// and consume no RNG; the seed is recorded so the provenance record is
/// explicit (FR-04: same input yields the same digest).
pub const PREPULSE_SEED: u64 = 0;

/// Target training-block count over the pre-pulse interval. The block length
/// is `interval_samples / PREPULSE_TARGET_BLOCKS` (floored at
/// [`PREPULSE_MIN_BLOCK_LEN`]); a short interval therefore yields fewer
/// blocks and trips the [`PREPULSE_MIN_TRAINING_BLOCKS`] gate instead of
/// silently fitting one giant block.
pub const PREPULSE_TARGET_BLOCKS: usize = 8;
/// Minimum usable training blocks (FR-06 short-interval gate, applied
/// correspondingly to the single pre-pulse interval).
pub const PREPULSE_MIN_TRAINING_BLOCKS: usize = 2;
/// Minimum block length in samples. The nuisance regression spans 26
/// parameters; blocks below this floor cannot support it.
pub const PREPULSE_MIN_BLOCK_LEN: usize = 64;
/// Minimum reference cycles per block. Pre-pulse blocks are short by
/// construction, so this is a separability floor (aligned with the
/// per-bin cycle minimum), not the archival 128-cycle bar: the quality
/// backstop is the training-block count plus the SCS adequacy split-half
/// check (FR-06), and the artifact record itself states the nuisance fit is
/// a noise-estimation input rather than estimator validation.
pub const PREPULSE_MIN_CYCLES_PER_BLOCK: f64 = 2.0;
/// Phase-variance table bins (the `pmoke calibrate` CLI scale for modest
/// recorded channels).
pub const PREPULSE_BINS: usize = 8;
/// Minimum samples per phase bin (qualification-proven short-block scale).
pub const PREPULSE_MIN_SAMPLES_PER_BIN: usize = 8;
/// Minimum distinct reference cycles per phase bin.
pub const PREPULSE_MIN_CYCLES_PER_BIN: usize = 2;
/// Correlation shrinkage (the library default, shared with `calibrate`).
pub const PREPULSE_CORRELATION_ETA: f64 = 0.01;
/// Upper cap for the derived correlation support (the library default).
pub const PREPULSE_MAX_LAG: usize = 256;

/// Sentinel acquisition component of the derivation digest when no
/// acquisition manifest file exists (synthetic unit fixtures). Real runs
/// always hash the manifest; the sentinel keeps the digest total without
/// inventing provenance.
pub const PREPULSE_ABSENT_ACQUISITION: &str = "absent";

/// Digest domain separator. The exact field layout is fixed here (FR-04).
const PREPULSE_DIGEST_DOMAIN: &str = "pmoke-prepulse-calibration/v1";

/// In-memory noise models derived once per LI run, served per channel.
/// Created by [`derive_prepulse`]; never reads files.
#[derive(Debug, Clone)]
pub struct PrepulseNoiseModelSource {
    models: HashMap<u8, (NoiseModel, ModelBinding)>,
}

impl NoiseModelSource for PrepulseNoiseModelSource {
    fn load(&self, channel: u8, noise_mode: GlsNoiseMode) -> Result<(NoiseModel, ModelBinding)> {
        let (model, binding) = self
            .models
            .get(&channel)
            .ok_or_else(|| anyhow::anyhow!("no pre-pulse derived model for channel {channel}"))?;
        if binding.noise_mode != noise_mode {
            bail!(
                "pre-pulse derived model for channel {channel} was built for mode {}, not {}",
                gls_noise_mode_name(binding.noise_mode),
                gls_noise_mode_name(noise_mode),
            );
        }
        Ok((model.clone(), binding.clone()))
    }
}

/// Derivation output: the per-channel source plus the run-level FR-04
/// digest to record in LI provenance.
pub struct PrepulseDerivation {
    pub source: PrepulseNoiseModelSource,
    pub digest: String,
}

/// Acquisition digest for the FR-04 digest input: SHA-256 of the acquisition
/// manifest bytes (the same file `write_analysis_metadata` records as
/// `source_acquisition_sha256`), or [`PREPULSE_ABSENT_ACQUISITION`] when the
/// manifest is absent.
pub fn acquisition_digest_for(cfg: &crate::config::Config) -> String {
    let manifest = cfg.resolver().acquisition_manifest();
    if manifest.is_file()
        && let Ok(digest) = crate::utils::checksum::file_sha256(&manifest)
    {
        return digest;
    }
    PREPULSE_ABSENT_ACQUISITION.to_string()
}

/// FR-04 derivation digest: SHA-256 lowercase hex over the interval spec,
/// the interval sample count, the sample interval, the acquisition digest,
/// and the estimator parameters. The layout is fixed by construction;
/// identical inputs yield identical digests (fixed seed, no RNG).
pub fn prepulse_calibration_digest(
    window_start: f64,
    window_end: f64,
    sample_count: usize,
    sample_interval_s: f64,
    acquisition_digest: &str,
    gls: &JointHarmonicGlsConfig,
) -> String {
    let harmonics = gls
        .fit_harmonics
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let canonical = format!(
        "{PREPULSE_DIGEST_DOMAIN}\ninterval_start_bits={}\ninterval_end_bits={}\nsamples={sample_count}\nsample_interval_s_bits={}\nacquisition={acquisition_digest}\nfit_harmonics={harmonics}\nnoise_mode={}\nenvelope_degree={}\n",
        window_start.to_bits(),
        window_end.to_bits(),
        sample_interval_s.to_bits(),
        gls_noise_mode_name(gls.noise_mode),
        gls.envelope_degree,
    );
    crate::utils::checksum::sha256_hex(canonical.as_bytes())
}

/// Derives the per-channel noise model once per LI run from the pre-pulse
/// interval (FR-02/03/07): slice `pulse.background_before` from the full
/// timebase, run the calibrate-equivalent estimation per channel, gate on
/// training-block count plus SCS adequacy (FR-06), and reduce each built
/// in-memory artifact through the shared artifact-to-model gate.
///
/// `reference_phase_rad` is the LI reference phase (`omega_tref`), so the
/// nuisance regression uses the same phase convention as the downstream
/// estimator. Derivation runs for every `noise_mode` (FR-07); the
/// correlation table is estimated only when the mode needs it
/// (`stationary_correlated` / `phase_correlated`), mirroring the per-mode
/// reduction of the file loader.
#[allow(clippy::too_many_arguments)]
pub fn derive_prepulse(
    window: Window,
    t: TimeAxisRef<'_>,
    signal_ch: &[u8],
    signal_data: &[&[f64]],
    f_ref: f64,
    reference_phase_rad: f64,
    sample_interval_s: f64,
    gls: &JointHarmonicGlsConfig,
    acquisition_digest: &str,
) -> Result<PrepulseDerivation> {
    if !window.start.is_finite() || !window.end.is_finite() {
        bail!(
            "pre-pulse calibration needs a finite background_before window (got start={}, end={})",
            window.start,
            window.end
        );
    }
    if window.start >= window.end {
        bail!(
            "pre-pulse calibration needs a non-empty background_before window (got start={}, end={})",
            window.start,
            window.end
        );
    }
    if !(sample_interval_s.is_finite() && sample_interval_s > 0.0) {
        bail!("pre-pulse calibration needs a positive finite sample interval");
    }
    if !(f_ref.is_finite() && f_ref > 0.0) {
        bail!("pre-pulse calibration needs a positive finite reference frequency");
    }
    if !reference_phase_rad.is_finite() {
        bail!("pre-pulse calibration needs a finite reference phase");
    }
    if signal_ch.len() != signal_data.len() {
        bail!(
            "pre-pulse calibration channel count ({}) and signal data column count ({}) differ",
            signal_ch.len(),
            signal_data.len()
        );
    }
    if signal_data.is_empty() {
        bail!("pre-pulse calibration has no signal channels to derive from");
    }
    // Slice the interval with the same inclusive time convention as the
    // sensor background averages. The fetched timebase is monotonic, so the
    // selected samples form one contiguous half-open range.
    let mut selected: Option<(usize, usize)> = None;
    for (index, ti) in t.iter().enumerate() {
        if !ti.is_finite() {
            bail!("pre-pulse calibration needs a finite timebase (got non-finite t[{index}])");
        }
        if ti >= window.start && ti <= window.end {
            match selected.as_mut() {
                Some((_, last)) => *last = index + 1,
                None => selected = Some((index, index + 1)),
            }
        }
    }
    let (lo, hi) = selected.ok_or_else(|| {
        anyhow::anyhow!(
            "pre-pulse calibration interval background_before=[{}, {}] holds no samples (empty interval)",
            window.start, window.end
        )
    })?;
    for (channel, signal) in signal_ch.iter().zip(signal_data.iter()) {
        if signal.len() < hi {
            bail!(
                "pre-pulse calibration signal for channel {channel} holds {} samples, below the interval end {hi}",
                signal.len()
            );
        }
    }
    let samples = hi - lo;
    if samples < PREPULSE_MIN_TRAINING_BLOCKS * PREPULSE_MIN_BLOCK_LEN {
        bail!(
            "pre-pulse calibration interval holds only {samples} samples, below the short-interval floor of {} (insufficient_calibration)",
            PREPULSE_MIN_TRAINING_BLOCKS * PREPULSE_MIN_BLOCK_LEN
        );
    }
    let block_len = (samples / PREPULSE_TARGET_BLOCKS).max(PREPULSE_MIN_BLOCK_LEN);
    let plan_request = BlockPlanRequest {
        intervals: vec![RoleInterval {
            role: CalibrationRole::Training,
            start: lo as u64,
            end: hi as u64,
        }],
        block_len,
        total_samples: t.len() as u64,
        reference_frequency_hz: f_ref,
        sample_interval_s,
        min_reference_cycles_per_block: PREPULSE_MIN_CYCLES_PER_BLOCK,
        min_training_blocks: PREPULSE_MIN_TRAINING_BLOCKS,
        // A single pre-pulse interval nominates training blocks; requiring
        // two contributing intervals (the archival default) would make the
        // source unreachable by construction.
        min_training_intervals: 1,
    };
    let plan = plan_blocks(&plan_request)
        .context("pre-pulse calibration block planning failed (insufficient_calibration)")?;
    let training: Vec<(usize, usize)> = plan
        .blocks
        .iter()
        .filter(|block| block.role == CalibrationRole::Training)
        .map(|block| (block.start as usize, block.end as usize))
        .collect();
    if training.len() < PREPULSE_MIN_TRAINING_BLOCKS {
        bail!(
            "pre-pulse calibration holds only {} training blocks below the minimum {} (insufficient_calibration)",
            training.len(),
            PREPULSE_MIN_TRAINING_BLOCKS
        );
    }

    let sample_rate_hz = sample_interval_s.recip();
    let tolerances = JointSolverTolerances::default();
    let needs_correlation = matches!(
        gls.noise_mode,
        GlsNoiseMode::StationaryCorrelated | GlsNoiseMode::PhaseCorrelated
    );
    let mut models = HashMap::with_capacity(signal_data.len());
    let mut warned: HashSet<String> = HashSet::new();
    for (channel, signal) in signal_ch.iter().zip(signal_data.iter()) {
        let (model, binding) = derive_channel(
            *channel,
            t,
            signal,
            &training,
            f_ref,
            reference_phase_rad,
            sample_rate_hz,
            sample_interval_s,
            gls,
            needs_correlation,
            &plan,
            acquisition_digest,
            tolerances,
            &mut warned,
        )
        .with_context(|| {
            format!("pre-pulse calibration derivation failed for channel {channel}")
        })?;
        models.insert(*channel, (model, binding));
    }
    let digest = prepulse_calibration_digest(
        window.start,
        window.end,
        samples,
        sample_interval_s,
        acquisition_digest,
        gls,
    );
    Ok(PrepulseDerivation {
        source: PrepulseNoiseModelSource { models },
        digest,
    })
}

#[allow(clippy::too_many_arguments)]
fn derive_channel(
    channel: u8,
    t: TimeAxisRef<'_>,
    signal: &[f64],
    training: &[(usize, usize)],
    f_ref: f64,
    reference_phase_rad: f64,
    sample_rate_hz: f64,
    sample_interval_s: f64,
    gls: &JointHarmonicGlsConfig,
    needs_correlation: bool,
    plan: &pmoke_analysis_core::calibration::BlockPlan,
    acquisition_digest: &str,
    tolerances: JointSolverTolerances,
    warned: &mut HashSet<String>,
) -> Result<(NoiseModel, ModelBinding)> {
    let mut block_times = Vec::with_capacity(training.len());
    let mut block_residuals = Vec::with_capacity(training.len());
    for (start, end) in training {
        // Training blocks carry absolute trace indices into the full
        // timebase and signal series.
        let times: Vec<f64> = (*start..*end).map(|index| t.value_at(index)).collect();
        let fit = fit_nuisance(
            &times,
            &signal[*start..*end],
            f_ref,
            reference_phase_rad,
            sample_rate_hz,
            tolerances,
        )
        .with_context(|| {
            format!(
                "pre-pulse nuisance fit failed for channel {channel} (non_finite_input or rank)"
            )
        })?;
        block_times.push(times);
        block_residuals.push(fit.residual);
    }
    let recipe = PhaseVarianceRecipe {
        bins: PREPULSE_BINS,
        min_samples_per_bin: PREPULSE_MIN_SAMPLES_PER_BIN,
        min_cycles_per_bin: PREPULSE_MIN_CYCLES_PER_BIN,
        min_contributing_blocks: training.len(),
        shrinkage_alpha: pmoke_analysis_core::calibration::DEFAULT_SHRINKAGE_ALPHA,
        floor_ratio: pmoke_analysis_core::calibration::DEFAULT_FLOOR_RATIO,
    };
    let samples = assemble_samples(&block_times, &block_residuals, f_ref, reference_phase_rad)
        .context("pre-pulse sample assembly failed")?;
    let variance = estimate_phase_variance(&samples, recipe)
        .context("pre-pulse variance estimation failed")?;

    let (correlation, correlation_recipe) = if needs_correlation {
        let shortest = block_residuals.iter().map(Vec::len).min().unwrap_or(0);
        // Scale the lag support to the short blocks; the estimator below
        // still requires support strictly inside the shortest block.
        let max_lag = (shortest / PREPULSE_TARGET_BLOCKS).clamp(1, PREPULSE_MAX_LAG);
        let recipe = CorrelationRecipe {
            max_lag,
            shrinkage_eta: PREPULSE_CORRELATION_ETA,
        };
        let scale = variance.v0.sqrt();
        let standardized: Vec<Vec<f64>> = block_residuals
            .iter()
            .map(|block| {
                let mean = block.iter().sum::<f64>() / block.len() as f64;
                block.iter().map(|value| (value - mean) / scale).collect()
            })
            .collect();
        let output = estimate_correlation(&standardized, None, sample_interval_s, recipe)
            .context("pre-pulse correlation estimation failed")?;
        (Some(output), Some(recipe))
    } else {
        (None, None)
    };

    // SCS adequacy split-half gate (FR-06, calibrate-corresponding): the
    // training groups must agree with their held-out half; fewer than two
    // blocks are insufficient evidence by construction.
    let adequacy = {
        let groups: Vec<AdequacyGroup> = block_times
            .iter()
            .zip(block_residuals.iter())
            .map(|(times, residuals)| {
                let mean = residuals.iter().sum::<f64>() / residuals.len() as f64;
                let scale = variance.v0.sqrt();
                let standardized: Vec<f64> = residuals
                    .iter()
                    .map(|value| (value - mean) / scale)
                    .collect();
                let phases: Vec<f64> = times
                    .iter()
                    .map(|time| {
                        (std::f64::consts::TAU * f_ref * time - reference_phase_rad)
                            .rem_euclid(std::f64::consts::TAU)
                    })
                    .collect();
                AdequacyGroup {
                    standardized,
                    phases,
                }
            })
            .collect();
        if groups.len() < PREPULSE_MIN_TRAINING_BLOCKS {
            bail!(
                "pre-pulse calibration adequacy needs at least {} training blocks (got {})",
                PREPULSE_MIN_TRAINING_BLOCKS,
                groups.len()
            );
        }
        let (head, tail) = groups.split_at(groups.len() / 2);
        scs_adequacy(
            head,
            tail,
            AdequacyPolicy {
                lags: 4,
                min_pairs_per_cell: 10,
                max_phase_spread: 0.2,
                max_reserved_shift: 0.2,
            },
        )
        .context("pre-pulse adequacy assessment failed")?
    };
    if !adequacy.adequate {
        bail!(
            "pre-pulse calibration residuals fail SCS adequacy for channel {channel} ({}); refusing to derive",
            adequacy.reason
        );
    }

    let built = build_artifact(ArtifactRequest {
        model_id: format!("pmoke-prepulse-ch{channel}"),
        pmoke_version: env!("CARGO_PKG_VERSION").to_string(),
        backend_versions: BTreeMap::new(),
        binding: CoreBinding {
            channel: u32::from(channel),
            voltage_unit: crate::lockin::model_loading::PIPELINE_VOLTAGE_UNIT.to_string(),
            adc_scale_provenance: None,
            sample_interval_s,
            recorded_original_dt_s: Some(sample_interval_s),
            sample_interval_rel_tol: pmoke_analysis_core::calibration::DEFAULT_DT_REL_TOL,
            reference_frequency_hz: f_ref,
            frequency_rel_tol: pmoke_analysis_core::calibration::DEFAULT_FREQ_REL_TOL,
            phase_convention: CALIBRATION_PHASE_CONVENTION.to_string(),
            acquisition: AcquisitionMeta {
                device: None,
                gain: None,
                bandwidth_hz: None,
            },
        },
        variance,
        correlation,
        recipe,
        correlation_recipe,
        plan: plan.clone(),
        source_digests: vec![acquisition_digest.to_string()],
        seed: PREPULSE_SEED,
        tuning: TuningMode::Fixed,
        heldout: HeldoutReport {
            blocks: 0,
            profile_rmse_v2: None,
            standardized_lag1: None,
        },
    })
    .with_context(|| format!("pre-pulse artifact construction failed for channel {channel}"))?;

    let (model, warnings) = noise_model_from_artifact(
        &built.artifact,
        channel,
        gls.noise_mode,
        sample_interval_s,
        f_ref,
        crate::lockin::model_loading::PIPELINE_VOLTAGE_UNIT,
    )?;
    for warning in warnings {
        if warned.insert(warning.clone()) {
            crate::ui::warn(format!("pre-pulse calibration: {warning}"));
        }
    }
    let binding = ModelBinding {
        channel,
        path: format!("prepulse:ch{channel}"),
        sha256: built.sha256_hex,
        noise_mode: gls.noise_mode,
        bytes: None,
    };
    Ok((model, binding))
}

#[cfg(test)]
#[path = "prepulse_tests.rs"]
mod tests;
