//! Calibration-backed noise-model loading (WP-5B, AT-018).
//!
//! [`FileNoiseModelSource`] implements the engine's [`NoiseModelSource`]
//! seam from immutable calibration artifacts (INTERFACES section 4). Every
//! check runs on one frozen in-memory buffer: the bytes hashed are the bytes
//! parsed, so a file changed after discovery cannot slip through. Failure is
//! always explicit; nothing falls back, nothing is guessed.
//!
//! Run-side context the loader needs but never invents:
//! - `base_dir`: the declaring configuration file directory (never cwd).
//! - `voltage_unit`: the pipeline unit. pmoke's acquisition-to-analysis
//!   pipeline carries raw volts end to end (every header/plot labels `V`);
//!   the loader checks the artifact against that pipeline unit. A future
//!   multi-unit pipeline must thread real units here instead.
//! - config channel scaling (factor/scale_to_abs_max/unit_out) never touches
//!   lock-in estimator inputs: fetch stores physical volts, and factors apply
//!   at sensor/output stages only. The artifact variance therefore always
//!   matches estimator input units. If input rescaling upstream of LI is ever
//!   introduced, this loader must be revisited.

use crate::config::{EstimatorCalibration, GlsNoiseMode};
use crate::lockin::joint::{ModelBinding, NoiseModelSource, gls_noise_mode_name};
use anyhow::{Context, Result, bail};
use pmoke_analysis_core::calibration::{
    ApplicabilityRequest, CALIBRATION_ALGORITHM_VERSION, CALIBRATION_ARTIFACT_SCHEMA_VERSION,
    CALIBRATION_PHASE_CONVENTION, CalibrationArtifact, MAX_CALIBRATION_ARTIFACT_BYTES,
    VARIANCE_INTERP_ID, inspect_applicability,
};
use pmoke_analysis_core::joint::{
    CorrelationKernel, NoiseMode, NoiseModel, validate_correlation_kernel, validate_noise_model,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Pipeline voltage unit checked against artifact bindings (see module docs).
pub const PIPELINE_VOLTAGE_UNIT: &str = "V";

/// Phase-table center convention the runtime interpolator assumes
/// (uniform centers `2 pi (b + 0.5) / B`; mirrors the builder output).
pub const EXPECTED_PHASE_CENTER_CONVENTION: &str = "bin_centers_at_2pi*(i+0.5)/bins";

/// File-backed calibration source: one immutable artifact per channel.
pub struct FileNoiseModelSource {
    base_dir: PathBuf,
    entries: HashMap<u8, EstimatorCalibration>,
    voltage_unit: String,
    sample_interval_s: f64,
    reference_frequency_hz: f64,
    warned: std::cell::RefCell<HashSet<String>>,
}

impl FileNoiseModelSource {
    /// Builds the source. `calibrations` must cover every GLS channel;
    /// duplicates are rejected here rather than at first use.
    pub fn new(
        base_dir: PathBuf,
        calibrations: &[EstimatorCalibration],
        voltage_unit: String,
        sample_interval_s: f64,
        reference_frequency_hz: f64,
    ) -> Result<Self> {
        if !sample_interval_s.is_finite() || sample_interval_s <= 0.0 {
            bail!("joint model loading needs a positive finite sample interval");
        }
        if !reference_frequency_hz.is_finite() || reference_frequency_hz <= 0.0 {
            bail!("joint model loading needs a positive finite reference frequency");
        }
        let mut entries = HashMap::with_capacity(calibrations.len());
        for entry in calibrations {
            if entries.contains_key(&entry.channel) {
                bail!(
                    "duplicate calibration binding for channel {}",
                    entry.channel
                );
            }
            expect_digest_format(&entry.sha256).with_context(|| {
                format!(
                    "calibration binding for channel {} carries a malformed expected digest",
                    entry.channel
                )
            })?;
            entries.insert(entry.channel, entry.clone());
        }
        Ok(Self {
            base_dir,
            entries,
            voltage_unit,
            sample_interval_s,
            reference_frequency_hz,
            warned: std::cell::RefCell::new(HashSet::new()),
        })
    }

    fn warn_once(&self, warning: &str) {
        if self.warned.borrow_mut().insert(warning.to_string()) {
            crate::ui::warn(format!("joint model loading: {warning}"));
        }
    }
}

/// Requires exactly 64 lowercase hexadecimal SHA-256 characters. The
/// `${CALIBRATION_SHA256}` template placeholder and any other non-concrete
/// value fail here, before any file access.
fn expect_digest_format(digest: &str) -> Result<()> {
    if digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Ok(());
    }
    bail!("expected digest must be 64 lowercase hexadecimal SHA-256 characters");
}

