use super::*;

pub fn load_from_path(path: impl AsRef<Path>) -> ConfigLoad {
    let path = path.as_ref();
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            return ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: None,
                warnings: Vec::new(),
                diagnostics: vec![ConfigDiagnostic::new(
                    DiagnosticKind::Io,
                    None,
                    format!("failed to read {}: {}", path.display(), err),
                    None,
                )],
                normalized: None,
            });
        }
    };

    let mut load = load_from_str(&text);
    if let ConfigLoad::Ready { config, .. } = &mut load {
        config.source_path = path.to_path_buf();
        config.source_text = Some(text);
        if config.version >= 4 {
            let output_dir = PathBuf::from(&config.plot.output_dir);
            config.plot_output_relative = (!output_dir.is_absolute()).then_some(output_dir);
            config.plot.output_dir = config
                .artifact_path(&config.plot.output_dir)
                .to_string_lossy()
                .into_owned();
        }
    }
    load
}

pub fn load_from_str(s: &str) -> ConfigLoad {
    let parsed_value = match toml::from_str::<toml::Value>(s) {
        Ok(value) => value,
        Err(err) => {
            return ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: None,
                warnings: Vec::new(),
                diagnostics: vec![ConfigDiagnostic::new(
                    DiagnosticKind::Parse,
                    None,
                    format!("toml parse error: {err}"),
                    None,
                )],
                normalized: None,
            });
        }
    };

    let version = match parsed_value.get("version").and_then(|v| v.as_integer()) {
        Some(v) if v >= 0 => v as u32,
        Some(v) => {
            return ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: None,
                warnings: Vec::new(),
                diagnostics: vec![ConfigDiagnostic::new(
                    DiagnosticKind::Parse,
                    Some("version".to_string()),
                    format!("version must be a non-negative integer (got {v})"),
                    None,
                )],
                normalized: None,
            });
        }
        None => {
            return ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: None,
                warnings: Vec::new(),
                diagnostics: vec![ConfigDiagnostic::new(
                    DiagnosticKind::Parse,
                    Some("version".to_string()),
                    "missing required top-level `version`".to_string(),
                    None,
                )],
                normalized: None,
            });
        }
    };

    match version {
        1 => match deserialize_versioned::<ConfigV1>(s) {
            Ok(raw) => normalize_v1(raw),
            Err(diag) => ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: Some(1),
                warnings: Vec::new(),
                diagnostics: vec![diag],
                normalized: None,
            }),
        },
        2 => match deserialize_versioned::<ConfigV2>(s) {
            Ok(raw) => normalize_v2(raw),
            Err(diag) => ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: Some(2),
                warnings: Vec::new(),
                diagnostics: vec![diag],
                normalized: None,
            }),
        },
        3 => match deserialize_versioned::<ConfigV3>(s) {
            Ok(raw) => normalize_v3(raw),
            Err(diag) => ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: Some(3),
                warnings: Vec::new(),
                diagnostics: vec![diag],
                normalized: None,
            }),
        },
        4 => match deserialize_versioned::<ConfigV4>(s) {
            Ok(raw) => normalize_v4(raw),
            Err(diag) => ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: Some(4),
                warnings: Vec::new(),
                diagnostics: vec![diag],
                normalized: None,
            }),
        },
        other => {
            ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: Some(other),
                warnings: Vec::new(),
                diagnostics: vec![ConfigDiagnostic::new(
                DiagnosticKind::Parse,
                Some("version".to_string()),
                format!("unsupported config version: {other}"),
                Some("use version = 1, 2, or 3 for legacy configs, or version = 4 for the simplified schema"
                    .to_string()),
            )],
                normalized: None,
            })
        }
    }
}

fn deserialize_versioned<T>(s: &str) -> std::result::Result<T, ConfigDiagnostic>
where
    T: for<'de> Deserialize<'de>,
{
    let de = toml::de::Deserializer::parse(s).map_err(|e| {
        ConfigDiagnostic::new(
            DiagnosticKind::Parse,
            None,
            format!("toml parse error: {e}"),
            None,
        )
    })?;

    serde_path_to_error::deserialize(de).map_err(|e| {
        ConfigDiagnostic::new(
            DiagnosticKind::Deserialize,
            Some(e.path().to_string()),
            e.to_string(),
            None,
        )
    })
}

