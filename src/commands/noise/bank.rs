//! Opt-in frozen condition-specific bank plus deterministic center schedule
//! (PN-M3, Issue #246; PN-FR-018..021/023/024/025/037).
//!
//! The bank lane is explicit and opt-in: regimes are nominated by an
//! external schedule or declared training context, each regime fits its own
//! mode-specific model from its own training intervals, and the complete
//! output-center -> model mapping is resolved and hashed *before* any window
//! of the evaluation signal is touched (PN-FR-019). No window ever selects a
//! model from its own target amplitude or residual, and there is no online
//! re-estimation, blending, crossfade or interpolation (PN-FR-020).
//!
//! A window uses one complete model over its whole unchanged support.
//! Windows whose support crosses a regime boundary are flagged
//! `cross_regime` and stay scientifically unqualified until validated; they
//! are never dropped or duplicated. Identical regime models (same estimated
//! content) are reduced to one distinct model before inference, which makes
//! the identical-model bank reproduce the frozen global path exactly
//! (PN-FR-021).
//!
//! Apply is deterministic across worker/chunk settings: work is chunked,
//! executed with a bounded per-worker scratch window, and re-ordered by
//! original center index before any output is written (PN-FR-037).

use anyhow::{Result, bail};
use pmoke_analysis_core::joint::{JointHarmonicSettings, JointSolverTolerances, NoiseModel};
use pmoke_analysis_core::{
    HarmonicSignalModel, PreparedNoisePlan, estimate_joint, estimate_joint_with_plan,
};
use rayon::prelude::*;
use serde::Serialize;
use std::collections::BTreeMap;
use std::time::Instant;

use super::compare::{CompareBankSelectionProvenance, CompareBankToml, CompareCovarianceOutput};

/// Maximum distinct bank models per mode (inherited per-channel cap,
/// PN-NFR-003 / PN-A-007).
pub(super) const MAX_BANK_REGIMES_PER_MODE: usize = 8;

/// Default output chunk: bounds per-worker scratch and queue depth.
pub(super) const DEFAULT_BANK_CHUNK_SIZE: usize = 64;

/// Maximum accepted chunk size (a single chunk never materializes more than
/// this many windows at once).
pub(super) const MAX_BANK_CHUNK_SIZE: usize = 4096;

/// One nominated regime: condition label plus its half-open training
/// intervals (original sample indices).
#[derive(Debug, Clone)]
pub(super) struct BankRegime {
    pub name: String,
    pub condition: String,
    pub channel: Option<u8>,
    pub intervals: Vec<(u64, u64)>,
}

/// One schedule span assigned to a regime (half-open original indices).
#[derive(Debug, Clone)]
pub(super) struct BankScheduleSpan {
    pub regime_index: usize,
    pub regime: String,
    pub start: u64,
    pub end: u64,
}

/// Validated bank request resolved against the frozen source geometry.
#[derive(Debug, Clone)]
pub(super) struct ResolvedBank {
    pub provenance: &'static str,
    pub regimes: Vec<BankRegime>,
    pub spans: Vec<BankScheduleSpan>,
    pub workers: usize,
    pub chunk_size: usize,
}

/// Resolved per-center schedule row: one model, one regime, one boundary
/// classification; every center appears exactly once (PN-FR-019).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(super) struct BankScheduleRow {
    pub center: u64,
    pub regime_index: usize,
    pub boundary: bool,
}

/// One distinct bank model (deduplicated estimation content).
#[derive(Debug, Clone)]
pub(super) struct BankModelSlot {
    pub slot: usize,
    pub mode: String,
    pub model_id: String,
    pub sha256: String,
    pub bytes: usize,
    pub artifact_path: String,
    pub estimation_sha256: String,
    pub noise: NoiseModel,
    /// Representative training provenance (regime that first produced it).
    pub regime: String,
}

/// Regime -> model slot link for one requested mode.
#[derive(Debug, Clone)]
pub(super) struct BankRegimeLink {
    pub regime_index: usize,
    pub mode: String,
    pub slot: usize,
    pub model_id: String,
    pub sha256: String,
    pub artifact_path: String,
}

