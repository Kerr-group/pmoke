use super::*;

pub(super) fn validate_common(cfg: &mut Config) -> ValidationSummary {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    if !matches!(cfg.version, 3..=7) {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("version".to_string()),
            format!(
                "normalized config must have version 3, 4, 5, or 6 (got {})",
                cfg.version
            ),
            None,
        ));
    }
    if cfg.plot.max_points == 0 {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("plot.max_points".to_string()),
            "plot.max_points must be positive",
            None,
        ));
    }
    if cfg.plot.output_dir.trim().is_empty() {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("plot.output_dir".to_string()),
            "plot.output_dir must not be empty",
            None,
        ));
    }
    if matches!(
        cfg.instruments
            .as_ref()
            .map(|instruments| &instruments.oscilloscope.connection),
        Some(Connection::Usbtmc { .. })
    ) && !usbtmc_supported()
    {
        errors.push(usbtmc_unsupported_diagnostic(
            "instruments.oscilloscope.connection",
        ));
    }
    if cfg.lockin.workers == 0 {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.workers".to_string()),
            "lockin.workers must be positive",
            None,
        ));
    }
    if cfg.lockin.stride_samples == 0 {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.stride_samples".to_string()),
            "lockin.stride_samples must be positive",
            None,
        ));
    }
    if !cfg.lockin.lpf_half_window_cycles.is_finite() || cfg.lockin.lpf_half_window_cycles <= 0.0 {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.lpf_half_window_cycles".to_string()),
            format!(
                "lockin.lpf_half_window_cycles must be positive (got {})",
                cfg.lockin.lpf_half_window_cycles
            ),
            None,
        ));
    }
    if !matches!(cfg.lockin.lpf_kind, LockinLpfKind::BoxcarLegacy) {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.lpf_kind".to_string()),
            "the active runtime supports only the boxcar_legacy LPF",
            Some(
                "set lockin.filter.kind = \"boxcar_legacy\" after reviewing the behavior change"
                    .to_string(),
            ),
        ));
    }
    // Legacy-compat mirrors must track the validated window contract.
    if cfg.lockin.lpf_half_window_cycles != cfg.lockin.window.half_window_cycles {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.window.half_window_cycles".to_string()),
            "internal error: legacy window mirror diverged from lockin.window",
            None,
        ));
    }
    if !cfg.lockin.window.half_window_cycles.is_finite()
        || cfg.lockin.window.half_window_cycles <= 0.0
    {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.window.half_window_cycles".to_string()),
            format!(
                "lockin.window.half_window_cycles must be positive (got {})",
                cfg.lockin.window.half_window_cycles
            ),
            None,
        ));
    }
    validate_estimator(&cfg.lockin, &cfg.roles.signal_ch, &mut errors);
    if let Some(label) = &cfg.lockin.lpf_debug_label
        && !is_safe_debug_label(label)
    {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.lpf_debug_label".to_string()),
            "lockin.lpf_debug_label must be 1-64 ASCII characters using only A-Z, a-z, 0-9, '.', '_', or '-', and must not be '.' or '..'",
            None,
        ));
    }
    if cfg.phase.m_omega_t0_offset.len() != 6 {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("phase.m_omega_t0_offset".to_string()),
            format!(
                "phase.m_omega_t0_offset must have length 6 (got {})",
                cfg.phase.m_omega_t0_offset.len()
            ),
            None,
        ));
    }
    for (idx, value) in cfg.phase.m_omega_t0_offset.iter().enumerate() {
        if !value.is_finite() {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some(format!("phase.m_omega_t0_offset[{idx}]")),
                format!("phase.m_omega_t0_offset[{idx}] must be finite (got {value})"),
                None,
            ));
        }
    }

    let mut seen = BTreeSet::new();
    for ch in &cfg.channels {
        if !seen.insert(ch.index) {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("channels".to_string()),
                format!("duplicate channel index: {}", ch.index),
                None,
            ));
        }
    }

    for &idx in &cfg.roles.sensor_ch {
        if !seen.contains(&idx) {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("roles.sensor_ch".to_string()),
                format!("roles.sensor_ch contains undefined channel index: {}", idx),
                None,
            ));
        }
    }
    for &idx in &cfg.roles.signal_ch {
        if !seen.contains(&idx) {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("roles.signal_ch".to_string()),
                format!("roles.signal_ch contains undefined channel index: {}", idx),
                None,
            ));
        }
    }
    if cfg.roles.reference_ch != 0 && !seen.contains(&cfg.roles.reference_ch) {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("roles.reference_ch".to_string()),
            format!(
                "roles.reference_ch ({}) is not defined in channels",
                cfg.roles.reference_ch
            ),
            None,
        ));
    }
    if !cfg.roles.sensor_ch.contains(&cfg.moke.use_sensor_ch) {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("moke.use_sensor_ch".to_string()),
            format!(
                "moke.use_sensor_ch ({}) is not included in roles.sensor_ch",
                cfg.moke.use_sensor_ch
            ),
            None,
        ));
    }

    let check_win = |label: &str, w: Window| -> Option<ConfigDiagnostic> {
        if !w.start.is_finite() || !w.end.is_finite() {
            Some(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some(label.to_string()),
                format!(
                    "{label}: start and end must be finite (start={}, end={})",
                    w.start, w.end
                ),
                None,
            ))
        } else if w.start < w.end {
            None
        } else {
            Some(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some(label.to_string()),
                format!(
                    "{label}: start must be < end (start={}, end={})",
                    w.start, w.end
                ),
                None,
            ))
        }
    };
    if let Some(diag) = check_win("pulse.bg_window_before", cfg.pulse.bg_window_before) {
        errors.push(diag);
    }
    if let Some(diag) = check_win("pulse.bg_window_after", cfg.pulse.bg_window_after) {
        errors.push(diag);
    }
    if let Some(diag) = check_win("reference.fft_window", cfg.reference.fft_window) {
        errors.push(diag);
    }
    if let Some(window) = cfg.lockin.snr_background_window
        && let Some(diag) = check_win("lockin.snr_background_window", window)
    {
        errors.push(diag);
    }
    if let Some(window) = cfg.lockin.snr_signal_window
        && let Some(diag) = check_win("lockin.snr_signal_window", window)
    {
        errors.push(diag);
    }

    let mut used = BTreeSet::new();
    used.extend(cfg.roles.sensor_ch.iter().copied());
    used.extend(cfg.roles.signal_ch.iter().copied());
    used.insert(cfg.roles.reference_ch);
    for ch in &cfg.channels {
        if !used.contains(&ch.index) {
            warnings.push(ConfigWarning::new(format!(
                "channel index {} is defined in [channels] but not used in roles",
                ch.index
            )));
        }
    }

    cfg.channels.sort_by_key(|ch| ch.index);

    ValidationSummary { warnings, errors }
}