fn normalize_v1(raw: ConfigV1) -> ConfigLoad {
    let mut warnings = Vec::new();
    let mut diagnostics = Vec::new();

    let reference_ch = match normalize_reference_ch_v1(&raw.roles.reference_ch) {
        Ok(ch) => ch,
        Err(diag) => {
            diagnostics.push(diag);
            return ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: Some(1),
                warnings,
                diagnostics,
                normalized: None,
            });
        }
    };

    if let Some(_demod) = raw.lockin.demodulation {
        warnings.push(ConfigWarning::new(
            "lockin.demodulation is deprecated in version 1 and ignored; complex demodulation is always used",
        ));
    }

    if raw.lockin.filter_length_samples.is_some() {
        warnings.push(ConfigWarning::new(
            "lockin.filter_length_samples is deprecated; it is interpreted as lockin.lpf_half_window_cycles during normalization",
        ));
    }

    if !raw.phase.use_signal_ch.is_empty() && raw.phase.use_signal_ch != raw.roles.signal_ch {
        diagnostics.push(ConfigDiagnostic::new(
            DiagnosticKind::Migration,
            Some("phase.use_signal_ch".to_string()),
            "phase.use_signal_ch is deprecated and cannot be migrated automatically when it differs from roles.signal_ch",
            Some(
                "remove phase.use_signal_ch and set roles.signal_ch to the exact signal channels you want to analyse".to_string(),
            ),
        ));
        return ConfigLoad::Diagnostics(ConfigDiagnostics {
            version: Some(1),
            warnings,
            diagnostics,
            normalized: None,
        });
    }

    let lpf_half_window_cycles = match raw
        .lockin
        .lpf_half_window_cycles
        .or_else(|| raw.lockin.filter_length_samples.map(|v| v as f64))
    {
        Some(v) => v,
        None => {
            diagnostics.push(ConfigDiagnostic::new(
                DiagnosticKind::Migration,
                Some("lockin".to_string()),
                "version 1 config must provide lockin.lpf_half_window_cycles or lockin.filter_length_samples",
                None,
            ));
            return ConfigLoad::Diagnostics(ConfigDiagnostics {
                version: Some(1),
                warnings,
                diagnostics,
                normalized: None,
            });
        }
    };

    let lpf_kind = raw.lockin.lpf_kind.unwrap_or_else(|| {
        if raw.lockin.filter_length_samples.is_some() && raw.lockin.lpf_half_window_cycles.is_none()
        {
            LockinLpfKind::BoxcarLegacy
        } else {
            LockinLpfKind::FirZeroPhase
        }
    });

    let mut cfg = Config {
        version: 3,
        instruments: raw.instruments.map(Into::into),
        fetch: Fetch::default(),
        screenshot: raw.screenshot.into(),
        plot: Plot::default(),
        source_path: PathBuf::from("config.toml"),
        source_text: None,
        artifact_root: None,
        plot_output_relative: None,
        legacy_timebase: Some(raw.timebase.into()),
        roles: Roles {
            sensor_ch: raw.roles.sensor_ch,
            reference_ch,
            signal_ch: raw.roles.signal_ch,
        },
        channels: raw.channels.into_iter().map(Into::into).collect(),
        pulse: raw.pulse.into(),
        reference: raw.reference.into(),
        lockin: Lockin {
            workers: raw.lockin.workers,
            stride_samples: raw.lockin.stride_samples,
            lpf_kind,
            lpf_half_window_cycles,
            lpf_cutoff_hz: raw.lockin.lpf_cutoff_hz,
            lpf_cutoff_ref_ratio: raw.lockin.lpf_cutoff_ref_ratio,
            lpf_stopband_atten_db: raw.lockin.lpf_stopband_atten_db,
            lpf_sync_average_cycles: raw.lockin.lpf_sync_average_cycles,
            lpf_iir_order: raw.lockin.lpf_iir_order,
            lpf_debug_output: raw.lockin.lpf_debug_output,
            lpf_debug_label: raw.lockin.lpf_debug_label,
            lpf_debug_overwrite: raw.lockin.lpf_debug_overwrite,
            snr_background_window: raw.lockin.snr_background_window,
            snr_signal_window: raw.lockin.snr_signal_window,
        },
        phase: Phase {
            m_omega_t0_offset: raw.phase.m_omega_t0_offset,
        },
        kerr: raw.kerr.into(),
    };

    let validation = validate_common(&mut cfg);
    warnings.extend(validation.warnings);
    diagnostics.extend(validation.errors);

    if diagnostics.is_empty() {
        ConfigLoad::Ready {
            config: cfg,
            warnings,
        }
    } else {
        ConfigLoad::Diagnostics(ConfigDiagnostics {
            version: Some(1),
            warnings,
            diagnostics,
            normalized: None,
        })
    }
}

