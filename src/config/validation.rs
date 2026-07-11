use super::*;

fn uses_explicit_cutoff(kind: LockinLpfKind) -> bool {
    matches!(
        kind,
        LockinLpfKind::FirZeroPhase | LockinLpfKind::SyncIirZeroPhase
    )
}

pub(super) fn validate_common(cfg: &mut Config) -> ValidationSummary {
    let mut warnings = Vec::new();
    let mut errors = Vec::new();

    if !matches!(cfg.version, 3 | 4) {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("version".to_string()),
            format!(
                "normalized config must have version 3 or 4 (got {})",
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
    if !cfg.lockin.lpf_stopband_atten_db.is_finite() || cfg.lockin.lpf_stopband_atten_db <= 0.0 {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.lpf_stopband_atten_db".to_string()),
            format!(
                "lockin.lpf_stopband_atten_db must be positive (got {})",
                cfg.lockin.lpf_stopband_atten_db
            ),
            None,
        ));
    }
    if cfg.lockin.lpf_kind == LockinLpfKind::SyncIirZeroPhase {
        let max_sync_cycles = 100.0;
        if !cfg.lockin.lpf_sync_average_cycles.is_finite()
            || cfg.lockin.lpf_sync_average_cycles <= 0.0
            || cfg.lockin.lpf_sync_average_cycles > max_sync_cycles
        {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("lockin.lpf_sync_average_cycles".to_string()),
                format!(
                    "lockin.lpf_sync_average_cycles must be finite and in (0, {max_sync_cycles}] (got {})",
                    cfg.lockin.lpf_sync_average_cycles
                ),
                None,
            ));
        }
    }
    if cfg.lockin.lpf_kind == LockinLpfKind::SyncIirZeroPhase
        && (cfg.lockin.lpf_iir_order == 0
            || !cfg.lockin.lpf_iir_order.is_multiple_of(2)
            || cfg.lockin.lpf_iir_order > 8)
    {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("lockin.lpf_iir_order".to_string()),
            format!(
                "lockin.lpf_iir_order must be one of 2, 4, 6, or 8 (got {})",
                cfg.lockin.lpf_iir_order
            ),
            None,
        ));
    }
    if uses_explicit_cutoff(cfg.lockin.lpf_kind) {
        if let Some(cutoff_hz) = cfg.lockin.lpf_cutoff_hz
            && (!cutoff_hz.is_finite() || cutoff_hz <= 0.0)
        {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("lockin.lpf_cutoff_hz".to_string()),
                format!("lockin.lpf_cutoff_hz must be positive (got {cutoff_hz})"),
                None,
            ));
        }
        if let Some(cutoff_ratio) = cfg.lockin.lpf_cutoff_ref_ratio
            && (!cutoff_ratio.is_finite() || cutoff_ratio <= 0.0)
        {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("lockin.lpf_cutoff_ref_ratio".to_string()),
                format!("lockin.lpf_cutoff_ref_ratio must be positive (got {cutoff_ratio})"),
                None,
            ));
        }
        if cfg.lockin.lpf_cutoff_hz.is_some() && cfg.lockin.lpf_cutoff_ref_ratio.is_some() {
            errors.push(ConfigDiagnostic::new(
                DiagnosticKind::Validation,
                Some("lockin".to_string()),
                "lockin.lpf_cutoff_hz and lockin.lpf_cutoff_ref_ratio are mutually exclusive for cutoff-based LPF modes",
                None,
            ));
        }
    }
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
    if !seen.contains(&cfg.roles.reference_ch) {
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
    if !cfg.roles.sensor_ch.contains(&cfg.kerr.use_sensor_ch) {
        errors.push(ConfigDiagnostic::new(
            DiagnosticKind::Validation,
            Some("kerr.use_sensor_ch".to_string()),
            format!(
                "kerr.use_sensor_ch ({}) is not included in roles.sensor_ch",
                cfg.kerr.use_sensor_ch
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

    if uses_explicit_cutoff(cfg.lockin.lpf_kind)
        && cfg.lockin.lpf_cutoff_hz.is_none()
        && cfg.lockin.lpf_cutoff_ref_ratio.is_none()
    {
        warnings.push(ConfigWarning::new(
            "lockin.lpf_kind uses an explicit cutoff but no cutoff is specified; runtime will use the compatibility fallback cutoff 0.5 / t_half",
        ));
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
        | ValidationTarget::Phase
        | ValidationTarget::Kerr
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
            validate_reference_roles(cfg)?;
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
        ValidationTarget::Phase => {
            validate_signal_roles(cfg)?;
            validate_sensor_metadata(cfg)?;
            validate_lockin_results_exist(cfg)?;
        }
        ValidationTarget::Kerr => {
            validate_signal_roles(cfg)?;
            validate_sensor_roles(cfg)?;
            validate_sensor_metadata(cfg)?;
            validate_kerr_sensor(cfg)?;
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
        Connection::Gpib { .. } => {
            bail!("DHO5108 display capture does not support GPIB");
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

fn validate_kerr_sensor(cfg: &Config) -> Result<()> {
    if !cfg.roles.sensor_ch.contains(&cfg.kerr.use_sensor_ch) {
        bail!(
            "kerr.use_sensor_ch ({}) must be included in roles.sensor_ch",
            cfg.kerr.use_sensor_ch
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
    let paths = cfg.paths();
    match cfg.fetch.analysis_input {
        FetchAnalysisInput::Csv => validate_raw_csv_exists(cfg),
        FetchAnalysisInput::Raw => validate_raw_metadata_exists(cfg),
        FetchAnalysisInput::Auto => {
            let raw_dir = paths.acquisition_dir();
            let metadata = paths.acquisition_manifest();
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
    let paths = cfg.paths();
    let path = paths.waveform_csv();
    validate_file_exists(&path, &path.display().to_string())
}

fn validate_raw_metadata_exists(cfg: &Config) -> Result<()> {
    let paths = cfg.paths();
    let path = paths.acquisition_manifest();
    validate_file_exists(&path, &path.display().to_string())
}

fn validate_lockin_results_exist(cfg: &Config) -> Result<()> {
    let paths = cfg.paths();
    for ch in cfg.phase_signal_ch() {
        let path = paths.lockin_xy_csv(*ch);
        validate_file_exists(&path, &path.display().to_string())?;
    }
    Ok(())
}

fn validate_rotated_results_exist(cfg: &Config) -> Result<()> {
    let paths = cfg.paths();
    for ch in cfg.phase_signal_ch() {
        let path = paths.lockin_rotated_csv(*ch);
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
