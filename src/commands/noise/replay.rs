//! Retained-generation replay (PN-M3, Issue #246; PN-FR-023/025,
//! PN-AT-024).
//!
//! `pmoke noise replay --destination DIR` verifies a committed compare
//! destination and replays its carried phase -> MOKE -> NPY outputs from the
//! retained artifacts alone. The destination can be moved anywhere and the
//! original source or model files can be gone: the retained digests and
//! per-row records are authoritative (PN-AT-024).
//!
//! Verification runs in two separate steps (PN-FR-023): digest integrity
//! (manifest closure over every staged file) and semantic validation
//! (artifact schema/binding, schedule references, per-row model
//! attribution, row-count agreement). A failed step is named, never
//! repaired.
//!
//! Serialization policy is not independence (PN-FR-025): the MOKE
//! conditional variance is produced when the retained covariance carries
//! the required (x1, x2) marginal (full policy); `none`/`diagonal`
//! serializations report `unavailable`, never a reconstructed zero.

use anyhow::{Context, Result, bail};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pmoke_analysis_core::moke_uncertainty::{moke_standard_angle, moke_standard_angle_variance};

use super::compare::{StagingManifest, verify_manifest_closure};
use crate::utils::checksum::sha256_hex;

/// Replay artifact schema version (PN-A-004 reserved bank/schedule v1
/// family; the replay files are new additions).
const REPLAY_SCHEMA_VERSION: u32 = 1;

/// One retained bank row parsed for replay.
struct ReplayRow {
    center: u64,
    time_s: f64,
    xy: Vec<f64>,
    covariance: Option<Vec<f64>>,
}

/// One replayable leg: complete bank legs carry XY and attribution; other
/// legs are recorded as explicit unavailable entries.
struct ReplayLeg {
    name: String,
    mode: Option<String>,
    rows: Vec<ReplayRow>,
}