fn normalize_v2(raw: ConfigV2) -> ConfigLoad {
    let legacy_timebase = raw.timebase.into();
    let mut warnings = vec![ConfigWarning::new(
        "legacy config v2: [timebase] is deprecated and is used only when raw.csv has no time column; raw metadata and newer CSV files use their recorded time axis",
    )];
    let mut cfg = Config {
        version: 3,
        instruments: raw.instruments.map(Into::into),
        fetch: raw.fetch.into(),
        screenshot: raw.screenshot.into(),
        plot: raw.plot.into(),
        source_path: PathBuf::from("config.toml"),
        source_text: None,
        artifact_root: None,
        plot_output_relative: None,
        legacy_timebase: Some(legacy_timebase),
        roles: Roles {
            sensor_ch: raw.roles.sensor_ch,
            reference_ch: raw.roles.reference_ch,
            signal_ch: raw.roles.signal_ch,
        },
        channels: raw.channels.into_iter().map(Into::into).collect(),
        pulse: raw.pulse.into(),
        reference: raw.reference.into(),
        lockin: Lockin {
            workers: raw.lockin.workers,
            stride_samples: raw.lockin.stride_samples,
            lpf_kind: raw.lockin.lpf_kind.unwrap_or(LockinLpfKind::FirZeroPhase),
            lpf_half_window_cycles: raw.lockin.lpf_half_window_cycles,
            lpf_cutoff_hz: raw.lockin.lpf_cutoff_hz,
            lpf_cutoff_ref_ratio: raw.lockin.lpf_cutoff_ref_ratio,
            lpf_stopband_atten_db: raw.lockin.lpf_stopband_atten_db,
            lpf_sync_average_cycles: raw.lockin.lpf_sync_average_cycles,
            lpf_iir_order: raw.lockin.lpf_iir_order,
            lpf_debug_output: raw.lockin.lpf_debug_output,
            lpf_debug_label: raw.lockin.lpf_debug_label,
            lpf_debug_overwrite: raw.lockin.lpf_debug_overwrite,
            snr_background_window: raw.lockin.snr_background_window,
            snr_signal_window: raw.lockin.snr_signal_window,
        },
        phase: Phase {
            m_omega_t0_offset: raw.phase.m_omega_t0_offset,
        },
        kerr: raw.kerr.into(),
    };

    let validation = validate_common(&mut cfg);
    if validation.errors.is_empty() {
        warnings.extend(validation.warnings);
        ConfigLoad::Ready {
            config: cfg,
            warnings,
        }
    } else {
        ConfigLoad::Diagnostics(ConfigDiagnostics {
            version: Some(2),
            warnings: {
                warnings.extend(validation.warnings);
                warnings
            },
            diagnostics: validation.errors,
            normalized: None,
        })
    }
}

