//! PN-M4 acceptance tests (PN-AT-018/019/020/022, PN-NFR-006).
//!
//! Unit-level controls over the dependency-aware paired statistics, plus
//! end-to-end compare runs asserting the separately reported gates, complete
//! accounting and reproducibility. All fixtures are deterministic inline
//! synthetics: no private data, no network, no hardware.

use super::{
    BlockMoments, COMPARE_MEAN_BLOCK_SD_STATISTIC_ID, COMPARE_STABILITY_MAX_CONCENTRATION,
    COMPARE_STATISTIC_ID, COMPARE_TARGET_SD_RATIO, CandidateEvidence, CompareDetrend, LegSignal,
    LegState, StatisticsConfig, assess_candidate, build_accounting, mean_block_scatter,
    pooled_scatter_of, spans_pairwise_disjoint,
};

fn statistics_config(min_blocks: usize) -> StatisticsConfig {
    StatisticsConfig {
        detrend: CompareDetrend::None,
        confidence_level: 0.95,
        target_sd_ratio: COMPARE_TARGET_SD_RATIO,
        min_independent_blocks: min_blocks,
        bootstrap_replicates: 2_000,
        bootstrap_seed: 0x50_4e4d_3401,
    }
}

fn leg(name: &str, series: Vec<(u64, f64)>) -> LegSignal {
    LegSignal {
        name: name.to_string(),
        estimator: "boxcar_legacy".to_string(),
        mode: None,
        state: LegState::Complete,
        code: None,
        message: None,
        model_sha256: None,
        grid_fingerprint: None,
        output_rows: Some(series.len()),
        window_failures: 0,
        series,
        bank_rows: Vec::new(),
        bank_stats: None,
        bank_schedule_sha256: None,
    }
}

/// Deterministic pseudo-noise in [0, 1) keyed by index.
fn unit(index: usize) -> f64 {
    (((index as u64).wrapping_mul(6_364_136_223_846_793_005) >> 11) as f64) / ((1_u64 << 53) as f64)
}

/// One span per output center. In the workflow a statistic block is a
/// role-plan block, so unit fixtures model small explicit blocks.
fn per_center_spans(series: &[(u64, f64)]) -> Vec<(u64, u64)> {
    series
        .iter()
        .map(|(center, _)| (*center, *center + 10))
        .collect()
}

/// Spans grouping `group` consecutive centers into one block, so per-block
/// moments have more than one sample (as role-plan blocks do).
fn grouped_spans(series: &[(u64, f64)], group: usize) -> Vec<(u64, u64)> {
    series
        .chunks(group)
        .map(|chunk| {
            let first = chunk[0].0;
            let last = chunk[chunk.len() - 1].0;
            (first, last + 10)
        })
        .collect()
}

fn blocks_of(len: usize, scale: f64, offset: usize) -> Vec<BlockMoments> {
    (0..len)
        .map(|index| {
            BlockMoments::new(
                (0..8)
                    .map(|sample| scale * (unit(offset + index * 8 + sample) - 0.5))
                    .collect::<Vec<f64>>()
                    .as_slice(),
            )
        })
        .collect()
}

#[test]
fn unequal_length_and_power_blocks_separate_the_two_statistics() {
    // Two block populations: many short quiet blocks and one long loud block.
    // Pooling weights by residual degrees of freedom; averaging block SDs
    // does not, so the two statistics must differ (PN-AT-019).
    let mut baseline = blocks_of(6, 1.0, 11);
    let mut candidate = blocks_of(6, 1.0, 11);
    // The loud long block: same values but 4x the power and 4x the length.
    let loud_baseline = BlockMoments::new(
        &(0..32)
            .map(|sample| 4.0 * (unit(900 + sample) - 0.5))
            .collect::<Vec<f64>>(),
    );
    let loud_candidate = BlockMoments::new(
        &(0..32)
            .map(|sample| 2.0 * (unit(900 + sample) - 0.5))
            .collect::<Vec<f64>>(),
    );
    baseline.push(loud_baseline);
    candidate.push(loud_candidate);
    let pooled = pooled_scatter_of(&baseline, CompareDetrend::None).unwrap();
    let pooled_candidate = pooled_scatter_of(&candidate, CompareDetrend::None).unwrap();
    let pooled_ratio = pooled_candidate / pooled;
    let mean_baseline = mean_block_scatter(&baseline).unwrap();
    let mean_candidate = mean_block_scatter(&candidate).unwrap();
    let mean_ratio = mean_candidate / mean_baseline;
    assert!(
        (pooled_ratio - mean_ratio).abs() > 0.05,
        "pooled ratio {pooled_ratio} and mean-block ratio {mean_ratio} must differ on unequal blocks"
    );
    assert_eq!(COMPARE_STATISTIC_ID, "pooled_residual_scatter_ratio_v1");
    assert_eq!(COMPARE_MEAN_BLOCK_SD_STATISTIC_ID, "mean_block_sd_ratio_v1");
    assert_ne!(COMPARE_STATISTIC_ID, COMPARE_MEAN_BLOCK_SD_STATISTIC_ID);
}

