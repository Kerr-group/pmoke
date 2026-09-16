//! Controlled M6 evaluation protocol and truthful statistical verdicts.
//!
//! This command evaluates already-produced paired baseline/candidate values
//! under a frozen protocol. It never acquires hardware, infers a physical
//! noise mechanism, or promotes an estimator default. A completed negative
//! result is a valid report; insufficient blocks or an unconfirmed feature
//! tolerance are explicitly inconclusive.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const EVALUATION_REQUEST_SCHEMA_VERSION: u32 = 1;
/// Versioned identity of the statistic this lane computes (PN-FR-036): a
/// ratio of average block standard deviations. It is a different estimator
/// from the compare lane's pooled residual-scatter ratio; the two are never
/// silently renamed into each other.
pub const EVALUATE_STATISTIC_ID: &str = "mean_block_sd_ratio_v1";
pub const EVALUATE_STATISTIC_FORMULA: &str = "mean_block_sd(candidate) / mean_block_sd(baseline) over the declared equal-length evaluation blocks (not pooled-variance equivalent)";
pub const EVALUATE_RESAMPLING_UNIT: &str = "whole_equal_length_evaluation_blocks";
pub const EVALUATE_MULTIPLICITY_POLICY_ID: &str = "single_candidate_no_selection/v1";
pub const EVALUATE_PROMOTION_POLICY: &str = "not_authorized";
const BOOTSTRAP_REPLICATES: usize = 10_000;
const BOOTSTRAP_SEED: u64 = 0x4d_3645_7661_6c31;
const BOOTSTRAP_RNG_ID: &str = "lcg6364136223846793005/v1";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationRequest {
    pub schema_version: u32,
    pub gate_version: String,
    pub source_fingerprint_sha256: String,
    pub method_label: String,
    pub channel: u8,
    pub interval_label: String,
    pub evaluation_start_s: f64,
    pub evaluation_end_s: f64,
    pub reference_frequency_hz: f64,
    pub rotation_rad: f64,
    pub modulation_depth: f64,
    pub field_factor: f64,
    pub block_length: usize,
    pub min_independent_blocks: usize,
    pub confidence_level: f64,
    pub target_sd_ratio: f64,
    #[serde(default)]
    pub detrend: DetrendPolicy,
    pub feature_tolerance_confirmed: bool,
    pub design_blocks: usize,
    pub tuning_blocks: usize,
    pub baseline: Vec<f64>,
    pub candidate: Vec<f64>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DetrendPolicy {
    #[default]
    None,
    BlockMean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Pass,
    Fail,
    Inconclusive,
    Unverified,
}

#[derive(Debug, Clone, Serialize)]
pub struct ControlStatuses {
    pub known_noise: Verdict,
    pub dynamic_fidelity: Verdict,
    pub private_measurement: Verdict,
}

#[derive(Debug, Clone, Serialize)]
pub struct EvaluationReport {
    pub schema_version: u32,
    pub gate_version: String,
    pub source_fingerprint_sha256: String,
    pub method_label: String,
    pub channel: u8,
    pub interval_label: String,
    pub evaluation_start_s: f64,
    pub evaluation_end_s: f64,
    pub reference_frequency_hz: f64,
    pub rotation_rad: f64,
    pub modulation_depth: f64,
    pub field_factor: f64,
    pub detrend: DetrendPolicy,
    pub design_blocks: usize,
    pub tuning_blocks: usize,
    pub evaluation_samples: usize,
    pub independent_blocks: usize,
    pub discarded_tail_samples: usize,
    pub baseline_block_sd_mean: Option<f64>,
    pub candidate_block_sd_mean: Option<f64>,
    pub sd_ratio: Option<f64>,
    pub paired_bootstrap_ci: Option<[f64; 2]>,
    pub target_sd_ratio: f64,
    pub confidence_level: f64,
    pub benefit_gate: Verdict,
    pub scientific_verdict: Verdict,
    pub computation_complete: bool,
    pub controls: ControlStatuses,
    /// Versioned statistic identity and reproducibility policy (PN-FR-036,
    /// PN-NFR-006): this lane reports the mean-block-SD ratio, never the
    /// compare lane's pooled residual-scatter ratio.
    pub statistic_id: String,
    pub statistic_formula: String,
    pub pooled_statistic_id: String,
    pub resampling_unit: String,
    pub bootstrap_replicates: usize,
    pub bootstrap_seed: u64,
    pub rng_id: String,
    pub multiplicity_policy_id: String,
    pub promotion: String,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
struct BlockPair {
    baseline_sd: f64,
    candidate_sd: f64,
}

pub fn run_evaluate(request_path: &Path, output: Option<&Path>) -> Result<()> {
    let text = std::fs::read_to_string(request_path)
        .with_context(|| format!("cannot read evaluation request: {}", request_path.display()))?;
    let request: EvaluationRequest = toml::from_str(&text)
        .with_context(|| format!("invalid evaluation request: {}", request_path.display()))?;
    let report = evaluate_request(&request)?;
    let encoded =
        serde_json::to_string_pretty(&report).context("cannot encode evaluation report")?;
    let output_path = output.map(Path::to_path_buf).unwrap_or_else(|| {
        request_path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("m6-evaluation-report.json")
    });
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "cannot create evaluation output parent: {}",
                parent.display()
            )
        })?;
    }
    std::fs::write(&output_path, format!("{encoded}\n"))
        .with_context(|| format!("cannot write evaluation report: {}", output_path.display()))?;
    println!("{encoded}");
    // A completed negative or inconclusive scientific result is still a
    // completed report. Only malformed requests fail the command.
    Ok(())
}