/// Rejects duplicate object keys anywhere in the document (AT-018).
/// serde_json keeps the last duplicate silently, so a dedicated pass over
/// the same tokenizer drives key tracking per nesting level.
fn reject_duplicate_keys(bytes: &[u8]) -> Result<()> {
    use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
    use std::fmt;

    struct Checker;

    impl<'de> Visitor<'de> for Checker {
        type Value = ();

        fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
            formatter.write_str("any JSON value")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<(), A::Error> {
            use serde::de::Error;
            let mut seen = HashSet::new();
            while let Some(key) = map.next_key::<String>()? {
                if !seen.insert(key.clone()) {
                    return Err(A::Error::custom(format!("duplicate object key: {key}")));
                }
                map.next_value::<Nested>()?;
            }
            Ok(())
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
            while seq.next_element::<Nested>()?.is_some() {}
            Ok(())
        }

        fn visit_none<E: serde::de::Error>(self) -> Result<(), E> {
            Ok(())
        }

        fn visit_unit<E: serde::de::Error>(self) -> Result<(), E> {
            Ok(())
        }

        fn visit_some<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
            DeserializeSeed::deserialize(Nested, deserializer)
        }

        fn visit_newtype_struct<D: serde::Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<(), D::Error> {
            DeserializeSeed::deserialize(Nested, deserializer)
        }

        fn visit_bool<E: serde::de::Error>(self, _: bool) -> Result<(), E> {
            Ok(())
        }

        fn visit_i64<E: serde::de::Error>(self, _: i64) -> Result<(), E> {
            Ok(())
        }

        fn visit_u64<E: serde::de::Error>(self, _: u64) -> Result<(), E> {
            Ok(())
        }

        fn visit_f64<E: serde::de::Error>(self, _: f64) -> Result<(), E> {
            Ok(())
        }

        fn visit_str<E: serde::de::Error>(self, _: &str) -> Result<(), E> {
            Ok(())
        }

        fn visit_string<E: serde::de::Error>(self, _: String) -> Result<(), E> {
            Ok(())
        }

        fn visit_bytes<E: serde::de::Error>(self, _: &[u8]) -> Result<(), E> {
            Ok(())
        }

        fn visit_byte_buf<E: serde::de::Error>(self, _: Vec<u8>) -> Result<(), E> {
            Ok(())
        }
    }

    struct Nested;

    impl<'de> serde::Deserialize<'de> for Nested {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            DeserializeSeed::deserialize(Nested, deserializer)?;
            Ok(Nested)
        }
    }

    impl<'de> DeserializeSeed<'de> for Nested {
        type Value = ();

        fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
            deserializer.deserialize_any(Checker)
        }
    }

    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    DeserializeSeed::deserialize(Nested, &mut deserializer)
        .map_err(|error| anyhow::anyhow!("calibration artifact has {error}"))?;
    Ok(())
}

/// Reads one artifact file into a frozen buffer under the byte cap
/// (NFR-008: 1 MiB per channel). A single read serves hashing, parsing,
/// and validation alike.
fn read_frozen_artifact(path: &Path) -> Result<Vec<u8>> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("cannot open calibration artifact: {}", path.display()))?;
    let mut bytes = Vec::new();
    use std::io::Read;
    file.take(MAX_CALIBRATION_ARTIFACT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("cannot read calibration artifact: {}", path.display()))?;
    if bytes.len() > MAX_CALIBRATION_ARTIFACT_BYTES {
        bail!(
            "calibration artifact {} holds {} bytes above cap {MAX_CALIBRATION_ARTIFACT_BYTES}",
            path.display(),
            bytes.len()
        );
    }
    Ok(bytes)
}

fn mode_name(noise_mode: GlsNoiseMode) -> &'static str {
    gls_noise_mode_name(noise_mode)
}

