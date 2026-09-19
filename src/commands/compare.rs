//! compare-lockin: run estimator legs on shared data/grid into a new directory.
//!
//! FR-036 mechanical contract: every requested method runs on the same
//! fetched data, reference-fit policy, window, and stride; only the
//! estimator differs. Candidate generations land in separate
//! subdirectories of a NEW output directory. No source run directory is
//! ever opened for writing: no mutation lock, no staging, no baseline
//! regeneration on the source experiment. Per-leg execution reuses the
//! LI-only entrypoint, so staged reruns resolve the same estimator
//! (FR-037).
//!
//! Out of scope for this slice (explicitly deferred, not silently
//! replaced): the frozen nuisance snapshot controlled lane (INTERFACES
//! 3.1), oracle R_eval sandwiches, and statistical benefit plans. Legs
//! record their provenance; scientific qualification belongs to a later
//! evaluation slice.

use crate::config::{Config, GlsCovarianceOutput, GlsNoiseMode, LockinEstimator};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Comparison request schema version.
pub const COMPARE_REQUEST_SCHEMA_VERSION: u32 = 1;

/// Versioned comparison request (deny_unknown_fields: no silent options).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareRequest {
    pub schema_version: u32,
    pub base_config: String,
    pub output: String,
    pub methods: Vec<CompareMethod>,
    pub gate_version: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompareMethod {
    pub name: String,
    pub estimator: CompareEstimator,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum CompareEstimator {
    BoxcarLegacy,
    Joint {
        noise_mode: GlsNoiseMode,
        #[serde(default)]
        covariance_output: GlsCovarianceOutput,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LegStatus {
    Complete,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct LegReport {
    pub name: String,
    pub estimator: String,
    pub status: LegStatus,
    pub code: Option<String>,
    pub message: Option<String>,
    pub grid_fingerprint: Option<String>,
    pub reference_frequency_hz: Option<f64>,
    pub artifact_dir: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CompareReport {
    pub schema_version: u32,
    pub gate_version: String,
    pub base_config: String,
    pub base_config_sha256: String,
    pub legs: Vec<LegReport>,
    pub shared_grid_equal: bool,
    pub shared_reference_equal: bool,
    pub computation_complete: bool,
}

fn safe_label(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

fn resolve_request_path(request_dir: &Path, value: &str) -> Result<PathBuf> {
    if value.is_empty() {
        bail!("comparison request path must not be empty");
    }
    let path = Path::new(value);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    if value.contains("..") {
        bail!("comparison request path must not contain '..': {value}");
    }
    Ok(request_dir.join(path))
}

/// Canonicalizes an existing path; errors on missing components.
fn canonical_existing(path: &Path, label: &str) -> Result<PathBuf> {
    path.canonicalize()
        .with_context(|| format!("comparison {label} does not exist: {}", path.display()))
}

fn is_strict_prefix(prefix: &Path, path: &Path) -> bool {
    path.starts_with(prefix) && path != prefix
}

fn ensure_new_destination(output: &Path, base_artifact_root: &Path) -> Result<PathBuf> {
    if output.exists() {
        bail!(
            "comparison output already exists (no overwrite by default): {}",
            output.display()
        );
    }
    let parent = output.parent().filter(|p| !p.as_os_str().is_empty());
    if output
        .components()
        .any(|component| component == std::path::Component::ParentDir)
    {
        bail!(
            "comparison output must not contain '..': {}",
            output.display()
        );
    }
    let anchor: PathBuf = match parent {
        Some(parent) if parent.exists() => canonical_existing(parent, "comparison output parent")?,
        Some(parent) => bail!(
            "comparison output parent does not exist: {}",
            parent.display()
        ),
        None => bail!("comparison output needs a parent directory"),
    };
    // The output itself is new, so its would-be canonical path is the
    // canonical parent joined with the final component. Equal, containing,
    // or contained relative to the canonical base root are all rejected
    // (symlinks in the parent already resolved).
    let file_name = output.file_name().ok_or_else(|| {
        anyhow::anyhow!(
            "comparison output needs a final component: {}",
            output.display()
        )
    })?;
    let synthetic = anchor.join(file_name);
    let root = canonical_existing(base_artifact_root, "base artifact root")?;
    if synthetic == root
        || is_strict_prefix(&synthetic, &root)
        || is_strict_prefix(&root, &synthetic)
    {
        bail!("comparison output must not equal, contain, or sit inside the base artifact root");
    }
    Ok(output.to_path_buf())
}

fn load_compare_request(request_path: &Path) -> Result<(CompareRequest, PathBuf)> {
    let request_dir = request_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let text = std::fs::read_to_string(request_path)
        .with_context(|| format!("cannot read comparison request: {}", request_path.display()))?;
    let request: CompareRequest = toml::from_str(&text)
        .with_context(|| format!("invalid comparison request: {}", request_path.display()))?;
    if request.schema_version != COMPARE_REQUEST_SCHEMA_VERSION {
        bail!(
            "unsupported comparison request schema_version {} (expected {COMPARE_REQUEST_SCHEMA_VERSION})",
            request.schema_version
        );
    }
    if request.methods.is_empty() {
        bail!("comparison request lists no methods");
    }
    let mut names = std::collections::HashSet::new();
    for method in &request.methods {
        if !safe_label(&method.name) || method.name.eq_ignore_ascii_case(FROZEN_SOURCE_DIR_NAME) {
            bail!(
                "comparison method name {:?} is not a safe output label",
                method.name
            );
        }
        // Use portable directory identity, including case-insensitive filesystems.
        if !names.insert(method.name.to_ascii_lowercase()) {
            bail!("duplicate comparison method name: {}", method.name);
        }
    }
    if request.gate_version.trim().is_empty() {
        bail!("comparison request needs a non-empty gate_version");
    }
    Ok((request, request_dir))
}

fn estimator_label(estimator: &CompareEstimator) -> String {
    match estimator {
        CompareEstimator::BoxcarLegacy => "boxcar_legacy".to_string(),
        CompareEstimator::Joint {
            noise_mode,
            covariance_output,
        } => format!("joint_{noise_mode:?}_{covariance_output:?}").to_lowercase(),
    }
}

fn apply_method_estimator(cfg: &mut Config, method: &CompareMethod) -> Result<()> {
    match &method.estimator {
        CompareEstimator::BoxcarLegacy => {
            cfg.lockin.estimator = LockinEstimator::BoxcarLegacy;
            Ok(())
        }
        CompareEstimator::Joint {
            noise_mode,
            covariance_output,
        } => {
            let base = match &cfg.lockin.estimator {
                LockinEstimator::JointHarmonicGls(gls) => gls.clone(),
                LockinEstimator::BoxcarLegacy => bail!(
                    "comparison method {} needs a joint estimator: the base config is boxcar_legacy \
                     and carries no calibration bindings",
                    method.name
                ),
            };
            if matches!(
                base.calibration_source,
                crate::config::GlsCalibrationSource::Prepulse
            ) {
                // Pre-pulse legs derive every channel model from the recorded
                // background; file bindings are forbidden alongside, not required.
            } else if base.calibrations.is_empty() {
                bail!(
                    "comparison method {} needs calibration bindings, none configured",
                    method.name
                );
            }
            // Methods share the base signal model and differ only in the
            // estimator/noise fields the method explicitly requests: the
            // fit design (nuisance constraints, Nyquist/rank behavior,
            // covariance) is cloned, never rebuilt with a default list.
            cfg.lockin.estimator =
                LockinEstimator::JointHarmonicGls(crate::config::JointHarmonicGlsConfig {
                    fit_harmonics: base.fit_harmonics,
                    output_harmonics: base.output_harmonics,
                    envelope_degree: base.envelope_degree,
                    noise_mode: *noise_mode,
                    covariance_output: *covariance_output,
                    failure_policy: base.failure_policy,
                    calibration_source: base.calibration_source,
                    calibrations: base.calibrations,
                });
            Ok(())
        }
    }
}

fn extract_code(message: &str) -> Option<String> {
    message.split("code=").nth(1).and_then(|tail| {
        let end = tail
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != '-')
            .unwrap_or(tail.len());
        let code = &tail[..end];
        (!code.is_empty()).then(|| code.to_string())
    })
}

/// Grid fingerprint from a leg analysis manifest: support ranges, xy row
/// count, and fitted reference frequency. Two legs share the grid exactly
/// when these agree.
fn leg_grid_fingerprint(manifest: &toml::Value) -> Result<(String, f64)> {
    let lockin = manifest
        .get("lockin")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| anyhow::anyhow!("leg manifest has no lockin provenance"))?;
    let get = |key: &str| {
        lockin
            .get(key)
            .and_then(toml::Value::as_integer)
            .ok_or_else(|| anyhow::anyhow!("leg manifest lockin misses {key}"))
    };
    let f_ref = manifest
        .get("reference")
        .and_then(|reference| reference.get("frequency_hz"))
        .and_then(toml::Value::as_float)
        .ok_or_else(|| anyhow::anyhow!("leg manifest has no reference frequency_hz"))?;
    let rows = manifest
        .get("artifacts")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("leg manifest has no artifacts"))?
        .iter()
        .find(|artifact| artifact.get("kind").and_then(toml::Value::as_str) == Some("lockin_xy"))
        .and_then(|artifact| artifact.get("rows"))
        .and_then(toml::Value::as_integer)
        .ok_or_else(|| anyhow::anyhow!("leg manifest has no lockin_xy rows"))?;
    let canonical = format!(
        "{}:{}:{}:{}:{rows}:{f_ref:.17e}",
        get("base_index_start")?,
        get("base_index_end")?,
        get("output_index_start")?,
        get("output_index_end")?,
    );
    Ok((
        crate::utils::checksum::sha256_hex(canonical.as_bytes()),
        f_ref,
    ))
}

/// Staging directory name (under the comparison destination) holding the
/// frozen source tree every leg is seeded from.
const FROZEN_SOURCE_DIR_NAME: &str = "_frozen_source";

/// Canonical source-input listing as (key, live path) pairs: every file
/// under the acquisition tree keyed by relative path, plus the resolved
/// waveform CSV keyed at its canonical position when a legacy layout keeps
/// it outside that tree. Keys are canonical positions, so a frozen staging
/// copy hashes exactly like the live source it was copied from.
fn source_entries(base_cfg: &Config) -> Result<Vec<(String, PathBuf)>> {
    let mut entries = Vec::new();
    let acquisition = base_cfg.paths().acquisition_dir();
    if acquisition.is_dir() {
        collect_tree_entries(&acquisition, &acquisition, &mut entries)?;
    }
    let csv = base_cfg.resolver().waveform_csv();
    if csv.is_file() && !csv.starts_with(&acquisition) {
        entries.push(("waveforms/waveform.csv".to_string(), csv));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(entries)
}

/// Collects `(relative key, path)` for every file under a tree root.
fn collect_tree_entries(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) -> Result<()> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("cannot list {}", dir.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<_>>()
        .with_context(|| format!("cannot list {}", dir.display()))?;
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect_tree_entries(root, &path, out)?;
        } else if path.is_file() {
            let key = path
                .strip_prefix(root)
                .context("failed to relativize source input")?
                .to_string_lossy()
                .replace('\\', "/");
            out.push((key, path));
        }
    }
    Ok(())
}

/// Streaming canonical hash over sorted (key, length, bytes) entries.
/// Every consumed input byte shapes the print: a waveform mutation with an
/// unchanged manifest still changes it.
pub(crate) fn hash_source_entries(entries: &[(String, PathBuf)]) -> Result<String> {
    let mut ordered: Vec<&(String, PathBuf)> = entries.iter().collect();
    ordered.sort_by(|left, right| left.0.cmp(&right.0));
    let mut canonical = Vec::new();
    for (key, path) in ordered {
        let digest = crate::utils::checksum::file_sha256(path)
            .with_context(|| format!("cannot hash {}", path.display()))?;
        canonical.extend_from_slice(key.as_bytes());
        canonical.push(0u8);
        canonical.extend_from_slice(digest.as_bytes());
        canonical.push(0u8);
    }
    Ok(crate::utils::checksum::sha256_hex(&canonical))
}

/// Copies live source entries into staging canonical positions.
fn freeze_source_entries(entries: &[(String, PathBuf)], staging_acquisition: &Path) -> Result<()> {
    for (key, path) in entries {
        let target = staging_acquisition.join(key);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("cannot stage frozen source: {}", parent.display()))?;
        }
        std::fs::copy(path, &target).with_context(|| {
            format!(
                "cannot freeze source input: {} -> {}",
                path.display(),
                target.display()
            )
        })?;
    }
    Ok(())
}