pub fn evaluate_request(request: &EvaluationRequest) -> Result<EvaluationReport> {
    validate_request(request)?;
    let pairs = block_pairs(request)?;
    let independent_blocks = pairs.len();
    let discarded_tail_samples = request.baseline.len() % request.block_length;
    let mut notes = vec![
        "Residual variance is not identified with a physical noise mechanism.".to_string(),
        "Private measurement evaluation was not run by this synthetic/statistical command."
            .to_string(),
        "This lane reports the mean-block-SD ratio statistic; it is not pooled-variance equivalent to the compare lane's pooled residual-scatter statistic (PN-FR-036).".to_string(),
    ];
    if request.design_blocks == 0 || request.tuning_blocks == 0 {
        notes.push("Design and tuning role counts are incomplete; scientific qualification is not established.".to_string());
    }

    let (
        baseline_block_sd_mean,
        candidate_block_sd_mean,
        sd_ratio,
        paired_bootstrap_ci,
        benefit_gate,
    ) = if independent_blocks == 0 {
        notes.push("No complete evaluation blocks were available.".to_string());
        (None, None, None, None, Verdict::Inconclusive)
    } else if independent_blocks < request.min_independent_blocks {
        notes.push(format!(
            "Only {independent_blocks} independent blocks are available; required minimum is {}.",
            request.min_independent_blocks
        ));
        let baseline = mean(pairs.iter().map(|pair| pair.baseline_sd));
        let candidate = mean(pairs.iter().map(|pair| pair.candidate_sd));
        (
            baseline,
            candidate,
            ratio(baseline, candidate),
            None,
            Verdict::Inconclusive,
        )
    } else {
        let baseline = mean(pairs.iter().map(|pair| pair.baseline_sd));
        let candidate = mean(pairs.iter().map(|pair| pair.candidate_sd));
        let ratio = ratio(baseline, candidate);
        let ci = bootstrap_ratio_ci(&pairs, request.confidence_level);
        let gate = match (ratio, ci) {
            (Some(ratio), Some(ci)) if ratio <= request.target_sd_ratio && ci[1] < 1.0 => {
                Verdict::Pass
            }
            (Some(_), Some(_)) => Verdict::Fail,
            _ => Verdict::Unverified,
        };
        (baseline, candidate, ratio, ci, gate)
    };

    let scientific_verdict = if benefit_gate == Verdict::Unverified {
        Verdict::Unverified
    } else if !request.feature_tolerance_confirmed {
        notes.push("The retained fastest physical feature and fidelity tolerance are not confirmed (A-006/O-002).".to_string());
        Verdict::Inconclusive
    } else if request.design_blocks == 0 || request.tuning_blocks == 0 {
        Verdict::Inconclusive
    } else {
        benefit_gate
    };

    Ok(EvaluationReport {
        schema_version: EVALUATION_REQUEST_SCHEMA_VERSION,
        gate_version: request.gate_version.clone(),
        source_fingerprint_sha256: request.source_fingerprint_sha256.clone(),
        method_label: request.method_label.clone(),
        channel: request.channel,
        interval_label: request.interval_label.clone(),
        evaluation_start_s: request.evaluation_start_s,
        evaluation_end_s: request.evaluation_end_s,
        reference_frequency_hz: request.reference_frequency_hz,
        rotation_rad: request.rotation_rad,
        modulation_depth: request.modulation_depth,
        field_factor: request.field_factor,
        detrend: request.detrend,
        design_blocks: request.design_blocks,
        tuning_blocks: request.tuning_blocks,
        evaluation_samples: request.baseline.len(),
        independent_blocks,
        discarded_tail_samples,
        baseline_block_sd_mean,
        candidate_block_sd_mean,
        sd_ratio,
        paired_bootstrap_ci,
        target_sd_ratio: request.target_sd_ratio,
        confidence_level: request.confidence_level,
        benefit_gate,
        scientific_verdict,
        computation_complete: true,
        controls: ControlStatuses {
            known_noise: Verdict::Unverified,
            dynamic_fidelity: Verdict::Unverified,
            private_measurement: Verdict::Unverified,
        },
        statistic_id: EVALUATE_STATISTIC_ID.to_string(),
        statistic_formula: EVALUATE_STATISTIC_FORMULA.to_string(),
        pooled_statistic_id: crate::commands::noise::compare::COMPARE_STATISTIC_ID.to_string(),
        resampling_unit: EVALUATE_RESAMPLING_UNIT.to_string(),
        bootstrap_replicates: BOOTSTRAP_REPLICATES,
        bootstrap_seed: BOOTSTRAP_SEED,
        rng_id: BOOTSTRAP_RNG_ID.to_string(),
        multiplicity_policy_id: EVALUATE_MULTIPLICITY_POLICY_ID.to_string(),
        promotion: EVALUATE_PROMOTION_POLICY.to_string(),
        notes,
    })
}