fn is_safe_debug_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 64
        && label != "."
        && label != ".."
        && label
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

pub fn validate_for_target(cfg: &Config, target: ValidationTarget) -> Result<()> {
    match target {
        ValidationTarget::Single
        | ValidationTarget::Fetch
        | ValidationTarget::Screenshot
        | ValidationTarget::Process
        | ValidationTarget::Auto => {
            validate_oscilloscope_required(cfg)?;
        }
        ValidationTarget::Trigger | ValidationTarget::Autoshot | ValidationTarget::Automeasure => {
            validate_oscilloscope_required(cfg)?;
            validate_function_generator_required(cfg)?;
        }
        ValidationTarget::Reference
        | ValidationTarget::Sensor
        | ValidationTarget::Li
        | ValidationTarget::Signal
        | ValidationTarget::Phase
        | ValidationTarget::Moke
        | ValidationTarget::Analyze => {}
    }

    let needs_screenshot = matches!(target, ValidationTarget::Screenshot)
        || (cfg.screenshot.enabled
            && matches!(
                target,
                ValidationTarget::Fetch
                    | ValidationTarget::Automeasure
                    | ValidationTarget::Process
                    | ValidationTarget::Auto
            ));
    if needs_screenshot {
        validate_screenshot_target(cfg)?;
    }

    match target {
        ValidationTarget::Reference => {
            validate_oscilloscope_required(cfg)?;
            validate_reference_roles(cfg)?;
            validate_analysis_input_exists(cfg)?;
        }
        ValidationTarget::Sensor => {
            validate_oscilloscope_required(cfg)?;
            validate_sensor_roles(cfg)?;
            validate_sensor_metadata(cfg)?;
            validate_analysis_input_exists(cfg)?;
        }
        ValidationTarget::Li => {
            validate_oscilloscope_required(cfg)?;
            validate_reference_roles(cfg)?;
            validate_sensor_roles(cfg)?;
            validate_signal_roles(cfg)?;
            validate_sensor_metadata(cfg)?;
            validate_analysis_input_exists(cfg)?;
        }
        ValidationTarget::Signal => {
            validate_oscilloscope_required(cfg)?;
            validate_reference_roles(cfg)?;
            validate_sensor_roles(cfg)?;
            validate_signal_roles(cfg)?;
            validate_sensor_metadata(cfg)?;
            validate_analysis_input_exists(cfg)?;
            validate_signal_entries(cfg)?;
        }
        ValidationTarget::Phase => {
            validate_signal_roles(cfg)?;
            validate_sensor_metadata(cfg)?;
            validate_lockin_results_exist(cfg)?;
        }
        ValidationTarget::Moke => {
            validate_signal_roles(cfg)?;
            validate_sensor_roles(cfg)?;
            validate_sensor_metadata(cfg)?;
            validate_moke_sensor(cfg)?;
            validate_rotated_results_exist(cfg)?;
        }
        ValidationTarget::Analyze => {
            validate_oscilloscope_required(cfg)?;
            validate_reference_roles(cfg)?;
            validate_sensor_roles(cfg)?;
            validate_signal_roles(cfg)?;
            validate_sensor_metadata(cfg)?;
            validate_analysis_input_exists(cfg)?;
        }
        ValidationTarget::Process => {
            validate_reference_roles(cfg)?;
            validate_sensor_roles(cfg)?;
            validate_signal_roles(cfg)?;
            validate_sensor_metadata(cfg)?;
        }
        ValidationTarget::Auto => {
            validate_function_generator_required(cfg)?;
            validate_reference_roles(cfg)?;
            validate_sensor_roles(cfg)?;
            validate_signal_roles(cfg)?;
            validate_sensor_metadata(cfg)?;
        }
        ValidationTarget::Automeasure
        | ValidationTarget::Fetch
        | ValidationTarget::Screenshot
        | ValidationTarget::Single
        | ValidationTarget::Trigger
        | ValidationTarget::Autoshot => {}
    }

    Ok(())
}