fn normalize_v3(raw: ConfigV3) -> ConfigLoad {
    let mut cfg = Config {
        version: raw.version,
        instruments: raw.instruments.map(Into::into),
        fetch: raw.fetch.into(),
        screenshot: raw.screenshot.into(),
        plot: raw.plot.into(),
        source_path: PathBuf::from("config.toml"),
        source_text: None,
        artifact_root: None,
        plot_output_relative: None,
        legacy_timebase: None,
        roles: Roles {
            sensor_ch: raw.roles.sensor_ch,
            reference_ch: raw.roles.reference_ch,
            signal_ch: raw.roles.signal_ch,
        },
        channels: raw.channels.into_iter().map(Into::into).collect(),
        pulse: raw.pulse.into(),
        reference: raw.reference.into(),
        lockin: Lockin {
            workers: raw.lockin.workers,
            stride_samples: raw.lockin.stride_samples,
            lpf_kind: raw.lockin.lpf_kind.unwrap_or(LockinLpfKind::FirZeroPhase),
            lpf_half_window_cycles: raw.lockin.lpf_half_window_cycles,
            lpf_cutoff_hz: raw.lockin.lpf_cutoff_hz,
            lpf_cutoff_ref_ratio: raw.lockin.lpf_cutoff_ref_ratio,
            lpf_stopband_atten_db: raw.lockin.lpf_stopband_atten_db,
            lpf_sync_average_cycles: raw.lockin.lpf_sync_average_cycles,
            lpf_iir_order: raw.lockin.lpf_iir_order,
            lpf_debug_output: raw.lockin.lpf_debug_output,
            lpf_debug_label: raw.lockin.lpf_debug_label,
            lpf_debug_overwrite: raw.lockin.lpf_debug_overwrite,
            snr_background_window: raw.lockin.snr_background_window,
            snr_signal_window: raw.lockin.snr_signal_window,
        },
        phase: Phase {
            m_omega_t0_offset: raw.phase.m_omega_t0_offset,
        },
        kerr: raw.kerr.into(),
    };

    let validation = validate_common(&mut cfg);
    if validation.errors.is_empty() {
        ConfigLoad::Ready {
            config: cfg,
            warnings: validation.warnings,
        }
    } else {
        ConfigLoad::Diagnostics(ConfigDiagnostics {
            version: Some(3),
            warnings: validation.warnings,
            diagnostics: validation.errors,
            normalized: None,
        })
    }
}