/// Verifies a directory tree still matches its fingerprint (code
/// `source_changed` on mismatch).
pub(crate) fn verify_source_tree(dir: &Path, expected: &str) -> Result<()> {
    let actual = fingerprint_tree(dir)?;
    if actual != expected {
        anyhow::bail!("comparison source changed during execution (code=source_changed)");
    }
    Ok(())
}

/// Fingerprint of a directory tree in canonical source form (stable for
/// missing or empty trees).
pub(crate) fn fingerprint_tree(dir: &Path) -> Result<String> {
    let mut entries = Vec::new();
    if dir.is_dir() {
        collect_tree_entries(dir, dir, &mut entries)?;
    }
    hash_source_entries(&entries)
}

/// Pure summary over leg reports: shared grid/reference equality and
/// overall completion (unit-testable without I/O).
pub(crate) fn summarize_legs(legs: &[LegReport]) -> (bool, bool, bool) {
    let grids: std::collections::HashSet<&str> = legs
        .iter()
        .filter_map(|leg| leg.grid_fingerprint.as_deref())
        .collect();
    let references: Vec<f64> = legs
        .iter()
        .filter_map(|leg| leg.reference_frequency_hz)
        .collect();
    let grid_equal = grids.len() <= 1 && legs.iter().any(|leg| leg.grid_fingerprint.is_some());
    let reference_equal = references.windows(2).all(|pair| pair[0] == pair[1]);
    let complete =
        legs.iter().all(|leg| leg.status == LegStatus::Complete) && grid_equal && reference_equal;
    (grid_equal, reference_equal, complete)
}