/// Runs `pmoke noise replay`. `output` defaults to `<destination>/replay`;
/// `verify_only` stops after verification and writes nothing.
pub fn run_replay(destination: &Path, output: Option<&Path>, verify_only: bool) -> Result<()> {
    if !destination.is_dir() {
        bail!(
            "noise replay destination must be a committed compare directory: {}",
            destination.display()
        );
    }
    let manifest_path = destination.join("staging-manifest.json");
    let text = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("cannot read {}", manifest_path.display()))?;
    let manifest: StagingManifest = serde_json::from_str(&text)
        .with_context(|| format!("invalid staging manifest: {}", manifest_path.display()))?;
    if manifest.schema_version > 2 {
        bail!(
            "noise replay does not support manifest schema_version {}",
            manifest.schema_version
        );
    }
    // Step 1: digest integrity and retained-file closure.
    verify_manifest_closure(destination, &manifest)?;

    // Step 2: semantic schema validation.
    let context = read_json(&destination.join("context.json"))?;
    let sample_interval_s = context["sample_interval_s"]
        .as_f64()
        .ok_or_else(|| anyhow::anyhow!("frozen context lacks sample_interval_s"))?;
    let reference_frequency_hz = context["reference_frequency_hz"]
        .as_f64()
        .ok_or_else(|| anyhow::anyhow!("frozen context lacks reference_frequency_hz"))?;
    let rotation_rad = context["rotation_rad"].as_f64().unwrap_or(0.0);
    let phim = context["modulation_depth"]
        .as_f64()
        .ok_or_else(|| anyhow::anyhow!("frozen context lacks modulation_depth"))?;
    let comparison = read_json(&destination.join("comparison.json"))?;
    if let Some(bank) = comparison.get("bank") {
        if bank["schedule_sha256"].as_str() != manifest.schedule_sha256.as_deref() {
            bail!(
                "comparison.json schedule digest disagrees with the staging manifest (code=closure_mismatch)"
            );
        }
        if bank["bank_sha256"].as_str() != manifest.bank_sha256.as_deref() {
            bail!(
                "comparison.json bank digest disagrees with the staging manifest (code=closure_mismatch)"
            );
        }
    }
    let schedule = read_json(&destination.join("schedule.json"))?;
    let models = read_json(&destination.join("models.json"))?;
    let bank = if destination.join("model-bank.json").is_file() {
        Some(read_json(&destination.join("model-bank.json"))?)
    } else {
        None
    };

    // Every referenced model artifact must exist, match its recorded digest,
    // and parse as a calibration artifact v1 bound to this destination's
    // grid.
    let mut known_model_ids: BTreeMap<String, String> = BTreeMap::new();
    for model in models.as_array().map(Vec::as_slice).unwrap_or(&[]) {
        validate_model_artifact(
            destination,
            model,
            sample_interval_s,
            reference_frequency_hz,
            &mut known_model_ids,
        )?;
    }
    let mut schedule_sha: Option<String> = None;
    let mut bank_models_by_mode: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    if let Some(bank) = bank.as_ref() {
        schedule_sha = bank["schedule_sha256"].as_str().map(str::to_string);
        for model in bank["models"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            validate_model_artifact(
                destination,
                model,
                sample_interval_s,
                reference_frequency_hz,
                &mut known_model_ids,
            )?;
            let mode = model["mode"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("bank model entry lacks mode"))?;
            let model_id = model["model_id"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("bank model entry lacks model_id"))?;
            bank_models_by_mode
                .entry(mode.to_string())
                .or_default()
                .insert(model_id.to_string(), model_id.to_string());
        }
        // Bank hash closure: the stored bank hash covers the regime
        // definitions and distinct model identities.
        let expected_bank_sha = bank["bank_sha256"].as_str().unwrap_or_default();
        if expected_bank_sha.len() != 64 {
            bail!("model-bank.json lacks a valid bank_sha256");
        }
    }
    let schedule_rows: Vec<Value> = schedule["bank"]["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let schedule_by_center: BTreeMap<u64, (usize, bool)> = schedule_rows
        .iter()
        .map(|row| {
            let center = row["center"].as_u64().unwrap_or(u64::MAX);
            (
                center,
                (
                    row["regime_index"].as_u64().unwrap_or(u64::MAX) as usize,
                    row["boundary"].as_bool().unwrap_or(false),
                ),
            )
        })
        .collect();

    // Step 3: per-leg semantic checks and row-mapping agreement.
    let legs_dir = destination.join("legs");
    let mut replay_legs: Vec<ReplayLeg> = Vec::new();
    let mut unavailable_legs: Vec<Value> = Vec::new();
    let mut entries: Vec<std::fs::DirEntry> = std::fs::read_dir(&legs_dir)
        .with_context(|| format!("cannot list {}", legs_dir.display()))?
        .collect::<std::io::Result<_>>()
        .with_context(|| format!("cannot list {}", legs_dir.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        let leg = read_json(&path)?;
        let name = leg["name"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("leg file lacks name: {}", path.display()))?
            .to_string();
        let mode = leg["mode"].as_str().map(str::to_string);
        match leg["rows"].as_array() {
            Some(rows) => {
                let mut parsed = Vec::with_capacity(rows.len());
                for row in rows {
                    let center = row["center"]
                        .as_u64()
                        .ok_or_else(|| anyhow::anyhow!("leg {name} row lacks center"))?;
                    let (schedule_regime, schedule_boundary) = schedule_by_center
                        .get(&center)
                        .copied()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "leg {name} row at center {center} is absent from the frozen schedule (code=row_mapping_mismatch)"
                            )
                        })?;
                    if row["regime_index"].as_u64().unwrap_or(u64::MAX) as usize != schedule_regime
                        || row["boundary"].as_bool().unwrap_or(false) != schedule_boundary
                    {
                        bail!(
                            "leg {name} row at center {center} disagrees with the frozen schedule (code=row_mapping_mismatch)"
                        );
                    }
                    let model_id = row["model_id"].as_str().unwrap_or_default().to_string();
                    if let Some(mode) = mode.as_ref() {
                        let allowed = bank_models_by_mode.get(mode);
                        let known = allowed.is_some_and(|set| set.contains_key(&model_id))
                            || known_model_ids.contains_key(&model_id);
                        if !known {
                            bail!(
                                "leg {name} references unknown model '{model_id}' (code=unknown_model_reference)"
                            );
                        }
                    }
                    let xy: Vec<f64> = row["xy"]
                        .as_array()
                        .ok_or_else(|| anyhow::anyhow!("leg {name} row lacks xy"))?
                        .iter()
                        .map(|value| value.as_f64().unwrap_or(f64::NAN))
                        .collect();
                    if xy.len() != 12 || xy.iter().any(|value| !value.is_finite()) {
                        bail!("leg {name} row at center {center} holds an invalid XY vector");
                    }
                    let covariance: Option<Vec<f64>> = row["covariance"].as_array().map(|values| {
                        values
                            .iter()
                            .map(|value| value.as_f64().unwrap_or(f64::NAN))
                            .collect()
                    });
                    if let Some(covariance) = covariance.as_ref()
                        && (covariance.len() != 78 && covariance.len() != 12)
                    {
                        bail!(
                            "leg {name} row at center {center} holds {} covariance entries; expected 12 or 78",
                            covariance.len()
                        );
                    }
                    parsed.push(ReplayRow {
                        center,
                        time_s: row["time_s"].as_f64().unwrap_or(f64::NAN),
                        xy,
                        covariance,
                    });
                }
                if !schedule_rows.is_empty() && parsed.len() != schedule_rows.len() {
                    bail!(
                        "leg {name} retains {} rows but the frozen schedule holds {} centers (code=row_mapping_mismatch)",
                        parsed.len(),
                        schedule_rows.len()
                    );
                }
                replay_legs.push(ReplayLeg {
                    name,
                    mode,
                    rows: parsed,
                });
            }
            None => {
                unavailable_legs.push(serde_json::json!({
                    "leg": name,
                    "mode": mode,
                    "reason": "no retained per-row bank records (boxcar or pre-bank generation)",
                }));
            }
        }
    }
    if replay_legs.is_empty() && unavailable_legs.is_empty() {
        bail!(
            "noise replay found no leg records in {}",
            legs_dir.display()
        );
    }

    if verify_only {
        crate::ui::success(format!(
            "noise replay verification passed: {} leg(s) with retained rows, {} leg(s) without (digest closure + semantic schemas)",
            replay_legs.len(),
            unavailable_legs.len()
        ));
        return Ok(());
    }

    // Step 4: replay phase -> MOKE -> NPY into the replay destination.
    let out = output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| destination.join("replay"));
    if out.exists() {
        bail!(
            "noise replay output already exists (no overwrite): {}",
            out.display()
        );
    }
    std::fs::create_dir_all(&out)
        .with_context(|| format!("cannot create replay output: {}", out.display()))?;
    let mut leg_manifests: Vec<Value> = Vec::new();
    for leg in &replay_legs {
        let phase_dir = out.join("phase");
        let moke_dir = out.join("moke");
        let npy_dir = out.join("npy");
        for dir in [&phase_dir, &moke_dir, &npy_dir] {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        // Phase stage: the retained per-row quadratures, unchanged.
        let phase_path = phase_dir.join(format!("{}.csv", leg.name));
        write_phase_csv(&phase_path, leg)?;
        // MOKE stage: rotate by the frozen shared rotation, evaluate the
        // standard angle formula, and propagate the retained (x1, x2)
        // covariance marginal when the serialization carries it.
        let (moke_path, variance_path) = replay_moke(&moke_dir, &npy_dir, leg, rotation_rad, phim)?;
        leg_manifests.push(serde_json::json!({
            "leg": leg.name,
            "mode": leg.mode,
            "rows": leg.rows.len(),
            "phase_csv": relative(&out, &phase_path),
            "moke_csv": relative(&out, &moke_path),
            "moke_npy": variance_path
                .as_ref()
                .map(|(_, npy)| relative(&out, npy)),
            "covariance_policy": leg
                .rows
                .iter()
                .find_map(|row| row.covariance.as_ref().map(|values| values.len()))
                .map(|len| if len == 78 { "full" } else { "diagonal" })
                .unwrap_or("none"),
        }));
    }
    let replay_manifest = serde_json::json!({
        "schema_version": REPLAY_SCHEMA_VERSION,
        "source_generation": {
            "manifest_schema_version": manifest.schema_version,
            "request_sha256": manifest.request_sha256,
            "context_sha256": manifest.context_sha256,
            "bank_sha256": manifest.bank_sha256,
            "schedule_sha256": manifest.schedule_sha256,
        },
        "rotation_rad": rotation_rad,
        "phim_source": "frozen context modulation_depth",
        "legs": leg_manifests,
        "legs_without_retained_rows": unavailable_legs,
        "schedule_sha256": schedule_sha,
    });
    let manifest_out = out.join("replay-manifest.json");
    let text =
        serde_json::to_string_pretty(&replay_manifest).context("cannot encode replay manifest")?;
    std::fs::write(&manifest_out, format!("{text}\n"))
        .with_context(|| format!("cannot write {}", manifest_out.display()))?;
    crate::ui::success(format!(
        "noise replay committed: {} ({} leg(s), phase -> MOKE -> NPY; verification passed)",
        out.display(),
        replay_legs.len()
    ));
    Ok(())
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

fn read_json(path: &Path) -> Result<Value> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("invalid JSON: {}", path.display()))
}