/// Per-row retained record for one bank leg.
#[derive(Debug, Clone, Serialize)]
pub(super) struct BankLegRow {
    pub center: u64,
    /// Recorded time of the center sample (from the frozen source timebase),
    /// so replay needs no external source.
    pub time_s: f64,
    pub regime: String,
    pub regime_index: usize,
    pub boundary: bool,
    pub model_id: String,
    pub model_sha256: String,
    pub magnitude: f64,
    pub xy: Vec<f64>,
    pub condition: f64,
    pub rank: usize,
    /// Standardized per-unit-noise residual RMS (dimensionless; numerically
    /// equal to the raw volts RMS for Identity noise when the reference
    /// variance is 1 V^2, and dimensionless in that coincidence), same
    /// convention as the joint quality column. The `_v` suffix is frozen
    /// promise; contrast the diagnostics-path volts RMS of a raw nuisance
    /// residual, which must not be compared directly.
    pub residual_rms_v: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub covariance: Option<Vec<f64>>,
}

/// One applied bank leg with its measured resource/acceleration evidence.
#[derive(Debug, Clone)]
pub(super) struct BankLegOutput {
    pub rows: Vec<BankLegRow>,
    pub window_failures: usize,
    pub prepared_plans: usize,
    pub prepare_ms: f64,
    pub apply_ms: f64,
    pub direct_reference_rows: usize,
    pub direct_reference_ms: f64,
    pub max_abs_error_v: f64,
}

