//! Deterministic, platform-independent validation for canonical pmoke configs v6 and v7.

pub mod connection;
mod model;

use connection::{ConnectionDefaults, ConnectionUri};
use model::{
    ConfigV6, ConfigV7, Filter, Generator, GlsCalibrationSourceV7, JointHarmonicGlsConfigV7,
    LockinEstimatorV7, LockinV7, Moke, Phase, Plot, Pulse, Reference, Scope, Sensor, SensorScale,
    Signal, Window,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ops::Range;

pub const REPORT_FORMAT_VERSION: u32 = 1;
pub const CONFIG_SCHEMA_VERSION: u32 = 6;
/// Canonical config versions accepted by browser validation.
pub const SUPPORTED_SCHEMA_VERSIONS: [u32; 2] = [6, 7];
/// Maximum GLS model parameters (NUMERICS shared-core cap, mirrored here so
/// the browser core stays dependency-light).
const MAX_GLS_MODEL_PARAMETERS: usize = 25;
pub const MAX_CONFIG_BYTES: usize = 1_048_576;
pub const CORE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const CORE_COMMIT: &str = env!("PMOKE_SOURCE_COMMIT");

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticCode {
    InputTooLarge,
    TomlSyntax,
    MissingVersion,
    InvalidVersion,
    UnsupportedVersion,
    SchemaMismatch,
    UnsupportedModel,
    InvalidConnection,
    UnsupportedTransport,
    PlatformNotChecked,
    DuplicateChannel,
    ChannelOutOfRange,
    EmptyValue,
    InvalidScale,
    InvalidCount,
    InvalidRange,
    InvalidWindow,
    OverlappingWindows,
    MutuallyExclusive,
    UnsafeLabel,
    InvalidExpression,
    DeprecatedField,
    ImplicitFallback,
    SerializationFailure,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceSpan {
    pub start: usize,
    pub end: usize,
    pub line: usize,
    pub column: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigDiagnostic {
    pub code: DiagnosticCode,
    pub severity: DiagnosticSeverity,
    pub path: Option<String>,
    pub span: Option<SourceSpan>,
    pub message: String,
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigSummary {
    pub version: u32,
    pub scope_model: String,
    pub scope_connection: String,
    pub generator_model: Option<String>,
    pub generator_connection: Option<String>,
    pub sensor_channels: Vec<u8>,
    pub reference_channel: u8,
    pub signal_channels: Vec<u8>,
    pub lockin_filter: String,
    pub lockin_workers: usize,
    pub plot_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub format_version: u32,
    pub core_version: String,
    pub core_commit: String,
    pub schema_version: Option<u32>,
    pub valid: bool,
    pub diagnostics: Vec<ConfigDiagnostic>,
    pub normalized_toml: Option<String>,
    pub summary: Option<ConfigSummary>,
}

impl ValidationReport {
    fn new(schema_version: Option<u32>) -> Self {
        Self {
            format_version: REPORT_FORMAT_VERSION,
            core_version: CORE_VERSION.to_string(),
            core_commit: CORE_COMMIT.to_string(),
            schema_version,
            valid: false,
            diagnostics: Vec::new(),
            normalized_toml: None,
            summary: None,
        }
    }

    pub fn error_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|item| item.severity == DiagnosticSeverity::Error)
            .count()
    }
}

pub fn validate_config_toml_json(input: &str) -> String {
    let report = validate_config_toml(input);
    serde_json::to_string(&report).unwrap_or_else(|_| {
        format!(
            "{{\"format_version\":{REPORT_FORMAT_VERSION},\"core_version\":\"{CORE_VERSION}\",\"core_commit\":\"{CORE_COMMIT}\",\"schema_version\":null,\"valid\":false,\"diagnostics\":[{{\"code\":\"serialization_failure\",\"severity\":\"error\",\"path\":null,\"span\":null,\"message\":\"failed to serialize validation report\",\"suggestion\":null}}],\"normalized_toml\":null,\"summary\":null}}"
        )
    })
}

pub fn validate_config_toml(input: &str) -> ValidationReport {
    if input.len() > MAX_CONFIG_BYTES {
        let mut report = ValidationReport::new(None);
        push(
            &mut report,
            DiagnosticCode::InputTooLarge,
            DiagnosticSeverity::Error,
            None,
            None,
            format!(
                "configuration input exceeds the {MAX_CONFIG_BYTES}-byte limit (got {} bytes)",
                input.len()
            ),
            Some("reduce the configuration below 1 MiB".to_string()),
        );
        return report;
    }

    let parsed = match toml::from_str::<toml::Value>(input) {
        Ok(value) => value,
        Err(error) => {
            let mut report = ValidationReport::new(None);
            let span = error.span().map(|span| source_span(input, span));
            push(
                &mut report,
                DiagnosticCode::TomlSyntax,
                DiagnosticSeverity::Error,
                None,
                span,
                format!("TOML syntax error: {error}"),
                Some("correct the TOML syntax at the reported location".to_string()),
            );
            return report;
        }
    };

    let version = match parsed.get("version") {
        None => {
            let mut report = ValidationReport::new(None);
            push(
                &mut report,
                DiagnosticCode::MissingVersion,
                DiagnosticSeverity::Error,
                Some("version".to_string()),
                None,
                "missing required top-level `version`",
                Some("add `version = 6` at the top of the configuration".to_string()),
            );
            return report;
        }
        Some(value) => match value
            .as_integer()
            .and_then(|value| u32::try_from(value).ok())
        {
            Some(value) => value,
            _ => {
                let mut report = ValidationReport::new(None);
                push(
                    &mut report,
                    DiagnosticCode::InvalidVersion,
                    DiagnosticSeverity::Error,
                    Some("version".to_string()),
                    None,
                    "version must be a non-negative integer",
                    Some("set `version = 6`".to_string()),
                );
                return report;
            }
        },
    };

    if !SUPPORTED_SCHEMA_VERSIONS.contains(&version) {
        let mut report = ValidationReport::new(Some(version));
        push(
            &mut report,
            DiagnosticCode::UnsupportedVersion,
            DiagnosticSeverity::Error,
            Some("version".to_string()),
            None,
            format!(
                "browser validation supports canonical config versions 6 and 7 (got {version})"
            ),
            Some("use `pmoke config migrate` for legacy configuration versions".to_string()),
        );
        return report;
    }

    let deserializer = match toml::de::Deserializer::parse(input) {
        Ok(deserializer) => deserializer,
        Err(error) => {
            let mut report = ValidationReport::new(Some(version));
            let span = error.span().map(|span| source_span(input, span));
            push(
                &mut report,
                DiagnosticCode::TomlSyntax,
                DiagnosticSeverity::Error,
                None,
                span,
                format!("TOML syntax error: {error}"),
                None,
            );
            return report;
        }
    };
    let mut config = match version {
        6 => match serde_path_to_error::deserialize::<_, ConfigV6>(deserializer) {
            Ok(config) => VersionedConfig::V6(config),
            Err(error) => return schema_mismatch(input, version, error),
        },
        _ => match serde_path_to_error::deserialize::<_, ConfigV7>(deserializer) {
            Ok(config) => VersionedConfig::V7(config),
            Err(error) => return schema_mismatch(input, version, error),
        },
    };

    let mut report = ValidationReport::new(Some(version));
    match &mut config {
        VersionedConfig::V6(config) => validate_v6(config, &mut report),
        VersionedConfig::V7(config) => validate_v7(config, &mut report),
    }
    report.valid = report.error_count() == 0;
    if report.valid {
        report.summary = Some(match &config {
            VersionedConfig::V6(config) => summary(config),
            VersionedConfig::V7(config) => summary_v7(config),
        });
        let rendered = match &config {
            VersionedConfig::V6(config) => toml::to_string_pretty(config),
            VersionedConfig::V7(config) => toml::to_string_pretty(config),
        };
        match rendered {
            Ok(normalized) => report.normalized_toml = Some(normalized),
            Err(error) => push(
                &mut report,
                DiagnosticCode::SerializationFailure,
                DiagnosticSeverity::Error,
                None,
                None,
                format!("failed to render normalized config: {error}"),
                None,
            ),
        }
        report.valid = report.error_count() == 0;
    }
    report
}

enum VersionedConfig {
    V6(ConfigV6),
    V7(ConfigV7),
}

fn schema_mismatch(
    input: &str,
    version: u32,
    error: serde_path_to_error::Error<toml::de::Error>,
) -> ValidationReport {
    let mut report = ValidationReport::new(Some(version));
    let span = error.inner().span().map(|span| source_span(input, span));
    push(
        &mut report,
        DiagnosticCode::SchemaMismatch,
        DiagnosticSeverity::Error,
        Some(error.path().to_string()),
        span,
        error.inner().to_string(),
        Some("compare this field with the generated config reference".to_string()),
    );
    report
}

/// Borrowed view of the sections shared by every supported canonical version.
struct Shared<'a> {
    scope: &'a mut Scope,
    generator: &'a mut Option<Generator>,
    sensors: &'a [Sensor],
    signals: &'a [Signal],
    pulse: &'a Pulse,
    reference: &'a Reference,
    lockin_channels: &'a [u8],
    lockin_snr_background_window: Option<Window>,
    lockin_snr_signal_window: Option<Window>,
    lockin_workers: usize,
    lockin_stride_samples: usize,
    lockin_debug_label: &'a Option<String>,
    phase: &'a Phase,
    moke: &'a Moke,
    plot: &'a Plot,
}

fn validate_v6(config: &mut ConfigV6, report: &mut ValidationReport) {
    if config.version != CONFIG_SCHEMA_VERSION {
        error(
            report,
            DiagnosticCode::InvalidVersion,
            "version",
            format!(
                "version 6 schema must declare version = 6 (got {})",
                config.version
            ),
        );
    }
    validate_shared(
        Shared {
            scope: &mut config.scope,
            generator: &mut config.generator,
            sensors: &config.sensors,
            signals: &config.signals,
            pulse: &config.pulse,
            reference: &config.reference,
            lockin_channels: &config.lockin.channels,
            lockin_snr_background_window: config.lockin.snr_background_window,
            lockin_snr_signal_window: config.lockin.snr_signal_window,
            lockin_workers: config.lockin.workers,
            lockin_stride_samples: config.lockin.stride_samples,
            lockin_debug_label: &config.lockin.debug_label,
            phase: &config.phase,
            moke: &config.moke,
            plot: &config.plot,
        },
        report,
    );
    validate_filter(&config.lockin.filter, report);
}

fn validate_v7(config: &mut ConfigV7, report: &mut ValidationReport) {
    if config.version != 7 {
        error(
            report,
            DiagnosticCode::InvalidVersion,
            "version",
            format!(
                "version 7 schema must declare version = 7 (got {})",
                config.version
            ),
        );
    }
    validate_shared(
        Shared {
            scope: &mut config.scope,
            generator: &mut config.generator,
            sensors: &config.sensors,
            signals: &config.signals,
            pulse: &config.pulse,
            reference: &config.reference,
            lockin_channels: &config.lockin.channels,
            lockin_snr_background_window: config.lockin.snr_background_window,
            lockin_snr_signal_window: config.lockin.snr_signal_window,
            lockin_workers: config.lockin.workers,
            lockin_stride_samples: config.lockin.stride_samples,
            lockin_debug_label: &config.lockin.debug_label,
            phase: &config.phase,
            moke: &config.moke,
            plot: &config.plot,
        },
        report,
    );
    validate_window_estimator(&config.lockin, report);
}

fn validate_shared(shared: Shared<'_>, report: &mut ValidationReport) {
    let Shared {
        scope, generator, ..
    } = shared;
    if scope.model != "DHO5108" {
        error(
            report,
            DiagnosticCode::UnsupportedModel,
            "scope.model",
            format!("unsupported oscilloscope model: {}", scope.model),
        );
    }
    match ConnectionUri::parse(&scope.connection, ConnectionDefaults::default()) {
        Ok(connection @ (ConnectionUri::Tcp { .. } | ConnectionUri::Visa { .. })) => {
            if matches!(connection, ConnectionUri::Visa { .. }) {
                warning(
                    report,
                    DiagnosticCode::PlatformNotChecked,
                    "scope.connection",
                    "VISA driver and Cargo feature availability are not checked in the browser",
                );
            }
            scope.connection = connection.to_string();
        }
        Ok(_) => error(
            report,
            DiagnosticCode::UnsupportedTransport,
            "scope.connection",
            "DHO5108 requires direct TCP/IP or VISA",
        ),
        Err(message) => error(
            report,
            DiagnosticCode::InvalidConnection,
            "scope.connection",
            message,
        ),
    }

    if let Some(generator) = generator {
        if generator.model != "WF1946B" {
            error(
                report,
                DiagnosticCode::UnsupportedModel,
                "generator.model",
                format!("unsupported function generator model: {}", generator.model),
            );
        }
        match ConnectionUri::parse(&generator.connection, ConnectionDefaults::default()) {
            Ok(
                connection @ (ConnectionUri::Gpib { .. }
                | ConnectionUri::PrologixTcp { .. }
                | ConnectionUri::PrologixSerial { .. }),
            ) => {
                warning(
                    report,
                    DiagnosticCode::PlatformNotChecked,
                    "generator.connection",
                    "transport feature, driver, and hardware reachability are not checked in the browser",
                );
                generator.connection = connection.to_string();
            }
            Ok(_) => error(
                report,
                DiagnosticCode::UnsupportedTransport,
                "generator.connection",
                "WF1946B requires GPIB or Prologix",
            ),
            Err(message) => error(
                report,
                DiagnosticCode::InvalidConnection,
                "generator.connection",
                message,
            ),
        }
    }

    validate_channels(
        shared.sensors,
        shared.reference.channel,
        shared.lockin_channels,
        shared.signals,
        report,
    );
    validate_windows(
        shared.pulse,
        shared.reference.fft_window,
        shared.lockin_snr_background_window,
        shared.lockin_snr_signal_window,
        report,
    );

    positive_usize(
        report,
        "reference.stride_samples",
        shared.reference.stride_samples,
    );
    positive_usize(
        report,
        "reference.window_samples",
        shared.reference.window_samples,
    );
    positive_usize(report, "lockin.workers", shared.lockin_workers);
    positive_usize(
        report,
        "lockin.stride_samples",
        shared.lockin_stride_samples,
    );
    positive_usize(report, "plot.max_points", shared.plot.max_points);

    if let Some(label) = shared.lockin_debug_label
        && !safe_debug_label(label)
    {
        error(
            report,
            DiagnosticCode::UnsafeLabel,
            "lockin.debug_label",
            "debug label must be 1-64 ASCII alphanumeric, '.', '_', or '-' characters and must not be '.' or '..'",
        );
    }

    if shared.phase.offsets.len() != 6 {
        error(
            report,
            DiagnosticCode::InvalidCount,
            "phase.offsets",
            format!(
                "phase.offsets must have length 6 (got {})",
                shared.phase.offsets.len()
            ),
        );
    }
    for (index, offset) in shared.phase.offsets.iter().enumerate() {
        match offset.evaluate() {
            Ok(value) if value.is_finite() => {}
            Ok(value) => error(
                report,
                DiagnosticCode::InvalidRange,
                format!("phase.offsets[{index}]"),
                format!("phase offset must be finite (got {value})"),
            ),
            Err(message) => error(
                report,
                DiagnosticCode::InvalidExpression,
                format!("phase.offsets[{index}]"),
                format!("invalid phase expression: {message}"),
            ),
        }
    }
    if !shared.moke.factor.is_finite() {
        error(
            report,
            DiagnosticCode::InvalidRange,
            "moke.factor",
            "moke.factor must be finite",
        );
    }
    if !shared
        .sensors
        .iter()
        .any(|sensor| sensor.channel == shared.moke.sensor)
    {
        error(
            report,
            DiagnosticCode::InvalidRange,
            "moke.sensor",
            format!(
                "moke.sensor ({}) is not defined in sensors",
                shared.moke.sensor
            ),
        );
    }
    if shared.plot.output_dir.is_some() {
        warning(
            report,
            DiagnosticCode::DeprecatedField,
            "plot.output_dir",
            "plot.output_dir is deprecated and ignored; plots use analysis/plots",
        );
    }
}

fn validate_channels(
    sensors: &[Sensor],
    reference_channel: u8,
    lockin_channels: &[u8],
    signals: &[Signal],
    report: &mut ValidationReport,
) {
    let mut assignments = BTreeMap::<u8, String>::new();
    let mut assign = |channel: u8, path: String, report: &mut ValidationReport| {
        if !(1..=8).contains(&channel) {
            error(
                report,
                DiagnosticCode::ChannelOutOfRange,
                path.clone(),
                format!("DHO5108 channel must be in 1..=8 (got {channel})"),
            );
        }
        if let Some(first) = assignments.get(&channel) {
            error(
                report,
                DiagnosticCode::DuplicateChannel,
                path,
                format!("channel {channel} is assigned more than once (first assigned at {first})"),
            );
        } else {
            assignments.insert(channel, path);
        }
    };

    for (index, sensor) in sensors.iter().enumerate() {
        let base = format!("sensors[{index}]");
        assign(sensor.channel, format!("{base}.channel"), report);
        if sensor.label.trim().is_empty() {
            error(
                report,
                DiagnosticCode::EmptyValue,
                format!("{base}.label"),
                "sensor label must not be empty",
            );
        }
        if sensor.unit.trim().is_empty() {
            error(
                report,
                DiagnosticCode::EmptyValue,
                format!("{base}.unit"),
                "sensor unit must not be empty",
            );
        }
        match &sensor.scale {
            SensorScale::Factor(scale) if !scale.factor.is_finite() || scale.factor == 0.0 => {
                error(
                    report,
                    DiagnosticCode::InvalidScale,
                    format!("{base}.scale.factor"),
                    "sensor scale factor must be finite and non-zero",
                );
            }
            SensorScale::MaxAbs(scale) => {
                if !scale.max_abs.is_finite() || scale.max_abs <= 0.0 {
                    error(
                        report,
                        DiagnosticCode::InvalidScale,
                        format!("{base}.scale.max_abs"),
                        "sensor scale max_abs must be finite and positive",
                    );
                }
                if !matches!(scale.polarity, -1 | 1) {
                    error(
                        report,
                        DiagnosticCode::InvalidScale,
                        format!("{base}.scale.polarity"),
                        "sensor scale polarity must be -1 or 1",
                    );
                }
            }
            SensorScale::Factor(_) => {}
        }
    }
    // A zero reference channel is the unspecified sentinel: it assigns no
    // hardware channel and is rejected later by reference-gated targets.
    if reference_channel != 0 {
        assign(reference_channel, "reference.channel".to_string(), report);
    }
    for (index, channel) in lockin_channels.iter().copied().enumerate() {
        assign(channel, format!("lockin.channels[{index}]"), report);
    }
    for (index, signal) in signals.iter().enumerate() {
        let base = format!("signals[{index}]");
        assign(signal.channel, format!("{base}.channel"), report);
        if signal.label.trim().is_empty() {
            error(
                report,
                DiagnosticCode::EmptyValue,
                format!("{base}.label"),
                "signal label must not be empty",
            );
        }
        if signal.unit.trim().is_empty() {
            error(
                report,
                DiagnosticCode::EmptyValue,
                format!("{base}.unit"),
                "signal unit must not be empty",
            );
        }
    }
}

fn validate_windows(
    pulse: &Pulse,
    reference_fft_window: Window,
    lockin_snr_background_window: Option<Window>,
    lockin_snr_signal_window: Option<Window>,
    report: &mut ValidationReport,
) {
    check_window(report, "pulse.background_before", pulse.background_before);
    check_window(report, "pulse.background_after", pulse.background_after);
    check_window(report, "reference.fft_window", reference_fft_window);
    if let Some(window) = lockin_snr_background_window {
        check_window(report, "lockin.snr_background_window", window);
    }
    if let Some(window) = lockin_snr_signal_window {
        check_window(report, "lockin.snr_signal_window", window);
    }
    let before = pulse.background_before;
    let after = pulse.background_after;
    if before.start <= after.end && after.start <= before.end {
        error(
            report,
            DiagnosticCode::OverlappingWindows,
            "pulse",
            "pulse background windows must not overlap",
        );
    }
}

fn validate_filter(filter: &Filter, report: &mut ValidationReport) {
    positive_f64(
        report,
        "lockin.filter.half_window_cycles",
        filter.half_window_cycles(),
    );
}

fn validate_window_estimator(lockin: &LockinV7, report: &mut ValidationReport) {
    positive_f64(
        report,
        "lockin.window.half_window_cycles",
        lockin.window.half_window_cycles,
    );
    let config = match &lockin.estimator {
        LockinEstimatorV7::BoxcarLegacy {} => return,
        LockinEstimatorV7::JointHarmonicGls(config) => config,
    };
    if config.fit_harmonics.is_empty() {
        error(
            report,
            DiagnosticCode::InvalidCount,
            "lockin.estimator.fit_harmonics",
            "lockin.estimator.fit_harmonics must not be empty",
        );
    }
    let mut previous = 0usize;
    let mut ordered = true;
    for harmonic in &config.fit_harmonics {
        if *harmonic == 0 || *harmonic <= previous {
            ordered = false;
            break;
        }
        previous = *harmonic;
    }
    if !ordered {
        error(
            report,
            DiagnosticCode::InvalidRange,
            "lockin.estimator.fit_harmonics",
            "lockin.estimator.fit_harmonics must be ascending unique positive harmonics",
        );
    }
    if 1 + 2 * config.fit_harmonics.len() > MAX_GLS_MODEL_PARAMETERS {
        error(
            report,
            DiagnosticCode::InvalidCount,
            "lockin.estimator.fit_harmonics",
            format!(
                "lockin.estimator.fit_harmonics needs {} model parameters, above the limit of {MAX_GLS_MODEL_PARAMETERS}",
                1 + 2 * config.fit_harmonics.len()
            ),
        );
    }
    if config.output_harmonics.as_slice() != [1usize, 2, 3, 4, 5, 6] {
        error(
            report,
            DiagnosticCode::InvalidCount,
            "lockin.estimator.output_harmonics",
            "lockin.estimator.output_harmonics must be exactly [1, 2, 3, 4, 5, 6]",
        );
    }
    for harmonic in &config.output_harmonics {
        if !config.fit_harmonics.contains(harmonic) {
            error(
                report,
                DiagnosticCode::InvalidRange,
                "lockin.estimator.fit_harmonics",
                format!("lockin.estimator.fit_harmonics must contain output harmonic {harmonic}"),
            );
        }
    }
    if config.envelope_degree != 0 {
        error(
            report,
            DiagnosticCode::InvalidRange,
            "lockin.estimator.envelope_degree",
            format!(
                "lockin.estimator.envelope_degree must be exactly 0 in v1 (got {})",
                config.envelope_degree
            ),
        );
    }
    validate_estimator_calibrations(config, &lockin.channels, report);
}

fn validate_estimator_calibrations(
    config: &JointHarmonicGlsConfigV7,
    lockin_channels: &[u8],
    report: &mut ValidationReport,
) {
    // FR-05/08 mirror: prepulse derives every channel model, so file bindings
    // alongside it are ambiguous; the per-channel rule below is artifact-only.
    if matches!(config.calibration_source, GlsCalibrationSourceV7::Prepulse) {
        if !config.calibrations.is_empty() {
            error(
                report,
                DiagnosticCode::MutuallyExclusive,
                "lockin.estimator.calibration_source",
                format!(
                    "lockin.estimator.calibration_source is prepulse, so lockin.estimator.calibrations must be empty (got {} entries)",
                    config.calibrations.len()
                ),
            );
        }
        return;
    }
    let mut seen: Vec<u8> = Vec::with_capacity(config.calibrations.len());
    for (index, calibration) in config.calibrations.iter().enumerate() {
        if !lockin_channels.contains(&calibration.channel) {
            error(
                report,
                DiagnosticCode::InvalidRange,
                format!("lockin.estimator.calibrations[{index}].channel"),
                format!(
                    "lockin.estimator.calibrations[{index}].channel ({}) is not a configured lock-in channel",
                    calibration.channel
                ),
            );
        }
        if seen.contains(&calibration.channel) {
            error(
                report,
                DiagnosticCode::DuplicateChannel,
                format!("lockin.estimator.calibrations[{index}].channel"),
                format!(
                    "duplicate lockin.estimator.calibrations entry for channel {}",
                    calibration.channel
                ),
            );
        }
        seen.push(calibration.channel);
        if !is_concrete_sha256(&calibration.sha256) {
            error(
                report,
                DiagnosticCode::InvalidRange,
                format!("lockin.estimator.calibrations[{index}].sha256"),
                format!(
                    "lockin.estimator.calibrations[{index}].sha256 must be 64 lowercase hex characters (got {:?})",
                    calibration.sha256
                ),
            );
        }
    }
    for channel in lockin_channels {
        if !seen.contains(channel) {
            error(
                report,
                DiagnosticCode::InvalidCount,
                "lockin.estimator.calibrations",
                format!(
                    "lockin.estimator.calibrations is missing an entry for lock-in channel {channel}"
                ),
            );
        }
    }
}

fn is_concrete_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn check_window(report: &mut ValidationReport, path: &str, window: Window) {
    if !window.start.is_finite() || !window.end.is_finite() {
        error(
            report,
            DiagnosticCode::InvalidWindow,
            path,
            format!(
                "window start and end must be finite (start={}, end={})",
                window.start, window.end
            ),
        );
    } else if window.start >= window.end {
        error(
            report,
            DiagnosticCode::InvalidWindow,
            path,
            format!(
                "window start must be less than end (start={}, end={})",
                window.start, window.end
            ),
        );
    }
}

fn positive_usize(report: &mut ValidationReport, path: &str, value: usize) {
    if value == 0 {
        error(
            report,
            DiagnosticCode::InvalidRange,
            path,
            format!("{path} must be positive"),
        );
    }
}

fn positive_f64(report: &mut ValidationReport, path: &str, value: f64) {
    if !value.is_finite() || value <= 0.0 {
        error(
            report,
            DiagnosticCode::InvalidRange,
            path,
            format!("{path} must be finite and positive (got {value})"),
        );
    }
}

fn safe_debug_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 64
        && label != "."
        && label != ".."
        && label
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn summary_v7(config: &ConfigV7) -> ConfigSummary {
    ConfigSummary {
        version: config.version,
        scope_model: config.scope.model.clone(),
        scope_connection: config.scope.connection.clone(),
        generator_model: config
            .generator
            .as_ref()
            .map(|generator| generator.model.clone()),
        generator_connection: config
            .generator
            .as_ref()
            .map(|generator| generator.connection.clone()),
        sensor_channels: config.sensors.iter().map(|sensor| sensor.channel).collect(),
        reference_channel: config.reference.channel,
        signal_channels: config.lockin.channels.clone(),
        lockin_filter: config.lockin.estimator.name().to_string(),
        lockin_workers: config.lockin.workers,
        plot_mode: config.plot.mode.as_str().to_string(),
    }
}

fn summary(config: &ConfigV6) -> ConfigSummary {
    ConfigSummary {
        version: config.version,
        scope_model: config.scope.model.clone(),
        scope_connection: config.scope.connection.clone(),
        generator_model: config
            .generator
            .as_ref()
            .map(|generator| generator.model.clone()),
        generator_connection: config
            .generator
            .as_ref()
            .map(|generator| generator.connection.clone()),
        sensor_channels: config.sensors.iter().map(|sensor| sensor.channel).collect(),
        reference_channel: config.reference.channel,
        signal_channels: config.lockin.channels.clone(),
        lockin_filter: config.lockin.filter.kind().to_string(),
        lockin_workers: config.lockin.workers,
        plot_mode: config.plot.mode.as_str().to_string(),
    }
}

fn source_span(input: &str, range: Range<usize>) -> SourceSpan {
    let start = range.start.min(input.len());
    let prefix = &input[..start];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
    let column = prefix[line_start..].chars().count() + 1;
    SourceSpan {
        start,
        end: range.end.min(input.len()),
        line,
        column,
    }
}

fn error(
    report: &mut ValidationReport,
    code: DiagnosticCode,
    path: impl Into<String>,
    message: impl Into<String>,
) {
    push(
        report,
        code,
        DiagnosticSeverity::Error,
        Some(path.into()),
        None,
        message,
        None,
    );
}

fn warning(
    report: &mut ValidationReport,
    code: DiagnosticCode,
    path: impl Into<String>,
    message: impl Into<String>,
) {
    push(
        report,
        code,
        DiagnosticSeverity::Warning,
        Some(path.into()),
        None,
        message,
        None,
    );
}

fn push(
    report: &mut ValidationReport,
    code: DiagnosticCode,
    severity: DiagnosticSeverity,
    path: Option<String>,
    span: Option<SourceSpan>,
    message: impl Into<String>,
    suggestion: Option<String>,
) {
    report.diagnostics.push(ConfigDiagnostic {
        code,
        severity,
        path,
        span,
        message: message.into(),
        suggestion,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"version = 6
[scope]
model = "DHO5108"
connection = "tcp://192.0.2.10:55255"
[data]
output = "raw"
input = "raw"
[[sensors]]
channel = 1
scale = { factor = -2.0 }
label = "field"
unit = "T"
[pulse]
background_before = { start = -0.005, end = -0.001 }
background_after = { start = 0.01, end = 0.02 }
[reference]
channel = 2
fft_window = { start = 0.0, end = 0.005 }
stride_samples = 100
window_samples = 1000
[lockin]
channels = [3]
workers = 2
stride_samples = 100
filter = { kind = "boxcar_legacy", half_window_cycles = 1.0 }
[phase]
offsets = [0, 0, 0, 0, 0, 0]
[moke]
sensor = 1
method = "harmonics"
factor = -1.0
"#;

    #[test]
    fn valid_v6_has_actual_summary_and_normalized_output() {
        let report = validate_config_toml(VALID);
        assert!(report.valid, "{:#?}", report.diagnostics);
        let summary = report.summary.unwrap();
        assert_eq!(summary.sensor_channels, vec![1]);
        assert_eq!(summary.reference_channel, 2);
        assert_eq!(summary.signal_channels, vec![3]);
        assert!(report.normalized_toml.unwrap().contains("channel = 2"));
    }

    const VALID_V7_LEGACY: &str = r#"version = 7
[scope]
model = "DHO5108"
connection = "tcp://192.0.2.10:55255"
[data]
output = "raw"
input = "raw"
[[sensors]]
channel = 1
scale = { factor = -2.0 }
label = "field"
unit = "T"
[pulse]
background_before = { start = -0.005, end = -0.001 }
background_after = { start = 0.01, end = 0.02 }
[reference]
channel = 2
fft_window = { start = 0.0, end = 0.005 }
stride_samples = 100
window_samples = 1000
[lockin]
channels = [3]
workers = 2
stride_samples = 100
[lockin.window]
kind = "reference_cycles"
half_window_cycles = 1.0
edge_policy = "legacy_trim"
[lockin.estimator]
kind = "boxcar_legacy"
[phase]
offsets = [0, 0, 0, 0, 0, 0]
[moke]
sensor = 1
method = "harmonics"
factor = -1.0
"#;

    const SHA_A: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn valid_v7_gls() -> String {
        VALID_V7_LEGACY.replace(
            "[lockin.estimator]\nkind = \"boxcar_legacy\"",
            &format!(
                "[lockin.estimator]\nkind = \"joint_harmonic_gls\"\nfit_harmonics = [1, 2, 3, 4, 5, 6]\noutput_harmonics = [1, 2, 3, 4, 5, 6]\nnoise_mode = \"identity\"\n[[lockin.estimator.calibrations]]\nchannel = 3\npath = \"calibration/ch3.json\"\nsha256 = \"{SHA_A}\""
            ),
        )
    }

    #[test]
    fn valid_v7_legacy_reports_version_7_and_round_trips() {
        let report = validate_config_toml(VALID_V7_LEGACY);
        assert!(report.valid, "{:#?}", report.diagnostics);
        assert_eq!(report.schema_version, Some(7));
        let summary = report.summary.unwrap();
        assert_eq!(summary.lockin_filter, "boxcar_legacy");
        let normalized = report.normalized_toml.unwrap();
        let reparsed = validate_config_toml(&normalized);
        assert!(reparsed.valid, "{:#?}", reparsed.diagnostics);
    }

    #[test]
    fn valid_v7_gls_names_the_estimator_and_round_trips() {
        let input = valid_v7_gls();
        let report = validate_config_toml(&input);
        assert!(report.valid, "{:#?}", report.diagnostics);
        assert_eq!(report.summary.unwrap().lockin_filter, "joint_harmonic_gls");
        let normalized = report.normalized_toml.unwrap();
        assert!(normalized.contains("joint_harmonic_gls"));
        let reparsed = validate_config_toml(&normalized);
        assert!(reparsed.valid, "{:#?}", reparsed.diagnostics);
    }

    #[test]
    fn v7_rejects_filter_tables_gls_keys_under_legacy_and_bad_digests() {
        // v6 filter table is rejected in v7 with a schema mismatch.
        let with_filter = VALID_V7_LEGACY.replace(
            "[lockin.window]",
            "filter = { kind = \"boxcar_legacy\", half_window_cycles = 1.0 }\n[lockin.window]",
        );
        let report = validate_config_toml(&with_filter);
        assert!(!report.valid);
        assert_eq!(report.diagnostics[0].code, DiagnosticCode::SchemaMismatch);

        // GLS-only settings are rejected under boxcar_legacy.
        let gls_under_legacy = VALID_V7_LEGACY.replace(
            "kind = \"boxcar_legacy\"",
            "kind = \"boxcar_legacy\"\nfit_harmonics = [1, 2]",
        );
        let report = validate_config_toml(&gls_under_legacy);
        assert!(!report.valid);
        assert_eq!(report.diagnostics[0].code, DiagnosticCode::SchemaMismatch);

        // Unexpanded template digests are rejected.
        let template = valid_v7_gls().replace(SHA_A, "${CALIBRATION_SHA256}");
        let report = validate_config_toml(&template);
        assert!(!report.valid);
        assert!(report.diagnostics.iter().any(|item| {
            item.path
                .as_deref()
                .unwrap_or_default()
                .ends_with(".sha256")
        }));

        // Missing per-channel bindings are rejected.
        let missing = valid_v7_gls().replace(
            &format!(
                "[[lockin.estimator.calibrations]]\nchannel = 3\npath = \"calibration/ch3.json\"\nsha256 = \"{SHA_A}\"\n"
            ),
            "",
        );
        let report = validate_config_toml(&missing);
        assert!(!report.valid);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|item| item.path.as_deref() == Some("lockin.estimator.calibrations"))
        );

        // Wrong output harmonics are rejected.
        let output = valid_v7_gls().replace(
            "output_harmonics = [1, 2, 3, 4, 5, 6]",
            "output_harmonics = [1, 2, 3]",
        );
        let report = validate_config_toml(&output);
        assert!(!report.valid);
    }

    #[test]
    fn v7_prepulse_matches_native_validation() {
        // AT-08: prepulse with empty calibrations is valid here exactly when
        // it is valid natively; mixing file bindings with prepulse is a
        // mutual-exclusion error on both sides (FR-05/FR-08).
        let prepulse = VALID_V7_LEGACY.replace(
            "[lockin.estimator]\nkind = \"boxcar_legacy\"",
            "[lockin.estimator]\nkind = \"joint_harmonic_gls\"\nfit_harmonics = [1, 2, 3, 4, 5, 6]\noutput_harmonics = [1, 2, 3, 4, 5, 6]\nnoise_mode = \"identity\"\ncalibration_source = \"prepulse\"",
        );
        let report = validate_config_toml(&prepulse);
        assert!(report.valid, "{:#?}", report.diagnostics);
        let mixed = prepulse.replace(
            "calibration_source = \"prepulse\"",
            &format!(
                "calibration_source = \"prepulse\"\n[[lockin.estimator.calibrations]]\nchannel = 3\npath = \"calibration/ch3.json\"\nsha256 = \"{SHA_A}\""
            ),
        );
        let report = validate_config_toml(&mixed);
        assert!(!report.valid);
        assert!(
            report
                .diagnostics
                .iter()
                .any(|item| item.code == DiagnosticCode::MutuallyExclusive)
        );
    }

    #[test]
    fn rejects_schema_and_cross_field_errors() {
        let report = validate_config_toml(&VALID.replace("channel = 2", "channel = 1"));
        assert!(!report.valid);
        assert!(report.diagnostics.iter().any(|item| {
            item.code == DiagnosticCode::DuplicateChannel
                && item.path.as_deref() == Some("reference.channel")
        }));
    }

    #[test]
    fn signals_entries_validate_and_overlap_is_rejected() {
        let input = format!("{VALID}\n[[signals]]\nchannel = 4\nlabel = \"DC\"\nunit = \"V\"\n");
        let report = validate_config_toml(&input);
        assert!(report.valid, "{:#?}", report.diagnostics);
        assert!(report.normalized_toml.unwrap().contains("[[signals]]"));

        let overlap = format!("{VALID}\n[[signals]]\nchannel = 3\nlabel = \"DC\"\nunit = \"V\"\n");
        let report = validate_config_toml(&overlap);
        assert!(!report.valid);
        assert!(report.diagnostics.iter().any(|item| {
            item.code == DiagnosticCode::DuplicateChannel
                && item.path.as_deref() == Some("signals[0].channel")
        }));
    }

    #[test]
    fn repeated_channels_keep_the_original_assignment_path() {
        let input = VALID
            .replace("channels = [3]", "channels = [1, 1]")
            .replace("channel = 2", "channel = 1");
        let report = validate_config_toml(&input);
        let duplicates = report
            .diagnostics
            .iter()
            .filter(|item| item.code == DiagnosticCode::DuplicateChannel)
            .collect::<Vec<_>>();
        assert_eq!(duplicates.len(), 3);
        assert!(duplicates.iter().all(|item| {
            item.message
                .contains("first assigned at sensors[0].channel")
        }));
    }

    #[test]
    fn legacy_signal_channels_key_is_aliased_to_channels() {
        let input = VALID.replace("channels = [3]", "signal_channels = [3]");
        let report = validate_config_toml(&input);
        assert!(report.valid, "{:#?}", report.diagnostics);
        assert_eq!(report.summary.unwrap().signal_channels, vec![3]);
    }

    #[test]
    fn syntax_diagnostic_has_a_source_span() {
        let report = validate_config_toml("version = 6\n[scope\n");
        let span = report.diagnostics[0].span.as_ref().unwrap();
        assert!(span.line >= 2);
        assert!(span.column >= 1);
    }

    #[test]
    fn size_limit_is_in_bytes_and_precedes_parsing() {
        let report = validate_config_toml(&"x".repeat(MAX_CONFIG_BYTES + 1));
        assert_eq!(report.diagnostics[0].code, DiagnosticCode::InputTooLarge);
    }

    #[test]
    fn exact_size_limit_is_still_parsed() {
        let input = format!("version = 6\n#{}", "x".repeat(MAX_CONFIG_BYTES - 13));
        assert_eq!(input.len(), MAX_CONFIG_BYTES);
        let report = validate_config_toml(&input);
        assert_ne!(report.diagnostics[0].code, DiagnosticCode::InputTooLarge);
    }

    #[test]
    fn rejects_unknown_fields_and_legacy_versions_with_stable_codes() {
        let unknown = validate_config_toml(
            &VALID.replace("model = \"DHO5108\"", "model = \"DHO5108\"\nunknown = true"),
        );
        assert_eq!(unknown.diagnostics[0].code, DiagnosticCode::SchemaMismatch);

        let legacy = validate_config_toml(&VALID.replacen("version = 6", "version = 3", 1));
        assert_eq!(
            legacy.diagnostics[0].code,
            DiagnosticCode::UnsupportedVersion
        );
        assert_eq!(legacy.schema_version, Some(3));

        let oversized_version =
            validate_config_toml(&VALID.replacen("version = 6", "version = 4294967300", 1));
        assert_eq!(
            oversized_version.diagnostics[0].code,
            DiagnosticCode::InvalidVersion
        );
    }

    #[test]
    fn rejects_removed_and_unknown_filter_kinds_and_fields() {
        for replacement in [
            "filter = { kind = \"fir_zero_phase\", half_window_cycles = 1.0 }",
            "filter = { kind = \"future_filter\", half_window_cycles = 1.0 }",
            "filter = { kind = \"boxcar_legacy\", half_window_cycles = 1.0, cutoff_hz = 10.0 }",
        ] {
            let report = validate_config_toml(&VALID.replace(
                "filter = { kind = \"boxcar_legacy\", half_window_cycles = 1.0 }",
                replacement,
            ));
            assert!(!report.valid, "unexpectedly accepted {replacement}");
            assert_eq!(
                report.diagnostics[0].code,
                DiagnosticCode::SchemaMismatch,
                "diagnostics: {:?}",
                report.diagnostics
            );
        }
    }

    #[test]
    fn json_report_is_deterministic_and_round_trips() {
        let first = validate_config_toml_json(VALID);
        let second = validate_config_toml_json(VALID);
        assert_eq!(first, second);

        let report: ValidationReport = serde_json::from_str(&first).unwrap();
        assert!(report.valid);
        assert_eq!(report.format_version, REPORT_FORMAT_VERSION);
        assert_eq!(report.schema_version, Some(CONFIG_SCHEMA_VERSION));
    }
}