fn normalize_v4(raw: ConfigV4) -> ConfigLoad {
    let mut errors = Vec::new();

    let scope_connection = match parse_connection_v4(&raw.scope.connection, "scope.connection") {
        Ok(connection) => Some(connection),
        Err(error) => {
            errors.push(error);
            None
        }
    };
    let generator_connection = match raw.generator.as_ref() {
        Some(generator) => match parse_connection_v4(&generator.connection, "generator.connection")
        {
            Ok(connection) => Some(connection),
            Err(error) => {
                errors.push(error);
                None
            }
        },
        None => None,
    };

    if raw.version != 4 {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("version".to_string()),
            format!(
                "version 4 schema must declare version = 4 (got {})",
                raw.version
            ),
            None,
        ));
    }
    if raw.scope.model != "DHO5108" {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("scope.model".to_string()),
            format!("unsupported oscilloscope model: {}", raw.scope.model),
            Some("use model = \"DHO5108\"".to_string()),
        ));
    }
    if let Some(connection) = &scope_connection {
        match connection {
            Connection::Tcpip { .. } => {}
            Connection::Usbtmc { .. } if cfg!(target_os = "windows") => {}
            Connection::Usbtmc { .. } => errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("scope.connection".to_string()),
                "visa connections currently require NI-VISA on Windows",
                Some("use a tcp://host:port connection on this platform".to_string()),
            )),
            Connection::Gpib { .. } => errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("scope.connection".to_string()),
                "DHO5108 does not support a GPIB connection",
                Some("use tcp://host:port or visa:RESOURCE".to_string()),
            )),
        }
    }
    if let Some(generator) = &raw.generator {
        if generator.model != "WF1946B" {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("generator.model".to_string()),
                format!("unsupported function generator model: {}", generator.model),
                Some("use model = \"WF1946B\"".to_string()),
            ));
        }
        if let Some(connection) = &generator_connection
            && !matches!(connection, Connection::Gpib { .. })
        {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("generator.connection".to_string()),
                "WF1946B requires a gpib://board/address connection",
                None,
            ));
        }
    }

    validate_v4_fields(&raw, &mut errors);

    if scope_connection.is_none() || (raw.generator.is_some() && generator_connection.is_none()) {
        return ConfigLoad::Diagnostics(ConfigDiagnostics {
            version: Some(4),
            warnings: Vec::new(),
            diagnostics: errors,
            normalized: None,
        });
    }
    let scope_connection = scope_connection.expect("scope connection parsed above");

    let sensor_ch = raw
        .sensors
        .iter()
        .map(|sensor| sensor.channel)
        .collect::<Vec<_>>();
    let mut channels = raw
        .sensors
        .iter()
        .map(channel_from_sensor_v4)
        .collect::<Vec<_>>();
    channels.extend(raw.lockin.signal_channels.iter().map(|&index| Channel {
        index,
        factor: None,
        scale_to_abs_max: None,
        label: None,
        unit_out: None,
    }));
    channels.push(Channel {
        index: raw.reference.channel,
        factor: None,
        scale_to_abs_max: None,
        label: None,
        unit_out: None,
    });

    let function_generator = raw.generator.map(|generator| FunctionGenerator {
        connection: generator_connection.expect("generator connection parsed above"),
        model: generator.model,
    });
    let mut cfg = Config {
        version: 4,
        instruments: Some(Instruments {
            function_generator,
            oscilloscope: Oscilloscope {
                connection: scope_connection,
                model: raw.scope.model,
            },
        }),
        fetch: Fetch {
            output: match raw.data.output {
                DataOutputV4::Csv => FetchOutput::Csv,
                DataOutputV4::Raw => FetchOutput::Raw,
                DataOutputV4::Both => FetchOutput::CsvAndRaw,
            },
            analysis_input: raw.data.input,
        },
        screenshot: Screenshot {
            enabled: raw.data.screenshot,
        },
        plot: raw.plot.into(),
        source_path: PathBuf::from("config.toml"),
        source_text: None,
        artifact_root: None,
        plot_output_relative: None,
        legacy_timebase: None,
        roles: Roles {
            sensor_ch,
            reference_ch: raw.reference.channel,
            signal_ch: raw.lockin.signal_channels.clone(),
        },
        channels,
        pulse: Pulse {
            bg_window_before: raw.pulse.background_before,
            bg_window_after: raw.pulse.background_after,
        },
        reference: raw.reference.into(),
        lockin: raw.lockin.into(),
        phase: Phase {
            m_omega_t0_offset: raw.phase.offsets,
        },
        kerr: Kerr {
            use_sensor_ch: raw.kerr.sensor,
            kerr_type: raw.kerr.method,
            factor: raw.kerr.factor,
        },
    };

    let validation = remap_v4_validation(validate_common(&mut cfg));
    errors.extend(validation.errors);
    if errors.is_empty() {
        ConfigLoad::Ready {
            config: cfg,
            warnings: validation.warnings,
        }
    } else {
        ConfigLoad::Diagnostics(ConfigDiagnostics {
            version: Some(4),
            warnings: validation.warnings,
            diagnostics: errors,
            normalized: None,
        })
    }
}

fn channel_from_sensor_v4(sensor: &SensorV4) -> Channel {
    let (factor, scale_to_abs_max) = match sensor.scale {
        SensorScaleV4::Factor(ref scale) => (Some(scale.factor), None),
        SensorScaleV4::MaxAbs(ref scale) => (None, Some(scale.max_abs * f64::from(scale.polarity))),
    };
    Channel {
        index: sensor.channel,
        factor,
        scale_to_abs_max,
        label: Some(sensor.label.clone()),
        unit_out: Some(sensor.unit.clone()),
    }
}