/// Runs estimator legs for one base config and already-loaded data.
/// Testable without fetch storage; the CLI wrapper adds request I/O.
/// `frozen_acquisition` is the destination-owned frozen source tree: legs
/// are seeded from these exact bytes, never re-read from the live source.
pub(crate) fn run_legs(
    base_cfg: &Config,
    data: &crate::utils::waveform::WaveformData,
    methods: &[CompareMethod],
    output: &Path,
    frozen_acquisition: &Path,
) -> Result<Vec<LegReport>> {
    crate::commands::analyze::validate_waveform_data(data)?;
    let frozen = fingerprint_tree(frozen_acquisition)?;
    let mut legs = Vec::with_capacity(methods.len());
    for method in methods {
        let leg_dir = output.join(&method.name);
        let report = run_single_leg(
            base_cfg,
            data,
            method,
            &leg_dir,
            frozen_acquisition,
            &frozen,
        );
        legs.push(report);
    }
    Ok(legs)
}

fn run_single_leg(
    base_cfg: &Config,
    data: &crate::utils::waveform::WaveformData,
    method: &CompareMethod,
    leg_dir: &Path,
    frozen_acquisition: &Path,
    frozen_fingerprint: &str,
) -> LegReport {
    let failed = |message: String| LegReport {
        name: method.name.clone(),
        estimator: estimator_label(&method.estimator),
        status: LegStatus::Failed,
        code: extract_code(&message),
        message: Some(message),
        grid_fingerprint: None,
        reference_frequency_hz: None,
        artifact_dir: leg_dir.display().to_string(),
    };
    let mut leg_cfg = base_cfg.clone();
    if let Err(error) = apply_method_estimator(&mut leg_cfg, method) {
        return failed(format!("{error:#}"));
    }
    leg_cfg.set_artifact_root(leg_dir.to_path_buf());
    // Freeze the frozen input into the leg: copy the destination-owned
    // frozen tree (never the live source) so LI validation sees the exact
    // bytes estimation parsed, and verify the copy matches. Estimation
    // still uses the single shared frozen read.
    if let Err(error) = seed_leg_acquisition(frozen_acquisition, frozen_fingerprint, &leg_cfg) {
        return failed(format!("{error:#}"));
    }
    // Reuse the LI-only entrypoint per leg (FR-037): same staging,
    // locking, validation, and publication as `pmoke li`, on the shared
    // frozen read.
    if let Err(error) = run_leg_li(&leg_cfg, data) {
        return failed(format!("{error:#}"));
    }
    let manifest_path = leg_cfg.paths().analysis_manifest();
    let manifest_text = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => text,
        Err(error) => {
            return failed(format!(
                "leg {} finished but its manifest is unreadable: {error:#}",
                method.name
            ));
        }
    };
    let manifest: toml::Value = match toml::from_str(&manifest_text) {
        Ok(value) => value,
        Err(error) => return failed(format!("leg {} manifest invalid: {error}", method.name)),
    };
    match leg_grid_fingerprint(&manifest) {
        Ok((grid_fingerprint, f_ref)) => LegReport {
            name: method.name.clone(),
            estimator: estimator_label(&method.estimator),
            status: LegStatus::Complete,
            code: None,
            message: None,
            grid_fingerprint: Some(grid_fingerprint),
            reference_frequency_hz: Some(f_ref),
            artifact_dir: leg_dir.display().to_string(),
        },
        Err(error) => failed(format!(
            "leg {} manifest misses grid evidence: {error:#}",
            method.name
        )),
    }
}