#[test]
fn stratified_bootstrap_is_deterministic_seed_sensitive_and_region_stratified() {
    // Two regions with different scales: the interval must resample within
    // each region (prespecified stratification) and stay reproducible for a
    // frozen seed (PN-FR-029, PN-NFR-006).
    let statistics = statistics_config(4);
    let baseline = leg(
        "boxcar_baseline",
        vec![
            (0, 1.0),
            (10, 1.1),
            (20, 0.9),
            (30, 1.05),
            (1_000, 3.0),
            (1_010, 3.2),
            (1_020, 2.8),
            (1_030, 3.1),
        ],
    );
    let candidate = leg(
        "identity_candidate",
        vec![
            (0, 0.9),
            (10, 1.0),
            (20, 0.8),
            (30, 0.95),
            (1_000, 2.7),
            (1_010, 2.9),
            (1_020, 2.5),
            (1_030, 2.8),
        ],
    );
    let strata = vec![
        per_center_spans(&baseline.series[..4]),
        per_center_spans(&baseline.series[4..]),
    ];
    let first = assess_candidate(&candidate, &baseline, "stratified", &strata, 1, &statistics);
    let second = assess_candidate(&candidate, &baseline, "stratified", &strata, 1, &statistics);
    assert_eq!(first.strata, 2);
    assert_eq!(first.independent_blocks, 8);
    assert_eq!(first.paired_ci, second.paired_ci);
    assert_eq!(first.sd_ratio, second.sd_ratio);
    let mut other_seed = statistics_config(4);
    other_seed.bootstrap_seed = statistics.bootstrap_seed + 7;
    let shifted = assess_candidate(&candidate, &baseline, "stratified", &strata, 1, &other_seed);
    assert_ne!(
        shifted.paired_ci, first.paired_ci,
        "an interval must depend on its recorded seed"
    );
    // Stratification matters: an unstratified (single-region) bootstrap over
    // the pooled blocks gives a different interval, so the policy is explicit.
    let pooled = assess_candidate(
        &candidate,
        &baseline,
        "pooled",
        &[per_center_spans(&baseline.series)],
        1,
        &statistics,
    );
    assert_ne!(pooled.paired_ci, first.paired_ci);
    assert_eq!(first.statistic_id, COMPARE_STATISTIC_ID);
    assert_eq!(
        first.secondary_statistic_id,
        COMPARE_MEAN_BLOCK_SD_STATISTIC_ID
    );
}

#[test]
fn overlapping_supports_are_never_counted_as_independent() {
    let statistics = statistics_config(2);
    let baseline = leg("boxcar_baseline", vec![(0, 1.0), (10, 1.2), (20, 0.8)]);
    let candidate = leg("identity_candidate", vec![(0, 0.9), (10, 1.1), (20, 0.7)]);
    assert!(!spans_pairwise_disjoint(&[(0, 30), (10, 40)]));
    let evidence = assess_candidate(
        &candidate,
        &baseline,
        "overlap",
        &[vec![(0, 30)], vec![(10, 40)]],
        1,
        &statistics,
    );
    assert!(evidence.sd_ratio.is_none());
    assert!(evidence.paired_ci.is_none());
    assert_eq!(evidence.benefit_gate, "inconclusive");
    assert!(
        evidence
            .notes
            .iter()
            .any(|note| note.contains("overlapping evaluation supports")),
        "overlap must be refused with an explicit note: {:?}",
        evidence.notes
    );
}