/// Digest + semantic validation of one referenced calibration artifact:
/// exact bytes match the recorded digest, the artifact parses as schema v1,
/// and its binding agrees with the frozen grid.
fn validate_model_artifact(
    destination: &Path,
    model: &Value,
    sample_interval_s: f64,
    reference_frequency_hz: f64,
    known_model_ids: &mut BTreeMap<String, String>,
) -> Result<()> {
    let artifact_path = match model["artifact_path"].as_str() {
        Some(path) if !path.is_empty() => path.to_string(),
        _ => {
            bail!("referenced model entry lacks an artifact_path");
        }
    };
    let digest = model["sha256"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("model {artifact_path} lacks sha256"))?;
    let absolute = destination.join(&artifact_path);
    let bytes = std::fs::read(&absolute)
        .with_context(|| format!("cannot read retained model {}", absolute.display()))?;
    let actual = sha256_hex(&bytes);
    if actual != digest {
        bail!(
            "retained model {artifact_path} digest mismatch (code=model_digest_mismatch): {actual} != {digest}"
        );
    }
    let artifact: Value = serde_json::from_slice(&bytes)
        .with_context(|| format!("retained model {artifact_path} is not valid JSON"))?;
    if artifact["schema_version"].as_u64() != Some(1) {
        bail!("retained model {artifact_path} is not a calibration artifact schema v1");
    }
    if let Some(model_id) = artifact["model_id"].as_str() {
        known_model_ids.insert(model_id.to_string(), digest.to_string());
    }
    let binding = artifact
        .get("binding")
        .ok_or_else(|| anyhow::anyhow!("retained model {artifact_path} lacks a binding section"))?;
    let artifact_dt = binding["sample_interval_s"].as_f64().unwrap_or(f64::NAN);
    if !artifact_dt.is_finite()
        || (artifact_dt - sample_interval_s).abs() > 1.0e-9 * sample_interval_s
    {
        bail!(
            "retained model {artifact_path} binds dt {artifact_dt} but the frozen context declares {sample_interval_s} (code=model_binding_mismatch)"
        );
    }
    let artifact_f = binding["reference_frequency_hz"]
        .as_f64()
        .unwrap_or(f64::NAN);
    if !artifact_f.is_finite()
        || (artifact_f - reference_frequency_hz).abs() > 1.0e-9 * reference_frequency_hz
    {
        bail!(
            "retained model {artifact_path} binds f_ref {artifact_f} but the frozen context declares {reference_frequency_hz} (code=model_binding_mismatch)"
        );
    }
    Ok(())
}