/// Validates the opt-in bank request against the frozen source geometry.
/// Unknown model references, overlaps, gaps, channel mismatches and
/// unsigned-overflow fallacies all fail here, before any inference.
pub(super) fn validate_bank_request(
    bank: &CompareBankToml,
    detector: u8,
    total_samples: u64,
) -> Result<ResolvedBank> {
    if bank.schema_version != 1 {
        bail!(
            "unsupported noise compare bank schema_version {} (expected 1)",
            bank.schema_version
        );
    }
    let provenance = match bank.selection_provenance {
        CompareBankSelectionProvenance::ExternalSchedule => "external_schedule",
        CompareBankSelectionProvenance::DeclaredTrainingContext => "declared_training_context",
    };
    if bank.regimes.is_empty() {
        bail!("noise compare bank lists no regimes");
    }
    if bank.regimes.len() > MAX_BANK_REGIMES_PER_MODE {
        bail!(
            "noise compare bank requests {} regimes above the per-mode cap {MAX_BANK_REGIMES_PER_MODE} (code=resource_limit_exceeded)",
            bank.regimes.len()
        );
    }
    if bank.schedule.is_empty() {
        bail!("noise compare bank lists no schedule spans");
    }
    let mut regimes = Vec::with_capacity(bank.regimes.len());
    let mut seen_names: Vec<String> = Vec::with_capacity(bank.regimes.len());
    for regime in &bank.regimes {
        validate_regime_name(&regime.name)?;
        if seen_names.contains(&regime.name) {
            bail!(
                "noise compare bank regime '{}' is declared twice (code=invalid_request)",
                regime.name
            );
        }
        seen_names.push(regime.name.clone());
        if regime.condition.trim().is_empty() {
            bail!(
                "noise compare bank regime '{}' needs a non-empty condition label",
                regime.name
            );
        }
        if regime.condition.len() > 64 {
            bail!(
                "noise compare bank regime '{}' condition label is longer than 64 characters",
                regime.name
            );
        }
        if let Some(channel) = regime.channel
            && channel != detector
        {
            bail!(
                "noise compare bank regime '{}' binds ch{channel} but the detector is ch{detector} (code=channel_mismatch)",
                regime.name
            );
        }
        if regime.intervals.is_empty() {
            bail!(
                "noise compare bank regime '{}' lists no training intervals (code=insufficient_calibration)",
                regime.name
            );
        }
        let mut intervals: Vec<(u64, u64)> = Vec::with_capacity(regime.intervals.len());
        for interval in &regime.intervals {
            if interval.end <= interval.start {
                bail!(
                    "noise compare bank regime '{}' interval [{}, {}) is empty or reversed",
                    regime.name,
                    interval.start,
                    interval.end
                );
            }
            if interval.end > total_samples {
                bail!(
                    "noise compare bank regime '{}' interval [{}, {}) exceeds recorded length {total_samples}",
                    regime.name,
                    interval.start,
                    interval.end
                );
            }
            intervals.push((interval.start, interval.end));
        }
        intervals.sort_unstable();
        for pair in intervals.windows(2) {
            if pair[1].0 < pair[0].1 {
                bail!(
                    "noise compare bank regime '{}' training intervals [{}, {}) and [{}, {}) overlap (code=invalid_request)",
                    regime.name,
                    pair[0].0,
                    pair[0].1,
                    pair[1].0,
                    pair[1].1
                );
            }
        }
        regimes.push(BankRegime {
            name: regime.name.clone(),
            condition: regime.condition.clone(),
            channel: regime.channel,
            intervals,
        });
    }
    let name_to_index: Vec<(String, usize)> = regimes
        .iter()
        .enumerate()
        .map(|(index, regime)| (regime.name.clone(), index))
        .collect();
    let mut spans: Vec<BankScheduleSpan> = Vec::with_capacity(bank.schedule.len());
    for span in &bank.schedule {
        if span.end <= span.start {
            bail!(
                "noise compare bank schedule span [{}, {}) is empty or reversed",
                span.start,
                span.end
            );
        }
        if span.end > total_samples {
            bail!(
                "noise compare bank schedule span [{}, {}) exceeds recorded length {total_samples}",
                span.start,
                span.end
            );
        }
        let regime_index = name_to_index
            .iter()
            .find(|(name, _)| name == &span.regime)
            .map(|(_, index)| *index)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "noise compare bank schedule references unknown regime '{}' (code=unknown_model_reference)",
                    span.regime
                )
            })?;
        spans.push(BankScheduleSpan {
            regime_index,
            regime: span.regime.clone(),
            start: span.start,
            end: span.end,
        });
    }
    spans.sort_by_key(|span| span.start);
    for pair in spans.windows(2) {
        if pair[1].start < pair[0].end {
            bail!(
                "noise compare bank schedule spans [{}, {}) and [{}, {}) overlap (code=schedule_overlap)",
                pair[0].start,
                pair[0].end,
                pair[1].start,
                pair[1].end
            );
        }
    }
    let workers = bank.workers.unwrap_or(1);
    if workers == 0 {
        bail!("noise compare bank workers must be at least 1");
    }
    let available = std::thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1);
    if workers > available * 4 {
        bail!(
            "noise compare bank requests {workers} workers above four times the available parallelism {available} (code=resource_limit_exceeded)"
        );
    }
    let chunk_size = bank.chunk_size.unwrap_or(DEFAULT_BANK_CHUNK_SIZE);
    if chunk_size == 0 || chunk_size > MAX_BANK_CHUNK_SIZE {
        bail!(
            "noise compare bank chunk_size must lie in 1..={MAX_BANK_CHUNK_SIZE} (got {chunk_size})"
        );
    }
    Ok(ResolvedBank {
        provenance,
        regimes,
        spans,
        workers,
        chunk_size,
    })
}

/// Portable regime names: short, lowercase, no path or alias characters.
fn validate_regime_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 32 {
        bail!("noise compare bank regime name must hold 1..=32 characters: '{name}'");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
    {
        bail!(
            "noise compare bank regime name must be lowercase [a-z0-9_-] only (portable, case-insensitive, no aliasing): '{name}'"
        );
    }
    if name == "global" {
        bail!("noise compare bank regime name 'global' is reserved for the frozen global lane");
    }
    Ok(())
}