fn validate_screenshot_target(cfg: &Config) -> Result<()> {
    let oscilloscope = &cfg
        .instruments
        .as_ref()
        .ok_or_else(|| anyhow!("instruments.oscilloscope is required"))?
        .oscilloscope;
    match &oscilloscope.connection {
        Connection::Gpib { .. }
        | Connection::PrologixTcp { .. }
        | Connection::PrologixSerial { .. } => {
            bail!("DHO5108 display capture requires TCP/IP or USB-TMC");
        }
        Connection::Tcpip { .. } | Connection::Usbtmc { .. } => {}
    }
    Ok(())
}

fn validate_reference_roles(cfg: &Config) -> Result<()> {
    if cfg.roles.reference_ch == 0 {
        bail!("roles.reference_ch must be set");
    }
    Ok(())
}

fn validate_sensor_roles(cfg: &Config) -> Result<()> {
    if cfg.roles.sensor_ch.is_empty() {
        bail!("roles.sensor_ch must contain at least one channel");
    }
    Ok(())
}

fn validate_signal_roles(cfg: &Config) -> Result<()> {
    if cfg.roles.signal_ch.is_empty() {
        bail!("roles.signal_ch must contain at least one channel");
    }
    Ok(())
}

fn validate_signal_entries(cfg: &Config) -> Result<()> {
    if cfg.signals.is_empty() {
        bail!("no [[signals]] entries are configured");
    }
    Ok(())
}

fn validate_moke_sensor(cfg: &Config) -> Result<()> {
    if !cfg.roles.sensor_ch.contains(&cfg.moke.use_sensor_ch) {
        bail!(
            "moke.use_sensor_ch ({}) must be included in roles.sensor_ch",
            cfg.moke.use_sensor_ch
        );
    }
    Ok(())
}

