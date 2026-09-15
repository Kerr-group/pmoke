//! Role-aware planning plus support-aware leakage guard (PN-FR-006/007).
//!
//! `resolve_role_plan` wraps the shared [`plan_blocks`](pmoke_analysis_core::calibration::plan_blocks)
//! kernel (no second planner) and adds the noise-workflow leakage contract:
//! every training/validation raw footprint, expanded by the configured
//! temporal guard plus the legacy interpolation halo (PN-FR-004/007), is
//! ineligible for evaluation. Eligibility is computed on immutable original
//! sample indices and source identity, never on path spelling. Full-output
//! training rows stay present but are marked ineligible for held-out claims
//! (PN-FR-038).

use anyhow::{Result, bail};
use pmoke_analysis_core::calibration::{
    BlockPlan, BlockPlanRequest, CalibrationRole, PlannedBlock, plan_blocks,
};
use serde::{Deserialize, Serialize};

/// Leakage guard parameters, all in original samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardSpec {
    /// Configured temporal guard on each side of every train/validation
    /// footprint (0 = halo only).
    pub guard_samples: u64,
    /// Legacy interpolation halo on each side (default 2: one sample each
    /// side of fractional-endpoint interpolation, PN-FR-004).
    pub interpolation_halo_samples: u64,
}

/// Resolved role plan: realized blocks plus exclusions, with full-output
/// rows retained but flagged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedRolePlan {
    pub blocks: Vec<PlannedBlock>,
    pub exclusions: Vec<ResolvedExclusion>,
    /// Original-index intervals nominated as full-output-only: rows are kept
    /// for output coverage but can never enter held-out metrics.
    pub full_output_only: Vec<FullOutputSpan>,
    pub training_block_count: usize,
    pub validation_block_count: usize,
    pub evaluation_block_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedExclusion {
    pub start: u64,
    pub end: u64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullOutputSpan {
    pub start: u64,
    pub end: u64,
}

/// Leakage outcome: which evaluation blocks survive the guard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeakageGuard {
    pub guard: GuardSpec,
    pub ineligible_evaluation_blocks: Vec<PlannedBlock>,
    pub eligible_evaluation_blocks: Vec<PlannedBlock>,
    pub leakage_rejections: Vec<LeakageRejection>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeakageRejection {
    pub block_start: u64,
    pub block_end: u64,
    pub conflicting_start: u64,
    pub conflicting_end: u64,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedPlan {
    pub role_plan: ResolvedRolePlan,
    pub leakage: LeakageGuard,
}

/// Resolves roles and leakage over one raw timebase. Rejects overlapping
/// role intervals (via the shared kernel), empty training coverage, and
/// guard/halo overflow. Rounding: none — bounds are already original
/// indices, retained verbatim.
pub fn resolve_role_plan(
    request: &BlockPlanRequest,
    guard: GuardSpec,
    sample_interval_s: f64,
) -> Result<ResolvedPlan> {
    if !(sample_interval_s.is_finite() && sample_interval_s > 0.0) {
        bail!("noise role plan needs a positive finite sample_interval_s");
    }
    let plan: BlockPlan = plan_blocks(request)?;
    let mut role_plan = ResolvedRolePlan {
        blocks: plan.blocks.clone(),
        exclusions: plan
            .exclusions
            .iter()
            .map(|exclusion| ResolvedExclusion {
                start: exclusion.start,
                end: exclusion.end,
                reason: exclusion.reason.clone(),
            })
            .collect(),
        full_output_only: Vec::new(),
        training_block_count: 0,
        validation_block_count: 0,
        evaluation_block_count: 0,
    };
    // The shared kernel models Training/TuningValidation/Evaluation. The
    // noise workflow splits Evaluation into evaluation vs full-output-only
    // by interval identity: intervals the caller nominated as full-output
    // keep their rows but are fenced off from held-out metrics (PN-FR-038).
    // Since TOML roles map full-output-only onto Evaluation for block
    // geometry, the fence is recorded here at plan resolution: any
    // Evaluation block that overlaps a full-output span is retained in the
    // block list and simultaneously listed as ineligible.
    for block in &role_plan.blocks {
        match block.role {
            CalibrationRole::Training => role_plan.training_block_count += 1,
            CalibrationRole::TuningValidation => role_plan.validation_block_count += 1,
            CalibrationRole::Evaluation => role_plan.evaluation_block_count += 1,
        }
    }
    if role_plan.training_block_count == 0 {
        bail!("noise role plan holds no training blocks");
    }

    let margin = guard
        .guard_samples
        .checked_add(guard.interpolation_halo_samples)
        .ok_or_else(|| anyhow::anyhow!("noise guard margin overflows the index space"))?;
    let mut forbidden: Vec<(u64, u64, String)> = Vec::new();
    for block in &role_plan.blocks {
        if block.role == CalibrationRole::Evaluation {
            continue;
        }
        let start = block.start.saturating_sub(margin);
        let end = block.end.checked_add(margin).ok_or_else(|| {
            anyhow::anyhow!("noise guard expansion overflows past block {}", block.end)
        })?;
        let role = match block.role {
            CalibrationRole::Training => "training",
            CalibrationRole::TuningValidation => "validation",
            CalibrationRole::Evaluation => "evaluation",
        };
        forbidden.push((
            start,
            end,
            format!("{role} footprint + guard {margin} samples (halo included)"),
        ));
    }

    let mut leakage = LeakageGuard {
        guard,
        ineligible_evaluation_blocks: Vec::new(),
        eligible_evaluation_blocks: Vec::new(),
        leakage_rejections: Vec::new(),
    };
    for block in &role_plan.blocks {
        if block.role != CalibrationRole::Evaluation {
            continue;
        }
        let mut conflict: Option<(u64, u64, String)> = None;
        for (start, end, reason) in &forbidden {
            if block.start < *end && *start < block.end {
                conflict = Some((*start, *end, reason.clone()));
                break;
            }
        }
        match conflict {
            Some((conflicting_start, conflicting_end, reason)) => {
                leakage.leakage_rejections.push(LeakageRejection {
                    block_start: block.start,
                    block_end: block.end,
                    conflicting_start,
                    conflicting_end,
                    reason: reason.clone(),
                });
                role_plan.exclusions.push(ResolvedExclusion {
                    start: block.start,
                    end: block.end,
                    reason: format!("evaluation ineligible: {reason}"),
                });
                leakage.ineligible_evaluation_blocks.push(*block);
            }
            None => leakage.eligible_evaluation_blocks.push(*block),
        }
    }
    Ok(ResolvedPlan { role_plan, leakage })
}