/// Resolves the complete output-center -> model schedule before inference.
/// Every center must fall inside exactly one span; a center outside every
/// span is an explicit unsupported-regime error, never extrapolation.
/// Windows whose support crosses a span edge are flagged `boundary`
/// (cross-regime support) and stay in the output.
pub(super) fn resolve_bank_schedule(
    centers: &[u64],
    half_window: u64,
    spans: &[BankScheduleSpan],
) -> Result<Vec<BankScheduleRow>> {
    if centers.is_empty() {
        bail!("noise compare bank schedule holds no output centers");
    }
    for pair in centers.windows(2) {
        if pair[1] <= pair[0] {
            bail!("noise compare bank schedule centers must be strictly increasing");
        }
    }
    let mut rows = Vec::with_capacity(centers.len());
    for center in centers {
        let span = spans
            .iter()
            .find(|span| *center >= span.start && *center < span.end)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "output center {center} falls outside every bank schedule span (code=unsupported_regime); no model may be extrapolated"
                )
            })?;
        let lo = center.saturating_sub(half_window);
        let hi = center.saturating_add(half_window);
        let boundary = lo < span.start || hi >= span.end;
        rows.push(BankScheduleRow {
            center: *center,
            regime_index: span.regime_index,
            boundary,
        });
    }
    Ok(rows)
}

/// Estimated-content fingerprint used for identical-model reduction: two
/// regime models with the same inference content are one distinct model even
/// though their artifact bytes carry different model IDs and training
/// provenance (PN-FR-021: an identical-model bank reduces to the fixed
/// global path).
pub(super) fn estimation_fingerprint(
    basis: &str,
    v0: f64,
    variances: &[f64],
    correlation_lags: Option<&[f64]>,
    correlation_lag_step_s: Option<f64>,
    reference_frequency_hz: f64,
    sample_interval_s: f64,
) -> String {
    let mut canonical = format!(
        "bank-estimation/v1|basis={basis}|f={reference_frequency_hz:.17e}|dt={sample_interval_s:.17e}|v0={v0:.17e}"
    );
    for variance in variances {
        canonical.push_str(&format!("|v={variance:.17e}"));
    }
    if let Some(lags) = correlation_lags {
        canonical.push_str(&format!(
            "|lag_step={:.17e}",
            correlation_lag_step_s.unwrap_or(f64::NAN)
        ));
        for lag in lags {
            canonical.push_str(&format!("|c={lag:.17e}"));
        }
    }
    crate::utils::checksum::sha256_hex(canonical.as_bytes())
}