/// Copies the frozen source tree into a leg directory (frozen source, new
/// destination) and verifies the copy matches the frozen fingerprint.
/// Missing frozen input is not an error here: LI validation reports the
/// absent input explicitly per leg.
fn seed_leg_acquisition(
    frozen_acquisition: &Path,
    frozen_fingerprint: &str,
    leg_cfg: &Config,
) -> Result<()> {
    let destination = leg_cfg.paths().acquisition_dir();
    if frozen_acquisition.is_dir() {
        copy_dir_recursive(frozen_acquisition, &destination).with_context(|| {
            format!(
                "cannot freeze shared input into leg: {} -> {}",
                frozen_acquisition.display(),
                destination.display()
            )
        })?;
        verify_source_tree(&destination, frozen_fingerprint).with_context(|| {
            format!(
                "leg input copy diverged from the frozen source: {}",
                destination.display()
            )
        })?;
    }
    Ok(())
}

fn copy_dir_recursive(source: &Path, destination: &Path) -> Result<()> {
    std::fs::create_dir_all(destination)
        .with_context(|| format!("cannot create {}", destination.display()))?;
    for entry in
        std::fs::read_dir(source).with_context(|| format!("cannot list {}", source.display()))?
    {
        let entry = entry?;
        let target = destination.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)
                .with_context(|| format!("cannot copy {}", entry.path().display()))?;
        }
    }
    Ok(())
}