#[test]
fn zero_denominators_and_too_few_blocks_stay_explicit() {
    let statistics = statistics_config(6);
    // A constant candidate block has zero scatter: it must be counted, not
    // silently dropped, and the pooled statistic stays computable.
    let baseline = leg(
        "boxcar_baseline",
        vec![
            (0, 1.0),
            (10, 1.2),
            (20, 0.8),
            (30, 1.1),
            (40, 0.9),
            (50, 1.05),
            (60, 0.95),
            (70, 1.0),
        ],
    );
    let candidate = leg(
        "identity_candidate",
        vec![
            (0, 0.0),
            (10, 0.0),
            (20, 0.0),
            (30, 0.0),
            (40, 0.9),
            (50, 1.05),
            (60, 0.95),
            (70, 1.0),
        ],
    );
    let evidence = assess_candidate(
        &candidate,
        &baseline,
        "zero",
        &[per_center_spans(&baseline.series)],
        1,
        &statistics,
    );
    assert!(evidence.zero_denominator_blocks >= 1);
    assert!(evidence.sd_ratio.is_some());
    // Too few blocks: explicit inconclusive evidence, never a pass.
    let thin = assess_candidate(
        &candidate,
        &baseline,
        "thin",
        &[vec![(0, 20)]],
        1,
        &statistics,
    );
    assert!(thin.paired_ci.is_none());
    assert_eq!(thin.benefit_gate, "inconclusive");
    assert!(
        thin.notes
            .iter()
            .any(|note| note.contains("independent blocks")),
        "too-few-block reason must be recorded: {:?}",
        thin.notes
    );
}

#[test]
fn familywise_interval_is_at_least_as_wide_as_the_unadjusted_one() {
    let statistics = statistics_config(2);
    let baseline = leg(
        "boxcar_baseline",
        (0..12)
            .map(|index| (index as u64 * 10, 1.0 + unit(index + 3) - 0.5))
            .collect(),
    );
    let candidate = leg(
        "identity_candidate",
        (0..12)
            .map(|index| (index as u64 * 10, 0.95 + 0.9 * (unit(index + 3) - 0.5)))
            .collect(),
    );
    let evidence = assess_candidate(
        &candidate,
        &baseline,
        "family",
        &[per_center_spans(&baseline.series)],
        4,
        &statistics,
    );
    let paired = evidence.paired_ci.unwrap();
    let familywise = evidence.familywise_ci.unwrap();
    assert!(familywise[0] <= paired[0] + 1e-12);
    assert!(familywise[1] >= paired[1] - 1e-12);
}

#[test]
fn transient_dominated_reduction_fails_the_stability_control() {
    let statistics = statistics_config(2);
    // Every block improves a little, but one block carries almost all of the
    // reduction: the pooled ratio improves while the reduction is
    // transient-dominated and must not be labeled a noise gain (PN-FR-028).
    let mut baseline_series = Vec::new();
    let mut candidate_series = Vec::new();
    for index in 0..20usize {
        let value = 1.0 + unit(index) - 0.5;
        baseline_series.push((index as u64 * 10, value));
        let scaled = if index < 5 { 0.1 * value } else { 0.9 * value };
        candidate_series.push((index as u64 * 10, scaled));
    }
    let evidence = assess_candidate(
        &leg("identity_candidate", candidate_series),
        &leg("boxcar_baseline", baseline_series.clone()),
        "transient",
        &[grouped_spans(&baseline_series, 5)],
        1,
        &statistics,
    );
    assert_eq!(evidence.stability_gate, "fail");
    let concentration = evidence.reduction_concentration.unwrap();
    assert!(
        concentration > COMPARE_STABILITY_MAX_CONCENTRATION,
        "reduction concentration {concentration} must exceed the guard"
    );
    assert!(
        evidence
            .notes
            .iter()
            .any(|note| note.contains("not a noise gain")),
        "the transient note must be explicit: {:?}",
        evidence.notes
    );
}

#[test]
fn a_shared_common_trend_does_not_manufacture_benefit() {
    // Both legs carry the same large trend: the paired ratio stays near 1
    // under every reported detrending policy (common-trend sensitivity).
    let statistics = statistics_config(2);
    let mut baseline_series = Vec::new();
    let mut candidate_series = Vec::new();
    for index in 0..12usize {
        let trend = index as f64 * 0.5;
        baseline_series.push((index as u64 * 10, trend + 1.0 + unit(index) - 0.5));
        candidate_series.push((index as u64 * 10, trend + 1.0 + unit(index + 50) - 0.5));
    }
    let evidence = assess_candidate(
        &leg("identity_candidate", candidate_series),
        &leg("boxcar_baseline", baseline_series.clone()),
        "trend",
        &[grouped_spans(&baseline_series, 3)],
        1,
        &statistics,
    );
    for ratio in [
        evidence.detrend_ratio_none.unwrap(),
        evidence.detrend_ratio_block_mean.unwrap(),
    ] {
        assert!(
            (ratio - 1.0).abs() < 0.3,
            "a shared trend must not manufacture benefit, got ratio {ratio}"
        );
    }
    assert_ne!(evidence.benefit_gate, "pass");
    assert!((evidence.mean_shift.unwrap()).abs() < 0.5);
}