fn validate_request(request: &EvaluationRequest) -> Result<()> {
    if request.schema_version != EVALUATION_REQUEST_SCHEMA_VERSION {
        bail!(
            "unsupported evaluation request schema_version {} (expected {EVALUATION_REQUEST_SCHEMA_VERSION})",
            request.schema_version
        );
    }
    if request.gate_version.trim().is_empty()
        || request.method_label.trim().is_empty()
        || request.interval_label.trim().is_empty()
    {
        bail!("evaluation request labels and gate_version must not be empty");
    }
    if request.source_fingerprint_sha256.len() != 64
        || !request
            .source_fingerprint_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("source_fingerprint_sha256 must be a 64-character hexadecimal digest");
    }
    for (name, value) in [
        ("evaluation_start_s", request.evaluation_start_s),
        ("evaluation_end_s", request.evaluation_end_s),
        ("reference_frequency_hz", request.reference_frequency_hz),
        ("rotation_rad", request.rotation_rad),
        ("modulation_depth", request.modulation_depth),
        ("field_factor", request.field_factor),
        ("confidence_level", request.confidence_level),
        ("target_sd_ratio", request.target_sd_ratio),
    ] {
        if !value.is_finite() {
            bail!("evaluation request {name} must be finite");
        }
    }
    if request.evaluation_end_s <= request.evaluation_start_s {
        bail!("evaluation interval must have positive duration");
    }
    if !(0.0 < request.confidence_level && request.confidence_level < 1.0) {
        bail!("confidence_level must be between 0 and 1");
    }
    if request.target_sd_ratio <= 0.0
        || request.block_length == 0
        || request.min_independent_blocks == 0
    {
        bail!("target_sd_ratio, block_length, and min_independent_blocks must be positive");
    }
    if request.baseline.len() != request.candidate.len() || request.baseline.is_empty() {
        bail!("baseline and candidate evaluation arrays must be non-empty and equal length");
    }
    for (index, (&baseline, &candidate)) in
        request.baseline.iter().zip(&request.candidate).enumerate()
    {
        if !baseline.is_finite() || !candidate.is_finite() {
            bail!("evaluation arrays contain a non-finite value at index {index}");
        }
    }
    Ok(())
}