fn remap_v4_validation(mut validation: ValidationSummary) -> ValidationSummary {
    for warning in &mut validation.warnings {
        warning.message = v4_config_terms(&warning.message);
    }
    for error in &mut validation.errors {
        error.path = error.path.as_deref().map(v4_config_terms);
        error.message = v4_config_terms(&error.message);
        error.suggestion = error.suggestion.as_deref().map(v4_config_terms);
    }
    validation.errors.retain(|error| {
        !(error.path.as_deref() == Some("channels")
            && error.message.starts_with("duplicate channel index:"))
    });
    validation
}

fn v4_config_terms(value: &str) -> String {
    [
        (
            "lockin.lpf_sync_average_cycles",
            "lockin.filter.sync_average_cycles",
        ),
        (
            "lockin.lpf_half_window_cycles",
            "lockin.filter.half_window_cycles",
        ),
        (
            "lockin.lpf_stopband_atten_db",
            "lockin.filter.stopband_atten_db",
        ),
        (
            "lockin.lpf_cutoff_ref_ratio",
            "lockin.filter.cutoff_ref_ratio",
        ),
        ("lockin.lpf_cutoff_hz", "lockin.filter.cutoff_hz"),
        ("lockin.lpf_iir_order", "lockin.filter.iir_order"),
        ("lockin.lpf_debug_label", "lockin.debug_label"),
        ("lockin.lpf_kind", "lockin.filter.kind"),
        ("phase.m_omega_t0_offset", "phase.offsets"),
        ("pulse.bg_window_before", "pulse.background_before"),
        ("pulse.bg_window_after", "pulse.background_after"),
        ("kerr.use_sensor_ch", "kerr.sensor"),
        ("roles.reference_ch", "reference.channel"),
        ("roles.signal_ch", "lockin.signal_channels"),
        ("roles.sensor_ch", "sensors"),
    ]
    .into_iter()
    .fold(value.to_string(), |text, (old, new)| text.replace(old, new))
}

fn validate_v4_fields(raw: &ConfigV4, errors: &mut Vec<ConfigDiagnostic>) {
    let channel_in_range = |channel: u8| (1..=8).contains(&channel);
    let mut assignments = BTreeMap::<u8, String>::new();
    let mut assign = |channel: u8, path: String| {
        if let Some(first) = assignments.get(&channel) {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some(path.clone()),
                format!("channel {channel} is assigned more than once (first assigned at {first})"),
                None,
            ));
        } else {
            assignments.insert(channel, path);
        }
    };
    for (index, sensor) in raw.sensors.iter().enumerate() {
        assign(sensor.channel, format!("sensors[{index}].channel"));
    }
    assign(raw.reference.channel, "reference.channel".to_string());
    for (index, &channel) in raw.lockin.signal_channels.iter().enumerate() {
        assign(channel, format!("lockin.signal_channels[{index}]"));
    }

    if !channel_in_range(raw.reference.channel) {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("reference.channel".to_string()),
            format!(
                "DHO5108 channel must be in 1..=8 (got {})",
                raw.reference.channel
            ),
            None,
        ));
    }
    for (index, &channel) in raw.lockin.signal_channels.iter().enumerate() {
        if !channel_in_range(channel) {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some(format!("lockin.signal_channels[{index}]")),
                format!("DHO5108 channel must be in 1..=8 (got {channel})"),
                None,
            ));
        }
    }
    for (index, sensor) in raw.sensors.iter().enumerate() {
        if !channel_in_range(sensor.channel) {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some(format!("sensors[{index}].channel")),
                format!("DHO5108 channel must be in 1..=8 (got {})", sensor.channel),
                None,
            ));
        }
        if sensor.label.trim().is_empty() {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some(format!("sensors[{index}].label")),
                "sensor label must not be empty",
                None,
            ));
        }
        if sensor.unit.trim().is_empty() {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some(format!("sensors[{index}].unit")),
                "sensor unit must not be empty",
                None,
            ));
        }
        match sensor.scale {
            SensorScaleV4::Factor(ref scale)
                if !scale.factor.is_finite() || scale.factor == 0.0 =>
            {
                errors.push(ConfigDiagnostic::new(
                    DiagnosticKind::Validation,
                    Some(format!("sensors[{index}].scale.factor")),
                    "sensor scale factor must be finite and non-zero",
                    None,
                ));
            }
            SensorScaleV4::MaxAbs(ref scale) => {
                if !scale.max_abs.is_finite() || scale.max_abs <= 0.0 {
                    errors.push(ConfigDiagnostic::new(
                        DiagnosticKind::Validation,
                        Some(format!("sensors[{index}].scale.max_abs")),
                        "sensor scale max_abs must be finite and positive",
                        None,
                    ));
                }
                if !matches!(scale.polarity, -1 | 1) {
                    errors.push(ConfigDiagnostic::new(
                        DiagnosticKind::Validation,
                        Some(format!("sensors[{index}].scale.polarity")),
                        "sensor scale polarity must be -1 or 1",
                        None,
                    ));
                }
            }
            SensorScaleV4::Factor(_) => {}
        }
    }
    if raw.reference.stride_samples == 0 {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("reference.stride_samples".to_string()),
            "reference.stride_samples must be positive",
            None,
        ));
    }
    if raw.reference.window_samples == 0 {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("reference.window_samples".to_string()),
            "reference.window_samples must be positive",
            None,
        ));
    }
    if !raw.kerr.factor.is_finite() {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("kerr.factor".to_string()),
            "kerr.factor must be finite",
            None,
        ));
    }
    let before = raw.pulse.background_before;
    let after = raw.pulse.background_after;
    if before.start <= after.end && after.start <= before.end {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("pulse".to_string()),
            "pulse background windows must not overlap",
            None,
        ));
    }
}