fn validate_oscilloscope_required(cfg: &Config) -> Result<()> {
    cfg.instruments
        .as_ref()
        .ok_or_else(|| anyhow!("instruments configuration is required for this command"))?;
    Ok(())
}

fn validate_function_generator_required(cfg: &Config) -> Result<()> {
    let instruments = cfg
        .instruments
        .as_ref()
        .ok_or_else(|| anyhow!("instruments configuration is required for this command"))?;

    if instruments.function_generator.is_none() {
        bail!("instruments.function_generator is required for this command");
    }
    Ok(())
}

pub(super) fn validate_sensor_metadata(cfg: &Config) -> Result<()> {
    for ch in &cfg.roles.sensor_ch {
        let meta = cfg
            .channels
            .iter()
            .find(|c| c.index == *ch)
            .ok_or_else(|| anyhow!("channel {} is not defined in [channels]", ch))?;

        match (meta.factor, meta.scale_to_abs_max) {
            (Some(_), Some(_)) => {
                bail!("channel {ch} cannot set both 'factor' and 'scale_to_abs_max'");
            }
            (Some(factor), None) => {
                if !factor.is_finite() {
                    bail!("channel {ch} factor must be finite");
                }
            }
            (None, Some(scale_to_abs_max)) => {
                if !scale_to_abs_max.is_finite() || scale_to_abs_max == 0.0 {
                    bail!("channel {ch} scale_to_abs_max must be finite and non-zero");
                }
            }
            (None, None) => {
                bail!("channel {ch} must set either 'factor' or 'scale_to_abs_max'");
            }
        }
        if meta.label.is_none() {
            bail!("channel {} has no 'label'", ch);
        }
        if meta.unit_out.is_none() {
            bail!("channel {} has no 'unit_out'", ch);
        }
    }
    for channel in &cfg.channels {
        if channel.scale_to_abs_max.is_some() && !cfg.roles.sensor_ch.contains(&channel.index) {
            bail!(
                "channel {} has 'scale_to_abs_max' but is not listed in roles.sensor_ch",
                channel.index
            );
        }
    }
    Ok(())
}

fn validate_analysis_input_exists(cfg: &Config) -> Result<()> {
    let resolver = cfg.resolver();
    match cfg.fetch.analysis_input {
        FetchAnalysisInput::Csv => validate_raw_csv_exists(cfg),
        FetchAnalysisInput::Raw => validate_raw_metadata_exists(cfg),
        FetchAnalysisInput::Auto => {
            let metadata = resolver.acquisition_manifest();
            let raw_dir = metadata.parent().unwrap_or_else(|| Path::new("."));
            if metadata.exists() {
                Ok(())
            } else if raw_dir.exists() {
                bail!("raw metadata not found: {}", metadata.display())
            } else {
                validate_raw_csv_exists(cfg)
            }
        }
    }
}

fn validate_raw_csv_exists(cfg: &Config) -> Result<()> {
    let resolver = cfg.resolver();
    let path = resolver.waveform_csv();
    validate_file_exists(&path, &path.display().to_string())
}

/// Validates the `[lockin.estimator]` contract. A well-formed GLS
/// configuration is accepted here; execution availability is decided at the
/// lock-in entry point, never by validation.
fn validate_estimator(lockin: &Lockin, signal_ch: &[u8], errors: &mut Vec<ConfigDiagnostic>) {
    let config = match &lockin.estimator {
        LockinEstimator::BoxcarLegacy => return,
        LockinEstimator::JointHarmonicGls(config) => config,
    };
    if config.fit_harmonics.is_empty() {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.estimator.fit_harmonics".to_string()),
            "lockin.estimator.fit_harmonics must not be empty",
            None,
        ));
    }
    let mut previous = 0usize;
    for (idx, harmonic) in config.fit_harmonics.iter().enumerate() {
        if *harmonic == 0 || *harmonic <= previous {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some(format!("lockin.estimator.fit_harmonics[{idx}]")),
                format!(
                    "lockin.estimator.fit_harmonics must be ascending unique positive harmonics (got {} at index {idx})",
                    config
                        .fit_harmonics
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                None,
            ));
            break;
        }
        previous = *harmonic;
    }
    let parameters = 1 + 2 * config.fit_harmonics.len();
    if parameters > pmoke_analysis_core::MAX_MODEL_PARAMETERS {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.estimator.fit_harmonics".to_string()),
            format!(
                "lockin.estimator.fit_harmonics needs {parameters} model parameters, above the limit of {}",
                pmoke_analysis_core::MAX_MODEL_PARAMETERS
            ),
            None,
        ));
    }
    if config.output_harmonics.as_slice() != [1usize, 2, 3, 4, 5, 6] {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.estimator.output_harmonics".to_string()),
            "lockin.estimator.output_harmonics must be exactly [1, 2, 3, 4, 5, 6]",
            None,
        ));
    }
    for harmonic in &config.output_harmonics {
        if !config.fit_harmonics.contains(harmonic) {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("lockin.estimator.fit_harmonics".to_string()),
                format!("lockin.estimator.fit_harmonics must contain output harmonic {harmonic}"),
                None,
            ));
        }
    }
    if config.envelope_degree != 0 {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.estimator.envelope_degree".to_string()),
            format!(
                "lockin.estimator.envelope_degree must be exactly 0 in v1 (got {})",
                config.envelope_degree
            ),
            None,
        ));
    }
    validate_estimator_calibrations(config, signal_ch, errors);
}