fn run_leg_li(cfg: &Config, data: &crate::utils::waveform::WaveformData) -> Result<()> {
    crate::commands::run_dir::ensure_run_directory(&cfg.paths().run_dir)?;
    let _lock = crate::commands::run_dir::RunMutationLock::acquire(&cfg.paths().run_dir, "li")?;
    crate::config::validate_for_target(cfg, crate::config::ValidationTarget::Li)?;
    crate::commands::run_dir::prepare_analysis_run(cfg)?;
    crate::commands::run_dir::write_run_state(cfg, "analyzing", "li", None)?;
    let result = crate::commands::li::li_inner_with_data(cfg, data);
    match &result {
        Ok(()) => crate::commands::run_dir::write_run_state(cfg, "analyzing", "li_complete", None)?,
        Err(error) => crate::commands::run_dir::write_run_state(cfg, "failed", "li", Some(error))?,
    }
    result
}

/// Digest of the exact parsed config bytes (kept as source text by the
/// loader): reporting never re-reads the config file.
pub(crate) fn config_source_digest(cfg: &Config) -> Result<String> {
    cfg.source_text
        .as_ref()
        .map(|text| crate::utils::checksum::sha256_hex(text.as_bytes()))
        .ok_or_else(|| anyhow::anyhow!("comparison base config lost its parsed source text"))
}