/// Artifact-level semantic validation after hashing (AT-018): a correct
/// digest authenticates the selected bytes but does not make their contents
/// valid. Algorithm identity, declared shapes, coverage-record shapes,
/// correlation support metadata, and parameter domains are checked here,
/// before the artifact is reduced to a mode-specific noise model.
fn check_artifact_structure(artifact: &CalibrationArtifact, channel: u8) -> Result<()> {
    let here = |what: &str| format!("calibration artifact for channel {channel} has {what}");
    if artifact.algorithm_version != CALIBRATION_ALGORITHM_VERSION {
        bail!(
            "{}",
            here(&format!(
                "unknown algorithm_version {:?} (expected {CALIBRATION_ALGORITHM_VERSION:?})",
                artifact.algorithm_version
            ))
        );
    }
    if artifact.phase.bins < 2 {
        bail!("{}", here("a phase table with fewer than two bins"));
    }
    if artifact.phase.bins != artifact.phase.variances_v2.len() {
        bail!(
            "{}",
            here(&format!(
                "phase table declares {} bins but carries {} variances",
                artifact.phase.bins,
                artifact.phase.variances_v2.len()
            ))
        );
    }
    for (bin, variance) in artifact.phase.variances_v2.iter().enumerate() {
        if !variance.is_finite() || *variance <= 0.0 {
            bail!(
                "{}",
                here(&format!(
                    "non-positive non-finite phase variance in bin {bin}"
                ))
            );
        }
    }
    if artifact.training.per_bin_counts.len() != artifact.phase.bins
        || artifact.training.per_bin_cycles.len() != artifact.phase.bins
    {
        bail!(
            "{}",
            here("training coverage records do not match the phase table shape")
        );
    }
    if artifact.training.contributing_blocks == 0 {
        bail!(
            "{}",
            here("no contributing training blocks; the calibration is vacuous")
        );
    }
    if let Some(table) = artifact.correlation.as_ref() {
        if table.support_samples != table.lags.len() {
            bail!(
                "{}",
                here("correlation support_samples disagrees with the lag count")
            );
        }
        validate_correlation_kernel(&CorrelationKernel {
            lags: table.lags.clone(),
            lag_step_s: table.lag_step_s,
        })
        .map_err(|error| {
            anyhow::anyhow!(
                "{}: {}",
                here("an invalid correlation kernel"),
                error.message()
            )
        })?;
        if !(0.0..=1.0).contains(&table.shrinkage_eta) {
            bail!(
                "{}",
                here(&format!(
                    "correlation shrinkage eta {} lies outside [0, 1]",
                    table.shrinkage_eta
                ))
            );
        }
        if table.shrinkage_eta != artifact.regularization.correlation_eta {
            bail!(
                "{}",
                here("correlation eta disagrees with the regularization record")
            );
        }
        if let Some(tail) = table.tail_energy
            && (!tail.is_finite() || tail < 0.0)
        {
            bail!(
                "{}",
                here("correlation tail energy must be finite non-negative")
            );
        }
        // The validated lag vector is nonempty. Compare its final index
        // directly instead of incrementing untrusted metadata.
        if table.max_tail_lag < table.support_samples - 1 {
            bail!(
                "{}",
                here("correlation max_tail_lag precedes the taper support")
            );
        }
    }
    Ok(())
}