/// Applies one bank leg: every schedule row demodulates with exactly one
/// frozen model over its whole window. The prepared factor per distinct
/// model is shared across windows (PN-M3 acceleration), and rows are
/// re-ordered by center so worker/chunk settings cannot change values or
/// order. Up to four rows are re-computed through the direct per-window
/// path and bounded against the accelerated value (PN-NFR-002).
#[allow(clippy::too_many_arguments)]
pub(super) fn apply_bank_leg(
    label: &str,
    slots: &[BankModelSlot],
    regime_names: &[String],
    regime_to_slot: &[usize],
    schedule: &[BankScheduleRow],
    center_origin: u64,
    half_taps: usize,
    times: &[f64],
    signal: &[f64],
    reference_frequency_hz: f64,
    reference_phase_rad: f64,
    sample_interval_s: f64,
    fit_harmonics: &[usize],
    covariance_policy: CompareCovarianceOutput,
    workers: usize,
    chunk_size: usize,
    direct_reference_sample: usize,
) -> Result<BankLegOutput> {
    let full_window_len = half_taps
        .checked_mul(2)
        .and_then(|value| value.checked_add(3))
        .ok_or_else(|| anyhow::anyhow!("bank window length overflow"))?;
    let tolerances = JointSolverTolerances::default();
    let mut window_failures = 0_usize;
    // (schedule index, absolute center, local lo, local hi)
    let mut jobs: Vec<(usize, u64, usize, usize)> = Vec::with_capacity(schedule.len());
    // Window lengths needed by the resolved grid: the fixed full support, or
    // one sample shorter when a span start clips the first window (that
    // center keeps its row instead of being dropped).
    let mut needed_lengths: Vec<usize> = vec![full_window_len];
    for (index, row) in schedule.iter().enumerate() {
        // Schedule centers are in original-sample coordinates; the decoded
        // evaluation arrays start at `center_origin`.
        let local = row
            .center
            .checked_sub(center_origin)
            .ok_or_else(|| anyhow::anyhow!("bank schedule center precedes the decoded origin"))?;
        let center_usize = usize::try_from(local)
            .map_err(|_| anyhow::anyhow!("bank center index exceeds platform range"))?;
        let lo = center_usize.saturating_sub(half_taps + 1);
        let hi = center_usize + half_taps + 1;
        if hi >= signal.len() || hi >= times.len() {
            window_failures += 1;
            continue;
        }
        let length = hi - lo + 1;
        if !needed_lengths.contains(&length) {
            needed_lengths.push(length);
        }
        jobs.push((index, row.center, lo, hi));
    }
    if jobs.is_empty() {
        bail!("bank leg {label} produced no bounded windows");
    }
    let prepare_started = Instant::now();
    let mut plan_cache: BTreeMap<usize, Vec<PreparedNoisePlan>> = BTreeMap::new();
    let mut prepared_plans = 0_usize;
    for length in &needed_lengths {
        let mut plans: Vec<PreparedNoisePlan> = Vec::with_capacity(slots.len());
        for slot in slots {
            let plan =
                PreparedNoisePlan::prepare(&slot.noise, *length, tolerances).map_err(|error| {
                    anyhow::anyhow!(
                        "bank leg {label} cannot prepare model {} (code={}): {error}",
                        slot.model_id,
                        error.code()
                    )
                })?;
            plans.push(plan);
            prepared_plans += 1;
        }
        plan_cache.insert(*length, plans);
    }
    let prepare_ms = prepare_started.elapsed().as_secs_f64() * 1.0e3;
    let model = HarmonicSignalModel {
        fit_harmonics: fit_harmonics.to_vec(),
        output_harmonics: vec![1, 2, 3, 4, 5, 6],
        envelope_degree: 0,
    };
    let apply_started = Instant::now();
    let compute =
        |schedule_index: usize, center: u64, lo: usize, hi: usize| -> Result<BankLegRow> {
            let row = &schedule[schedule_index];
            let slot_index = regime_to_slot[row.regime_index];
            let slot = &slots[slot_index];
            let center_usize =
                usize::try_from(center.checked_sub(center_origin).ok_or_else(|| {
                    anyhow::anyhow!("bank schedule center precedes the decoded origin")
                })?)
                .map_err(|_| anyhow::anyhow!("bank center index exceeds platform range"))?;
            let length = hi - lo + 1;
            let plans = plan_cache.get(&length).ok_or_else(|| {
                anyhow::anyhow!("bank leg {label} has no prepared plan for window length {length}")
            })?;
            let estimate = estimate_joint_with_plan(
                &times[lo..=hi],
                &signal[lo..=hi],
                reference_frequency_hz,
                reference_phase_rad,
                1.0 / sample_interval_s,
                &model,
                tolerances,
                &plans[slot_index],
            )?;
            if estimate.xy.len() < 12 {
                bail!(
                    "bank leg {label} produced {} XY values, need 12 (code=dimension_mismatch)",
                    estimate.xy.len()
                );
            }
            let magnitude = estimate.xy[0].hypot(estimate.xy[1]);
            if !magnitude.is_finite() {
                bail!("bank leg {label} produced a non-finite magnitude");
            }
            let covariance = match covariance_policy {
                CompareCovarianceOutput::None => None,
                CompareCovarianceOutput::Diagonal => Some(
                    (0..12)
                        .map(|index| estimate.covariance_xy[index][index])
                        .collect(),
                ),
                CompareCovarianceOutput::Full => {
                    let mut packed = Vec::with_capacity(78);
                    for (row_index, values) in estimate.covariance_xy.iter().enumerate() {
                        packed.extend_from_slice(&values[row_index..]);
                    }
                    Some(packed)
                }
            };
            Ok(BankLegRow {
                center,
                time_s: times[center_usize],
                regime: regime_names.get(row.regime_index).cloned().ok_or_else(|| {
                    anyhow::anyhow!("bank leg {label} lost regime {}", row.regime_index)
                })?,
                regime_index: row.regime_index,
                boundary: row.boundary,
                model_id: slot.model_id.clone(),
                model_sha256: slot.sha256.clone(),
                magnitude,
                xy: estimate.xy.clone(),
                condition: estimate.condition,
                rank: estimate.rank,
                residual_rms_v: estimate.residual_rms,
                covariance,
            })
        };
    let mut rows: Vec<BankLegRow> = Vec::with_capacity(jobs.len());
    let workers = workers.max(1);
    if workers == 1 && chunk_size >= jobs.len() {
        for (index, center, lo, hi) in &jobs {
            match compute(*index, *center, *lo, *hi) {
                Ok(row) => rows.push(row),
                Err(_) => window_failures += 1,
            }
        }
    } else {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(workers)
            .build()
            .map_err(|error| anyhow::anyhow!("cannot build bank worker pool: {error}"))?;
        for chunk in jobs.chunks(chunk_size) {
            let mut computed: Vec<Result<BankLegRow>> = pool.install(|| {
                chunk
                    .par_iter()
                    .map(|(index, center, lo, hi)| compute(*index, *center, *lo, *hi))
                    .collect()
            });
            // Deterministic order regardless of worker scheduling.
            computed
                .sort_by_key(|result| result.as_ref().map(|row| row.center).unwrap_or(u64::MAX));
            for result in computed {
                match result {
                    Ok(row) => rows.push(row),
                    Err(_) => window_failures += 1,
                }
            }
        }
    }
    rows.sort_by_key(|row| row.center);
    if rows.is_empty() {
        bail!("bank leg {label} produced no finite demodulation rows (code=estimator_failure)");
    }
    let apply_ms = apply_started.elapsed().as_secs_f64() * 1.0e3;
    // Direct-path differential control on a bounded prefix of rows: the
    // accelerated values must satisfy abs(error) <= 1e-8 + 1e-8*abs(ref)
    // (PN-NFR-002). The regression is measured, not assumed.
    let mut direct_reference_rows = 0_usize;
    let mut direct_reference_ms = 0.0_f64;
    let mut max_abs_error_v = 0.0_f64;
    for row in rows.iter().take(direct_reference_sample.min(4)) {
        let local = row
            .center
            .checked_sub(center_origin)
            .ok_or_else(|| anyhow::anyhow!("bank schedule center precedes the decoded origin"))?;
        let center_usize = usize::try_from(local)
            .map_err(|_| anyhow::anyhow!("bank center index exceeds platform range"))?;
        let lo = center_usize.saturating_sub(half_taps + 1);
        let hi = center_usize + half_taps + 1;
        let slot_index = regime_to_slot[row.regime_index];
        let direct_started = Instant::now();
        let direct = estimate_joint(
            &times[lo..=hi],
            &signal[lo..=hi],
            reference_frequency_hz,
            reference_phase_rad,
            1.0 / sample_interval_s,
            &JointHarmonicSettings {
                model: model.clone(),
                noise: slots[slot_index].noise.clone(),
                tolerances,
            },
        )?;
        direct_reference_ms += direct_started.elapsed().as_secs_f64() * 1.0e3;
        let direct_magnitude = direct.xy[0].hypot(direct.xy[1]);
        for (got, want) in row.xy.iter().zip(direct.xy.iter()) {
            let error = (got - want).abs();
            let bound = 1.0e-8 + 1.0e-8 * want.abs();
            if error > bound {
                bail!(
                    "bank leg {label} accelerated XY exceeds the PN-NFR-002 bound at center {}: {error:.6e} > {bound:.6e}",
                    row.center
                );
            }
            max_abs_error_v = max_abs_error_v.max(error);
        }
        max_abs_error_v = max_abs_error_v.max((row.magnitude - direct_magnitude).abs());
        direct_reference_rows += 1;
    }
    Ok(BankLegOutput {
        rows,
        window_failures,
        prepared_plans,
        prepare_ms,
        apply_ms,
        direct_reference_rows,
        direct_reference_ms,
        max_abs_error_v,
    })
}