fn validate_estimator_calibrations(
    config: &JointHarmonicGlsConfig,
    signal_ch: &[u8],
    errors: &mut Vec<ConfigDiagnostic>,
) {
    const BASE: &str = "lockin.estimator.calibrations";
    let mut seen: Vec<u8> = Vec::with_capacity(config.calibrations.len());
    for (idx, calibration) in config.calibrations.iter().enumerate() {
        if !signal_ch.contains(&calibration.channel) {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some(format!("{BASE}[{idx}].channel")),
                format!(
                    "lockin.estimator.calibrations[{idx}].channel ({}) is not a configured lock-in channel",
                    calibration.channel
                ),
                Some("use exactly one entry per lockin.channels value".to_string()),
            ));
        }
        if seen.contains(&calibration.channel) {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some(format!("{BASE}[{idx}].channel")),
                format!(
                    "duplicate lockin.estimator.calibrations entry for channel {}",
                    calibration.channel
                ),
                None,
            ));
        }
        seen.push(calibration.channel);
        if !is_concrete_sha256(&calibration.sha256) {
            let template_hint = if calibration.sha256.contains('$')
                || calibration.sha256.contains('{')
                || calibration.sha256.contains('}')
            {
                "expand the calibration template placeholder into the 64-character lowercase hex digest before running"
            } else {
                "use the 64-character lowercase hex digest recorded when the calibration artifact was created"
            };
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some(format!("{BASE}[{idx}].sha256")),
                format!(
                    "lockin.estimator.calibrations[{idx}].sha256 must be 64 lowercase hex characters (got {:?})",
                    calibration.sha256
                ),
                Some(template_hint.to_string()),
            ));
        }
    }
    for channel in signal_ch {
        if !seen.contains(channel) {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some(BASE.to_string()),
                format!(
                    "lockin.estimator.calibrations is missing an entry for lock-in channel {channel}"
                ),
                Some("use exactly one entry per lockin.channels value".to_string()),
            ));
        }
    }
}

fn is_concrete_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn validate_raw_metadata_exists(cfg: &Config) -> Result<()> {
    let resolver = cfg.resolver();
    let path = resolver.acquisition_manifest();
    validate_file_exists(&path, &path.display().to_string())
}

fn validate_lockin_results_exist(cfg: &Config) -> Result<()> {
    let resolver = cfg.resolver();
    for ch in cfg.phase_signal_ch() {
        let path = resolver.lockin_xy_csv(*ch);
        validate_file_exists(&path, &path.display().to_string())?;
    }
    Ok(())
}

fn validate_rotated_results_exist(cfg: &Config) -> Result<()> {
    let resolver = cfg.resolver();
    for ch in cfg.phase_signal_ch() {
        let path = resolver.lockin_rotated_csv(*ch);
        validate_file_exists(&path, &path.display().to_string())?;
    }
    Ok(())
}

fn validate_file_exists(path: &Path, label: &str) -> Result<()> {
    if path.exists() {
        Ok(())
    } else {
        bail!("{label} does not exist")
    }
}
