//! Recorded-only pulse-noise diagnosis (PN-M1, Issue #246).
//!
//! `pmoke noise diagnose --request REQUEST.toml` opens one recorded source
//! through [`RecordedSource`](crate::utils::recorded_source::RecordedSource),
//! resolves an explicit role plan over original sample indices
//! (PN-FR-006), guards evaluation eligibility with support-aware leakage
//! rules (PN-FR-007), and publishes a versioned diagnostic destination with
//! request/resolved-plan/frozen-input provenance. Mechanism-agnostic signal
//! diagnostics (PN-FR-009/010/011/012) land in `diagnostics.rs` (slice M1b);
//! this skeleton owns the request surface, planning, leakage guard, and
//! destination publication (PN-FR-001/002/003/004/006/007/035).
//!
//! Recorded-only: no acquisition, no instrument access, no config mutation.

use anyhow::{Context, Result, bail};
use pmoke_analysis_core::calibration::{
    BlockPlan, BlockPlanRequest, CalibrationRole, RoleInterval,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::utils::recorded_source::{
    ChannelBinding, FrozenInputView, GridBinding, RecordedSource, RecordedSourceKind,
    RecordedSourceRequest,
};

use super::plan::{GuardSpec, LeakageGuard, ResolvedRolePlan, resolve_role_plan};

/// Noise diagnose request schema version (workflow request v1, PN-A-004).
pub const NOISE_DIAGNOSE_REQUEST_SCHEMA_VERSION: u32 = 1;

/// Diagnostic destination schema version (diagnostic report v1, PN-A-004).
pub const NOISE_DIAGNOSTIC_SCHEMA_VERSION: u32 = 1;

/// Versioned diagnose request (deny_unknown_fields: no silent options).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnoseRequest {
    pub schema_version: u32,
    pub operation: DiagnoseOperation,
    pub source: DiagnoseSource,
    pub study: StudyClassification,
    pub reference_frequency_hz: f64,
    pub reference_phase_rad: f64,
    /// Sample interval in seconds. Explicit in the request so the plan never
    /// infers the acquisition clock from file layout (PN-FR-002/006).
    pub sample_interval_s: f64,
    pub roles: Vec<DiagnoseRoleInterval>,
    pub block_len: usize,
    pub guard: Option<GuardSpecToml>,
    pub output: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnoseOperation {
    Diagnose,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnoseSource {
    pub kind: RecordedSourceKind,
    pub path: String,
    pub channels: ChannelBinding,
    pub grid: GridBinding,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StudyClassification {
    pub classification: StudyKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StudyKind {
    Exploratory,
    Confirmatory,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnoseRoleInterval {
    pub role: DiagnoseRole,
    pub start: u64,
    pub end: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnoseRole {
    Training,
    Validation,
    Evaluation,
    FullOutputOnly,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GuardSpecToml {
    pub guard_samples: Option<u64>,
    /// Fractional-interpolation halo in samples (default 2: one sample each
    /// side of legacy endpoint interpolation, PN-FR-004/007).
    pub interpolation_halo_samples: Option<u64>,
}

/// Resolved diagnose plan written to `request.resolved.json`.
#[derive(Debug, Clone, Serialize)]
pub struct ResolvedDiagnosePlan {
    pub schema_version: u32,
    pub operation: String,
    pub study_classification: String,
    pub source_kind: String,
    pub detector_channel: u8,
    pub reference_channel: Option<u8>,
    pub witness_channel: Option<u8>,
    pub stride: usize,
    pub sample_count: u64,
    pub sample_interval_s: f64,
    pub reference_frequency_hz: f64,
    pub reference_phase_rad: f64,
    pub block_len: usize,
    pub guard: GuardSpec,
    pub role_plan: ResolvedRolePlan,
    pub leakage: LeakageGuard,
    pub frozen_inputs: FrozenInputView,
}

/// Minimal diagnostic report skeleton; M1b fills measurement sections.
/// Unknown/insufficient outcomes are first-class (PN-FR-012/035): an empty
/// measurement set still reports `unknown` with reasons, never a pass.
#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticReportSkeleton {
    pub schema_version: u32,
    pub operation: String,
    pub diagnostic_finding: String,
    pub finding_reason: String,
    pub measurements_available: bool,
    pub frozen_inputs: FrozenInputView,
    pub role_plan: ResolvedRolePlan,
}

/// Runs `pmoke noise diagnose --request FILE` into a new destination.
pub fn run_diagnose(request_path: &Path, output_override: Option<&Path>) -> Result<()> {
    let (request, request_dir) = load_request(request_path)?;
    let source_request = RecordedSourceRequest {
        kind: request.source.kind,
        path: request.source.path.clone(),
        channels: request.source.channels.clone(),
        grid: request.source.grid,
    };
    let source = RecordedSource::open(&request_dir, &source_request)?;
    let total_samples = source.sample_count() as u64;
    if total_samples == 0 {
        bail!("noise diagnose source holds no samples");
    }

    let role_intervals: Vec<RoleInterval> = request
        .roles
        .iter()
        .map(|interval| RoleInterval {
            role: calibration_role_of(interval.role),
            start: interval.start,
            end: interval.end,
        })
        .collect();
    let plan_request = BlockPlanRequest {
        intervals: role_intervals,
        block_len: request.block_len,
        total_samples,
        reference_frequency_hz: request.reference_frequency_hz,
        sample_interval_s: request.sample_interval_s,
        ..BlockPlanRequest::default()
    };
    let guard = GuardSpec {
        guard_samples: request
            .guard
            .and_then(|guard| guard.guard_samples)
            .unwrap_or(0),
        interpolation_halo_samples: request
            .guard
            .and_then(|guard| guard.interpolation_halo_samples)
            .unwrap_or(2),
    };
    if !(request.sample_interval_s.is_finite() && request.sample_interval_s > 0.0) {
        bail!("noise request needs a positive finite sample_interval_s");
    }
    let resolved = resolve_role_plan(&plan_request, guard, request.sample_interval_s)?;

    let output = match output_override {
        Some(path) => path.to_path_buf(),
        None => request_dir.join(&request.output),
    };
    ensure_new_dir(&output)?;

    let source_kind = match source.kind() {
        RecordedSourceKind::RecordedRaw => "recorded_raw",
        RecordedSourceKind::RecordedCsv => "recorded_csv",
    };
    let study = match request.study.classification {
        StudyKind::Exploratory => "exploratory",
        StudyKind::Confirmatory => "confirmatory",
    };
    let plan = ResolvedDiagnosePlan {
        schema_version: NOISE_DIAGNOSTIC_SCHEMA_VERSION,
        operation: "diagnose".to_string(),
        study_classification: study.to_string(),
        source_kind: source_kind.to_string(),
        detector_channel: source.binding().detector,
        reference_channel: source.binding().reference,
        witness_channel: source.binding().witness,
        stride: source.grid().stride,
        sample_count: total_samples,
        sample_interval_s: request.sample_interval_s,
        reference_frequency_hz: request.reference_frequency_hz,
        reference_phase_rad: request.reference_phase_rad,
        block_len: request.block_len,
        guard,
        role_plan: resolved.role_plan.clone(),
        leakage: resolved.leakage.clone(),
        frozen_inputs: source.frozen_view().clone(),
    };
    // Re-verify the live source after planning: the frozen view pins the
    // exact consumed bytes, and any drift fails before publication.
    source.verify_live()?;

    write_json(
        &output.join("request.resolved.json"),
        &plan,
        "resolved diagnose plan",
    )?;
    let request_text = std::fs::read_to_string(request_path)
        .with_context(|| format!("cannot re-read request: {}", request_path.display()))?;
    std::fs::write(output.join("request.source.toml"), request_text).with_context(|| {
        format!(
            "cannot write {}",
            output.join("request.source.toml").display()
        )
    })?;
    let inputs = serde_json::json!({
        "schema_version": NOISE_DIAGNOSTIC_SCHEMA_VERSION,
        "frozen_inputs": source.frozen_view(),
    });
    write_json(&output.join("inputs.json"), &inputs, "frozen inputs")?;

    // Skeleton report: measurements land in M1b; the finding is an honest
    // `unknown` until computed diagnostics exist (PN-FR-012).
    let report = DiagnosticReportSkeleton {
        schema_version: NOISE_DIAGNOSTIC_SCHEMA_VERSION,
        operation: "diagnose".to_string(),
        diagnostic_finding: "unknown".to_string(),
        finding_reason: "signal diagnostics not yet computed (M1b slice)".to_string(),
        measurements_available: false,
        frozen_inputs: source.frozen_view().clone(),
        role_plan: resolved.role_plan.clone(),
    };
    write_json(
        &output.join("diagnostics.json"),
        &report,
        "diagnostic skeleton",
    )?;

    crate::ui::success(format!(
        "noise diagnosis planned: {} ({} eligible evaluation blocks)",
        output.display(),
        resolved.leakage.eligible_evaluation_blocks.len(),
    ));
    Ok(())
}

fn calibration_role_of(role: DiagnoseRole) -> CalibrationRole {
    match role {
        DiagnoseRole::Training => CalibrationRole::Training,
        DiagnoseRole::Validation => CalibrationRole::TuningValidation,
        DiagnoseRole::Evaluation | DiagnoseRole::FullOutputOnly => CalibrationRole::Evaluation,
    }
}

fn load_request(request_path: &Path) -> Result<(DiagnoseRequest, PathBuf)> {
    let request_dir = request_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let text = std::fs::read_to_string(request_path)
        .with_context(|| format!("cannot read noise request: {}", request_path.display()))?;
    let request: DiagnoseRequest = toml::from_str(&text)
        .with_context(|| format!("invalid noise request: {}", request_path.display()))?;
    if request.schema_version != NOISE_DIAGNOSE_REQUEST_SCHEMA_VERSION {
        bail!(
            "unsupported noise request schema_version {} (expected {NOISE_DIAGNOSE_REQUEST_SCHEMA_VERSION})",
            request.schema_version
        );
    }
    if request.operation != DiagnoseOperation::Diagnose {
        bail!("noise diagnose request needs operation = \"diagnose\"");
    }
    if !(request.reference_frequency_hz.is_finite() && request.reference_frequency_hz > 0.0) {
        bail!("noise request needs a positive finite reference_frequency_hz");
    }
    if !request.reference_phase_rad.is_finite() {
        bail!("noise request needs a finite reference_phase_rad");
    }
    if request.block_len == 0 {
        bail!("noise request needs a positive block_len");
    }
    if request.roles.is_empty() {
        bail!("noise request lists no role intervals");
    }
    Ok((request, request_dir))
}

fn ensure_new_dir(path: &Path) -> Result<()> {
    if path.exists() {
        bail!(
            "noise output already exists (no overwrite by default): {}",
            path.display()
        );
    }
    std::fs::create_dir_all(path)
        .with_context(|| format!("cannot create noise output: {}", path.display()))?;
    Ok(())
}

fn write_json(path: &Path, value: &impl Serialize, label: &str) -> Result<()> {
    let text =
        serde_json::to_string_pretty(value).with_context(|| format!("cannot encode {label}"))?;
    std::fs::write(path, format!("{text}\n"))
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

/// Sorted block-plan summary helpers shared with M1b diagnostics.
#[allow(dead_code)]
fn blocks_by_role(plan: &BlockPlan, role: CalibrationRole) -> Vec<(u64, u64)> {
    let mut blocks: Vec<(u64, u64)> = plan
        .blocks
        .iter()
        .filter(|block| block.role == role)
        .map(|block| (block.start, block.end))
        .collect();
    blocks.sort_unstable();
    blocks
}

#[allow(dead_code)]
fn role_coverage(plan: &BlockPlan) -> BTreeMap<String, usize> {
    let mut coverage = BTreeMap::new();
    for block in &plan.blocks {
        let key = match block.role {
            CalibrationRole::Training => "training",
            CalibrationRole::TuningValidation => "validation",
            CalibrationRole::Evaluation => "evaluation",
        };
        *coverage.entry(key.to_string()).or_insert(0) += 1;
    }
    coverage
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