/// Boundary/switch diagnostics for one applied leg (PN-FR-020):
/// cross-regime windows, boundary windows, and the adjacent-sample jump RMS
/// at regime switches versus inside a regime (the no-change control).
#[derive(Debug, Clone, Serialize)]
pub(super) struct BoundaryStats {
    pub boundary_windows: usize,
    pub cross_regime_windows: usize,
    pub switch_jump_rms: Option<f64>,
    pub interior_jump_rms: Option<f64>,
    pub switch_transitions: usize,
}

pub(super) fn boundary_stats(rows: &[BankLegRow]) -> BoundaryStats {
    let mut boundary_windows = 0_usize;
    let mut cross_regime_windows = 0_usize;
    let mut switch_squares = 0.0_f64;
    let mut switch_count = 0_usize;
    let mut interior_squares = 0.0_f64;
    let mut interior_count = 0_usize;
    for (index, row) in rows.iter().enumerate() {
        if row.boundary {
            boundary_windows += 1;
            cross_regime_windows += 1;
        }
        if let Some(next) = rows.get(index + 1) {
            let delta = next.magnitude - row.magnitude;
            if next.regime_index != row.regime_index {
                switch_squares += delta * delta;
                switch_count += 1;
            } else {
                interior_squares += delta * delta;
                interior_count += 1;
            }
        }
    }
    BoundaryStats {
        boundary_windows,
        cross_regime_windows,
        switch_jump_rms: (switch_count > 0).then(|| (switch_squares / switch_count as f64).sqrt()),
        interior_jump_rms: (interior_count > 0)
            .then(|| (interior_squares / interior_count as f64).sqrt()),
        switch_transitions: switch_count,
    }
}