fn parse_connection_v4(
    value: &str,
    path: &str,
) -> std::result::Result<Connection, ConfigDiagnostic> {
    let invalid = |message: String| {
        ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some(path.to_string()),
            message,
            Some("use tcp://host:port, visa:RESOURCE, or gpib://board/address".to_string()),
        )
    };
    if let Some(endpoint) = value.strip_prefix("tcp://") {
        let (host, port) = parse_tcp_endpoint_v4(endpoint)
            .map_err(|message| invalid(format!("invalid TCP connection '{value}': {message}")))?;
        return Ok(Connection::Tcpip { ip: host, port });
    }
    if let Some(resource) = value.strip_prefix("visa:") {
        let resource = resource.trim();
        if resource.is_empty() {
            return Err(invalid("VISA resource must not be empty".to_string()));
        }
        return Ok(Connection::Usbtmc {
            resource: resource.to_string(),
        });
    }
    if let Some(endpoint) = value.strip_prefix("gpib://") {
        let (board, address) = endpoint
            .split_once('/')
            .ok_or_else(|| invalid("GPIB connection must be gpib://board/address".to_string()))?;
        let board = board
            .parse::<u8>()
            .map_err(|_| invalid(format!("invalid GPIB board: {board}")))?;
        let address = address
            .parse::<u8>()
            .map_err(|_| invalid(format!("invalid GPIB address: {address}")))?;
        if address > 30 {
            return Err(invalid(format!(
                "GPIB address must be in 0..=30 (got {address})"
            )));
        }
        return Ok(Connection::Gpib { board, address });
    }
    Err(invalid(format!("unsupported connection string: {value}")))
}

fn parse_tcp_endpoint_v4(endpoint: &str) -> std::result::Result<(String, u16), String> {
    let (host, port) = if let Some(rest) = endpoint.strip_prefix('[') {
        let (host, port) = rest
            .split_once("]:")
            .ok_or_else(|| "IPv6 endpoint must be [address]:port".to_string())?;
        (host, port)
    } else {
        endpoint
            .rsplit_once(':')
            .ok_or_else(|| "endpoint must include a port".to_string())?
    };
    let host = host.trim();
    if host.is_empty() {
        return Err("host must not be empty".to_string());
    }
    let port = port
        .parse::<u16>()
        .map_err(|_| format!("invalid port: {port}"))?;
    if port == 0 {
        return Err("port must be in 1..=65535".to_string());
    }
    Ok((host.to_string(), port))
}