#[test]
fn accounting_enumerates_every_requested_method_and_region() {
    let request_text = r#"
schema_version = 1
operation = "compare"
reference_frequency_hz = 1000.0
reference_phase_rad = 0.0
sample_interval_s = 0.00001
block_len = 100
output = "comparison"

[source]
kind = "recorded_csv"
path = "wave.csv"

[source.channels]
detector = 3

[source.grid]
stride = 10

[study]
classification = "exploratory"

[[roles]]
role = "training"
start = 0
end = 100

[calibration]

[context]
rotation_rad = 0.0
modulation_depth = 1.0
field_factor = 1.0

[candidates]
include_boxcar_baseline = true
joint_modes = ["identity", "phase_diagonal"]
"#;
    let request: super::CompareRequest = toml::from_str(request_text).unwrap();
    let spans = vec![(0u64, 100u64), (100u64, 200u64)];
    let baseline_leg = leg("boxcar_baseline", vec![(0, 1.0), (10, 1.1)]);
    let mut identity = leg("identity_candidate", vec![(0, 0.9), (10, 1.0)]);
    identity.mode = Some("identity".to_string());
    let mut unavailable = leg("stationary_correlated_candidate", Vec::new());
    unavailable.mode = Some("stationary_correlated".to_string());
    unavailable.state = LegState::Unavailable;
    unavailable.message = Some("model_unavailable: insufficient reserved blocks".to_string());
    let legs = vec![baseline_leg, identity, unavailable];
    let empty_evidence: Vec<CandidateEvidence> = Vec::new();
    let accounting = build_accounting(&legs, &empty_evidence, &spans, 3);
    // Complete legs contribute one row per region; terminal legs keep one
    // explicit record with a reason.
    assert_eq!(
        accounting
            .iter()
            .filter(|row| row.method == "identity")
            .count(),
        2
    );
    let terminal = accounting
        .iter()
        .find(|row| row.method == "stationary_correlated")
        .expect("unavailable method must keep its record");
    assert_eq!(terminal.outcome, "unavailable");
    assert!(terminal.reason.is_some());
    assert!(accounting.iter().all(|row| row.channel == 3));
    assert!(!super::accounting_complete(&accounting, &request, &spans));
}

fn adequate_model() -> super::FrozenModelRecord {
    super::FrozenModelRecord {
        mode: "identity".to_string(),
        model_id: "identity".to_string(),
        artifact_path: "calibrations/ch3/identity/model.json".to_string(),
        sha256: "0".repeat(64),
        bytes: 1,
        correlation_basis: "phase-standardized".to_string(),
        phase_bins: 64,
        max_lag: None,
        training_blocks: 8,
        training_intervals: 2,
        reserved_blocks_measured: 8,
        adequacy_adequate: true,
        adequacy_reason: "adequate".to_string(),
        status: "frozen".to_string(),
        reason: None,
    }
}

fn evidence_with_benefit(gate: &str) -> CandidateEvidence {
    let statistics = statistics_config(2);
    let baseline = leg("boxcar_baseline", vec![(0, 1.0)]);
    let candidate = leg("identity_candidate", vec![(0, 0.9)]);
    let mut evidence =
        super::unassessable_evidence(&candidate, &baseline, "unit", &statistics, Vec::new());
    evidence.sd_ratio = Some(0.90);
    evidence.paired_ci = Some([0.85, 0.95]);
    evidence.benefit_gate = gate.to_string();
    evidence
}

#[test]
fn a_numeric_benefit_pass_cannot_override_unverified_controls() {
    // PN-AT-022 / PN-FR-032: SD ratio <= 0.97 with a paired interval upper
    // endpoint below 1 still cannot override unverified fidelity or grant
    // default promotion.
    let evidence = vec![evidence_with_benefit("pass")];
    let gates = super::assemble_gates("complete", "valid", &[adequate_model()], &evidence, 3, 3);
    assert_eq!(gates.numeric_benefit, "pass");
    assert_eq!(gates.dynamic_fidelity, "unverified");
    assert_ne!(gates.scientific, "pass");
    assert_eq!(gates.scientific, "inconclusive");
    assert_eq!(gates.default_promotion, "not_authorized");
    assert_eq!(gates.model_adequacy, "adequate");
    // The same gate assembly reports a genuine no-benefit outcome as a
    // completed evaluation (never as an operational failure).
    let gates = super::assemble_gates(
        "complete",
        "valid",
        &[adequate_model()],
        &[evidence_with_benefit("fail")],
        3,
        3,
    );
    assert_eq!(gates.numeric_benefit, "fail");
    assert_eq!(gates.computation, "complete");
    assert_ne!(gates.scientific, "pass");
    assert_eq!(gates.default_promotion, "not_authorized");
}