/// Best-effort peak RSS in KiB for the current process; `None` with an
/// explicit reason is reported when the platform does not expose it
/// (PN-NFR-003: report actual values and exclusions, never invent them).
pub(super) fn peak_rss_kib() -> Result<Option<u64>> {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status.lines() {
            if let Some(value) = line.strip_prefix("VmHWM:") {
                let kib = value
                    .trim()
                    .trim_end_matches(" kB")
                    .trim()
                    .parse::<u64>()
                    .map_err(|error| anyhow::anyhow!("cannot parse VmHWM: {error}"))?;
                return Ok(Some(kib));
            }
        }
        return Ok(None);
    }
    if cfg!(unix) {
        let output = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p"])
            .arg(std::process::id().to_string())
            .output();
        if let Ok(output) = output
            && output.status.success()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            if let Ok(kib) = text.trim().parse::<u64>() {
                return Ok(Some(kib));
            }
        }
    }
    Ok(None)
}

/// Schedule hash: canonical digest over the resolved per-center mapping and
/// the regime spans (PN-FR-019/023). The mapping is hashed after resolution
/// and never read from target windows.
pub(super) fn schedule_sha256(spans: &[BankScheduleSpan], rows: &[BankScheduleRow]) -> String {
    let mut canonical = String::from("bank-schedule/v1");
    for span in spans {
        canonical.push_str(&format!("|s:{}:{}:{}", span.regime, span.start, span.end));
    }
    for row in rows {
        canonical.push_str(&format!(
            "|r:{}:{}:{}",
            row.center,
            row.regime_index,
            u8::from(row.boundary)
        ));
    }
    crate::utils::checksum::sha256_hex(canonical.as_bytes())
}

/// Bank hash: canonical digest over the regime definitions and the distinct
/// model identities (PN-FR-023 closure).
pub(super) fn bank_sha256(
    provenance: &str,
    regimes: &[BankRegime],
    slots: &[BankModelSlot],
    schedule_sha: &str,
) -> String {
    let mut canonical = format!("bank/v1|prov={provenance}|schedule={schedule_sha}");
    for regime in regimes {
        canonical.push_str(&format!("|g:{}:{}", regime.name, regime.condition));
        for (start, end) in &regime.intervals {
            canonical.push_str(&format!("|i:{start}:{end}"));
        }
    }
    for slot in slots {
        canonical.push_str(&format!(
            "|m:{}:{}:{}:{}",
            slot.mode, slot.model_id, slot.sha256, slot.estimation_sha256
        ));
    }
    crate::utils::checksum::sha256_hex(canonical.as_bytes())
}

#[cfg(test)]
#[path = "bank_tests.rs"]
mod tests;