fn block_pairs(request: &EvaluationRequest) -> Result<Vec<BlockPair>> {
    let block_count = request.baseline.len() / request.block_length;
    let mut pairs = Vec::with_capacity(block_count);
    for block in 0..block_count {
        let start = block * request.block_length;
        let end = start + request.block_length;
        let baseline = detrended_sd(&request.baseline[start..end], request.detrend);
        let candidate = detrended_sd(&request.candidate[start..end], request.detrend);
        if !baseline.is_finite() || !candidate.is_finite() {
            bail!("block {block} produced a non-finite standard deviation");
        }
        pairs.push(BlockPair {
            baseline_sd: baseline,
            candidate_sd: candidate,
        });
    }
    Ok(pairs)
}

fn detrended_sd(values: &[f64], policy: DetrendPolicy) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let sum = values
        .iter()
        .map(|value| {
            let residual = match policy {
                DetrendPolicy::None => *value,
                DetrendPolicy::BlockMean => *value - mean,
            };
            residual * residual
        })
        .sum::<f64>();
    (sum / values.len() as f64).sqrt()
}

fn mean(values: impl Iterator<Item = f64>) -> Option<f64> {
    let values: Vec<f64> = values.collect();
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn ratio(baseline: Option<f64>, candidate: Option<f64>) -> Option<f64> {
    match (baseline, candidate) {
        (Some(baseline), Some(candidate)) if baseline.is_finite() && baseline > 0.0 => {
            Some(candidate / baseline)
        }
        _ => None,
    }
}

fn bootstrap_ratio_ci(pairs: &[BlockPair], confidence_level: f64) -> Option<[f64; 2]> {
    if pairs.is_empty() {
        return None;
    }
    let mut ratios = Vec::with_capacity(BOOTSTRAP_REPLICATES);
    let mut state = BOOTSTRAP_SEED;
    for _ in 0..BOOTSTRAP_REPLICATES {
        let mut sum = 0.0;
        for _ in pairs {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            let index = (state as usize) % pairs.len();
            let pair = pairs[index];
            if pair.baseline_sd <= 0.0 {
                return None;
            }
            sum += pair.candidate_sd / pair.baseline_sd;
        }
        ratios.push(sum / pairs.len() as f64);
    }
    ratios.sort_by(f64::total_cmp);
    let alpha = (1.0 - confidence_level).clamp(0.0, 1.0);
    let low = ((ratios.len() - 1) as f64 * alpha / 2.0).round() as usize;
    let high = ((ratios.len() - 1) as f64 * (1.0 - alpha / 2.0)).round() as usize;
    Some([ratios[low], ratios[high]])
}

#[cfg(test)]
mod tests {
    use super::*;

    const FINGERPRINT: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn request(blocks: usize, candidate_scale: f64) -> EvaluationRequest {
        let baseline_block = [-2.0, -1.0, 1.0, 2.0];
        let mut baseline = Vec::new();
        let mut candidate = Vec::new();
        for _ in 0..blocks {
            baseline.extend(baseline_block);
            candidate.extend(baseline_block.map(|value| value * candidate_scale));
        }
        EvaluationRequest {
            schema_version: 1,
            gate_version: "m6-sd-ratio-v1".to_string(),
            source_fingerprint_sha256: FINGERPRINT.to_string(),
            method_label: "joint_identity".to_string(),
            channel: 3,
            interval_label: "steady_pre_pulse".to_string(),
            evaluation_start_s: 0.0,
            evaluation_end_s: 1.0,
            reference_frequency_hz: 1_000.0,
            rotation_rad: 0.2,
            modulation_depth: 0.92,
            field_factor: 1.0,
            block_length: 4,
            min_independent_blocks: 20,
            confidence_level: 0.95,
            target_sd_ratio: 0.97,
            detrend: DetrendPolicy::None,
            feature_tolerance_confirmed: true,
            design_blocks: 20,
            tuning_blocks: 20,
            baseline,
            candidate,
        }
    }

    #[test]
    fn accepted_gate_reports_pass_for_known_synthetic_reduction() {
        let report = evaluate_request(&request(40, 0.9)).unwrap();
        assert_eq!(report.benefit_gate, Verdict::Pass);
        assert_eq!(report.scientific_verdict, Verdict::Pass);
        assert!(report.computation_complete);
        assert!(report.paired_bootstrap_ci.unwrap()[1] < 1.0);
    }

    #[test]
    fn negative_result_is_complete_but_fails_gate() {
        let report = evaluate_request(&request(40, 1.1)).unwrap();
        assert_eq!(report.benefit_gate, Verdict::Fail);
        assert_eq!(report.scientific_verdict, Verdict::Fail);
        assert!(report.computation_complete);
    }

    #[test]
    fn insufficient_blocks_are_inconclusive_not_failure() {
        let report = evaluate_request(&request(4, 0.8)).unwrap();
        assert_eq!(report.benefit_gate, Verdict::Inconclusive);
        assert_eq!(report.scientific_verdict, Verdict::Inconclusive);
        assert!(report.computation_complete);
    }

    #[test]
    fn unconfirmed_feature_tolerance_blocks_scientific_promotion() {
        let mut request = request(40, 0.9);
        request.feature_tolerance_confirmed = false;
        let report = evaluate_request(&request).unwrap();
        assert_eq!(report.benefit_gate, Verdict::Pass);
        assert_eq!(report.scientific_verdict, Verdict::Inconclusive);
    }

    #[test]
    fn malformed_source_fingerprint_is_rejected() {
        let mut request = request(40, 0.9);
        request.source_fingerprint_sha256 = "not-a-digest".to_string();
        assert!(evaluate_request(&request).is_err());
    }

    #[test]
    fn statistic_identity_is_versioned_and_distinct_from_the_pooled_compare_statistic() {
        // PN-FR-036 / PN-AT-018: the evaluate-lockin statistic keeps its own
        // identity; it is never silently renamed to the compare lane's pooled
        // residual-scatter ratio.
        let report = evaluate_request(&request(40, 0.9)).unwrap();
        assert_eq!(report.statistic_id, EVALUATE_STATISTIC_ID);
        assert_eq!(report.statistic_id, "mean_block_sd_ratio_v1");
        assert_ne!(
            report.statistic_id,
            crate::commands::noise::compare::COMPARE_STATISTIC_ID
        );
        assert_eq!(
            report.pooled_statistic_id,
            crate::commands::noise::compare::COMPARE_STATISTIC_ID
        );
        assert_eq!(report.promotion, "not_authorized");
        assert_eq!(report.resampling_unit, EVALUATE_RESAMPLING_UNIT);
        assert!(
            report
                .notes
                .iter()
                .any(|note| note.contains("not pooled-variance equivalent"))
        );
        // No scientific result grants promotion authority, even when the
        // numeric benefit gate passes (PN-FR-032).
        assert_eq!(report.benefit_gate, Verdict::Pass);
        assert_eq!(report.promotion, "not_authorized");
    }
}