impl NoiseModelSource for FileNoiseModelSource {
    fn load(&self, channel: u8, noise_mode: GlsNoiseMode) -> Result<(NoiseModel, ModelBinding)> {
        let entry = self.entries.get(&channel).ok_or_else(|| {
            anyhow::anyhow!("no calibration binding configured for channel {channel}")
        })?;
        let path = if Path::new(&entry.path).is_absolute() {
            PathBuf::from(&entry.path)
        } else {
            self.base_dir.join(&entry.path)
        };
        // One frozen read serves hashing, parsing, and validation alike: the
        // bytes hashed are the bytes parsed (AT-018 changed-after-discovery).
        let bytes = read_frozen_artifact(&path)?;
        let actual = crate::utils::checksum::sha256_hex(&bytes);
        if actual != entry.sha256 {
            bail!(
                "calibration artifact digest mismatch for channel {channel}: \
                 file {} hashes to {actual}, expected {}",
                path.display(),
                entry.sha256
            );
        }
        reject_duplicate_keys(&bytes).with_context(|| {
            format!("calibration artifact for channel {channel} has duplicate keys")
        })?;
        let artifact: CalibrationArtifact = serde_json::from_slice(&bytes).with_context(|| {
            format!("calibration artifact for channel {channel} is not a valid v1 artifact")
        })?;
        if artifact.schema_version != CALIBRATION_ARTIFACT_SCHEMA_VERSION {
            bail!(
                "calibration artifact for channel {channel} has unsupported schema_version {} \
                 (expected {CALIBRATION_ARTIFACT_SCHEMA_VERSION})",
                artifact.schema_version
            );
        }
        check_artifact_structure(&artifact, channel)?;
        let name = mode_name(noise_mode);
        // Capability bridge: WP-3 artifacts predate the stationary runtime
        // and only advertise identity/phase_diagonal/phase_correlated, but a
        // present SPD-validated tapered correlation table IS the stationary
        // capability substance. Accept it explicitly (never silently); the
        // builder should advertise the mode directly in a later revision.
        let advertised = artifact.capabilities.modes.iter().any(|mode| mode == name);
        let bridged_stationary = noise_mode == GlsNoiseMode::StationaryCorrelated
            && artifact
                .correlation
                .as_ref()
                .is_some_and(|table| table.spd_validated);
        if !advertised && !bridged_stationary {
            bail!(
                "calibration artifact for channel {channel} does not advertise mode {name} \
                 (capabilities: {:?})",
                artifact.capabilities.modes
            );
        }
        if artifact.binding.phase_convention != CALIBRATION_PHASE_CONVENTION {
            bail!(
                "calibration artifact for channel {channel} uses unknown phase convention {:?}",
                artifact.binding.phase_convention
            );
        }
        let request = ApplicabilityRequest {
            channel: u32::from(channel),
            sample_interval_s: self.sample_interval_s,
            reference_frequency_hz: self.reference_frequency_hz,
            voltage_unit: self.voltage_unit.clone(),
            phase_convention: CALIBRATION_PHASE_CONVENTION.to_string(),
            adc_scale_provenance: None,
            acquisition: pmoke_analysis_core::calibration::AcquisitionMeta {
                device: None,
                gain: None,
                bandwidth_hz: None,
            },
        };
        let report = inspect_applicability(&artifact, &request);
        for warning in &report.warnings {
            self.warn_once(warning);
        }
        if !report.compatible {
            let details = report
                .errors
                .iter()
                .map(|error| format!("{}: {}", error.code, error.message))
                .collect::<Vec<_>>()
                .join("; ");
            bail!("calibration artifact for channel {channel} is not applicable: {details}");
        }
        let variance_bins = match noise_mode {
            GlsNoiseMode::PhaseDiagonal | GlsNoiseMode::PhaseCorrelated => {
                if artifact.phase.center_convention != EXPECTED_PHASE_CENTER_CONVENTION {
                    bail!(
                        "calibration artifact for channel {channel} uses unknown phase center \
                         convention {:?}",
                        artifact.phase.center_convention
                    );
                }
                if artifact.phase.interpolation != VARIANCE_INTERP_ID {
                    bail!(
                        "calibration artifact for channel {channel} uses unknown variance \
                         interpolation {:?}",
                        artifact.phase.interpolation
                    );
                }
                Some(artifact.phase.variances_v2.clone())
            }
            GlsNoiseMode::Identity | GlsNoiseMode::StationaryCorrelated => None,
        };
        let correlation = match noise_mode {
            GlsNoiseMode::StationaryCorrelated | GlsNoiseMode::PhaseCorrelated => {
                let table = artifact.correlation.as_ref().ok_or_else(|| {
                    anyhow::anyhow!(
                        "calibration artifact for channel {channel} carries no correlation \
                         table for mode {name}"
                    )
                })?;
                if !table.spd_validated {
                    bail!(
                        "calibration artifact for channel {channel} carries a correlation \
                         table without SPD evidence"
                    );
                }
                if table.taper != pmoke_analysis_core::calibration::CORRELATION_TAPER_ID {
                    bail!(
                        "calibration artifact for channel {channel} uses unknown correlation \
                         taper {:?}",
                        table.taper
                    );
                }
                let lag_detail =
                    (table.lag_step_s - self.sample_interval_s).abs() / self.sample_interval_s;
                if !lag_detail.is_finite() || lag_detail > artifact.binding.sample_interval_rel_tol
                {
                    bail!(
                        "calibration artifact for channel {channel} has lag step relative \
                         deviation {lag_detail:.3e} beyond the binding tolerance"
                    );
                }
                Some(CorrelationKernel {
                    lags: table.lags.clone(),
                    lag_step_s: table.lag_step_s,
                })
            }
            GlsNoiseMode::Identity | GlsNoiseMode::PhaseDiagonal => None,
        };
        let core_mode = match noise_mode {
            GlsNoiseMode::Identity => NoiseMode::Identity,
            GlsNoiseMode::PhaseDiagonal => NoiseMode::PhaseDiagonal,
            GlsNoiseMode::StationaryCorrelated => NoiseMode::StationaryCorrelated,
            GlsNoiseMode::PhaseCorrelated => NoiseMode::PhaseCorrelated,
        };
        let model = NoiseModel {
            mode: core_mode,
            reference_variance_v2: artifact.reference_variance_v2,
            variance_bins,
            correlation,
        };
        validate_noise_model(&model).map_err(|error| {
            anyhow::anyhow!(
                "calibration artifact for channel {channel} yields an invalid noise model: {}",
                error.message()
            )
        })?;
        Ok((
            model,
            ModelBinding {
                channel,
                path: path.display().to_string(),
                sha256: entry.sha256.clone(),
                noise_mode,
                bytes: Some(bytes),
            },
        ))
    }
}

#[cfg(test)]
#[path = "model_loading_tests.rs"]
mod tests;