/// CLI entrypoint: `pmoke compare-lockin --request REQ.toml [--output DIR]`.
/// Prints only the declared JSON report to stdout; progress goes to stderr.
pub fn run_compare(request_path: &Path, output_override: Option<&Path>) -> Result<()> {
    let (request, request_dir) = load_compare_request(request_path)?;
    let base_path = resolve_request_path(&request_dir, &request.base_config)?;
    let load = crate::config::load_from_path(&base_path);
    let (base_cfg, _) = load.into_ready().with_context(|| {
        format!(
            "comparison base config is not runnable: {}",
            base_path.display()
        )
    })?;
    // The digest covers the exact bytes parsed above (kept as source
    // text by the loader): the config is never re-read for reporting.
    let base_config_sha256 = config_source_digest(&base_cfg)?;
    let output = match output_override {
        Some(path) => path.to_path_buf(),
        None => resolve_request_path(&request_dir, &request.output)?,
    };
    ensure_new_destination(&output, &base_cfg.paths().run_dir)?;
    std::fs::create_dir(&output)
        .with_context(|| format!("cannot create comparison output: {}", output.display()))?;
    // Destination-only lock before any source input is touched: the source
    // experiment is never locked, staged, or written.
    let _lock = crate::commands::run_dir::RunMutationLock::acquire(&output, "compare-lockin")
        .context("cannot lock the comparison destination")?;
    // Freeze first: copy the live source entries into destination-owned
    // staging, then fingerprint the staging copy. Everything downstream —
    // waveform parsing, leg seeding, leg validation, estimation — reads
    // these exact bytes; the live source is only hashed afterwards.
    let staging_acquisition = output.join(FROZEN_SOURCE_DIR_NAME).join("acquisition");
    std::fs::create_dir_all(&staging_acquisition).with_context(|| {
        format!(
            "cannot stage frozen source: {}",
            staging_acquisition.display()
        )
    })?;
    let live_entries = source_entries(&base_cfg)?;
    let live_before = hash_source_entries(&live_entries)?;
    freeze_source_entries(&live_entries, &staging_acquisition)?;
    let frozen = fingerprint_tree(&staging_acquisition)?;
    if frozen != live_before || hash_source_entries(&source_entries(&base_cfg)?)? != live_before {
        anyhow::bail!(
            "comparison source changed while freezing (code=source_changed); refusing to mix generations"
        );
    }
    // Parse from the frozen copy through a config rooted at staging, so
    // the shared arrays and the leg inputs describe identical bytes.
    let mut frozen_cfg = base_cfg.clone();
    frozen_cfg.set_artifact_root(
        staging_acquisition
            .parent()
            .context("frozen staging has no parent")?
            .to_path_buf(),
    );
    let data = crate::utils::waveform::read_all_fetched_waveforms(&frozen_cfg)?;
    verify_source_tree(&staging_acquisition, &frozen).with_context(|| {
        format!(
            "frozen source changed during reading: {}",
            staging_acquisition.display()
        )
    })?;
    let legs = run_legs(
        &base_cfg,
        &data,
        &request.methods,
        &output,
        &staging_acquisition,
    )?;
    // Nonmutating source consistency: fail before claiming a complete
    // publication rather than mixing generations (code `source_changed`).
    // The report below still publishes the truthful partial status.
    let source_stable = hash_source_entries(&source_entries(&base_cfg)?)? == live_before;
    let (shared_grid_equal, shared_reference_equal, legs_complete) = summarize_legs(&legs);
    let completed = legs
        .iter()
        .filter(|leg| leg.status == LegStatus::Complete)
        .count();
    let computation_complete = legs_complete && source_stable;
    let report = CompareReport {
        schema_version: COMPARE_REQUEST_SCHEMA_VERSION,
        gate_version: request.gate_version.clone(),
        base_config: base_path.display().to_string(),
        base_config_sha256,
        legs,
        shared_grid_equal,
        shared_reference_equal,
        computation_complete,
    };
    let report_path = output.join("compare-report.json");
    let text = serde_json::to_string_pretty(&report).context("cannot encode compare report")?;
    std::fs::write(&report_path, format!("{text}\n"))
        .with_context(|| format!("cannot write {}", report_path.display()))?;
    // JSON stdout carries only the declared report; progress stays on stderr.
    println!("{text}");
    if computation_complete {
        Ok(())
    } else if !source_stable {
        bail!(
            "comparison source changed during execution (code=source_changed); refusing to mix \
             generations ({completed} of {} legs complete; report: {})",
            report.legs.len(),
            report_path.display()
        )
    } else {
        bail!(
            "comparison incomplete: {completed} of {} legs complete, grid equal: \
             {shared_grid_equal}, reference equal: {shared_reference_equal} \
             (report: {})",
            report.legs.len(),
            report_path.display()
        )
    }
}

#[cfg(test)]
#[path = "compare_tests.rs"]
mod tests;