/// Writes the phase-stage CSV: the retained per-row quadratures in frozen
/// schedule order (`time_s` plus `x1,y1,...,x6,y6`).
fn write_phase_csv(path: &Path, leg: &ReplayLeg) -> Result<()> {
    let mut writer = csv::WriterBuilder::new()
        .has_headers(true)
        .from_path(path)?;
    let mut header = vec!["time_s".to_string()];
    for name in [
        "x1", "y1", "x2", "y2", "x3", "y3", "x4", "y4", "x5", "y5", "x6", "y6",
    ] {
        header.push(name.to_string());
    }
    writer
        .write_record(&header)
        .context("cannot write phase header")?;
    for row in &leg.rows {
        let mut record = Vec::with_capacity(13);
        record.push(row.time_s.to_string());
        record.extend(row.xy.iter().map(ToString::to_string));
        writer
            .write_record(&record)
            .with_context(|| format!("cannot write phase row at center {}", row.center))?;
    }
    writer.flush().context("cannot flush phase replay")?;
    Ok(())
}

/// MOKE stage: rotate each harmonic pair by the frozen shared rotation,
/// evaluate the standard angle, and derive the conditional variance from
/// the retained (x1, x2) marginal when the serialization carries it
/// (PN-FR-024/025). Returns `(moke_csv, Option<(variance_csv, npy)>)`.
fn replay_moke(
    moke_dir: &Path,
    npy_dir: &Path,
    leg: &ReplayLeg,
    rotation_rad: f64,
    phim: f64,
) -> Result<(PathBuf, Option<(PathBuf, PathBuf)>)> {
    let (sin_r, cos_r) = rotation_rad.sin_cos();
    let rotate = |x: f64, y: f64| (x * cos_r + y * sin_r, -x * sin_r + y * cos_r);
    let mut times = Vec::with_capacity(leg.rows.len());
    let mut angles = Vec::with_capacity(leg.rows.len());
    let mut variances: Vec<Option<f64>> = Vec::with_capacity(leg.rows.len());
    let mut statuses: Vec<&'static str> = Vec::with_capacity(leg.rows.len());
    for row in &leg.rows {
        let (x1, _) = rotate(row.xy[0], row.xy[1]);
        let (x2, _) = rotate(row.xy[2], row.xy[3]);
        let angle = moke_standard_angle(x1, x2, phim).map_err(|error| {
            anyhow::anyhow!("MOKE angle failed at center {}: {error}", row.center)
        })?;
        let (variance, status) = match row.covariance.as_deref() {
            Some(entries) if entries.len() == 78 => {
                // Packed upper triangle, lexicographic (row, column) with
                // row <= column over the [X1,Y1,X2,Y2,...] layout:
                // (0,0)=0, (0,2)=2, (1,1)=12, (2,2)=23. The MOKE inputs are
                // the rotated X1 (row 0) and X2 (row 2) quadratures.
                let v11 = entries[0];
                let v12 = entries[2];
                let v22 = entries[23];
                let variance =
                    moke_standard_angle_variance(x1, x2, phim, v11, v12, v22).map_err(|error| {
                        anyhow::anyhow!("MOKE conditional variance failed: {error}")
                    })?;
                (Some(variance), "conditional_full_marginal")
            }
            Some(_) => (None, "unavailable_diagonal_serialization_omits_cross_terms"),
            None => (None, "unavailable_covariance_output_none"),
        };
        times.push(row.time_s);
        angles.push(angle);
        variances.push(variance);
        statuses.push(status);
    }
    let moke_path = moke_dir.join(format!("{}.csv", leg.name));
    let variance_available = variances.iter().all(Option::is_some);
    let mut writer = csv::WriterBuilder::new()
        .has_headers(true)
        .from_path(&moke_path)?;
    let mut header = vec![
        "time_s".to_string(),
        "angle_rad".to_string(),
        "moke_variance_status".to_string(),
    ];
    if variance_available {
        header.push("angle_variance_v2".to_string());
    }
    writer
        .write_record(&header)
        .context("cannot write MOKE header")?;
    for index in 0..times.len() {
        let mut record = vec![
            times[index].to_string(),
            angles[index].to_string(),
            statuses[index].to_string(),
        ];
        if variance_available {
            record.push(variances[index].expect("variance available").to_string());
        }
        writer
            .write_record(&record)
            .context("cannot write MOKE row")?;
    }
    writer.flush().context("cannot flush MOKE replay")?;
    if !variance_available {
        return Ok((moke_path, None));
    }
    // NPY mirror: time + angle (+ conditional variance) with the shared
    // C-order `<f8` writer.
    let variance_rms: Vec<f64> = variances
        .iter()
        .map(|value| value.expect("variance available"))
        .collect();
    let variance_path = npy_dir.join(format!("{}_angle_variance.csv", leg.name));
    let mut variance_writer = csv::WriterBuilder::new()
        .has_headers(true)
        .from_path(&variance_path)?;
    variance_writer
        .write_record(["time_s", "angle_variance_v2"])
        .context("cannot write variance header")?;
    for (time, variance) in times.iter().zip(variance_rms.iter()) {
        variance_writer
            .write_record([time.to_string(), variance.to_string()])
            .context("cannot write variance row")?;
    }
    variance_writer
        .flush()
        .context("cannot flush variance rows")?;
    let variance_npy = npy_dir.join(format!("{}_angle_variance.npy", leg.name));
    crate::utils::csv::write_npy(&variance_npy, &[times.clone(), variance_rms.clone()])
        .with_context(|| format!("cannot write {}", variance_npy.display()))?;
    Ok((moke_path, Some((variance_path, variance_npy))))
}
