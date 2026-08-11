use super::*;
use crate::connection::ConnectionUri;
use crate::constants::{FETCHED_FNAME, RAW_METADATA_FNAME, RAW_WAVEFORM_DIR};
use anyhow::Context;

pub fn render_normalized_config(config: &Config) -> Result<String> {
    if config.version >= 4 && config.instruments.is_none() {
        return Err(anyhow!(
            "current-schema normalized config has no oscilloscope configuration"
        ));
    }
    let can_render_v5 = config.instruments.is_some()
        && (config.version >= 4 || !legacy_timebase_is_required(config)?);
    if can_render_v5 {
        render_config_v5(config)
    } else {
        render_legacy_resolved_config(config)
    }
}

fn render_legacy_resolved_config(config: &Config) -> Result<String> {
    let mut value = toml::Value::try_from(config).context("failed to encode legacy config")?;
    let table = value
        .as_table_mut()
        .ok_or_else(|| anyhow!("normalized config did not encode as a TOML table"))?;
    if let Some(timebase) = &config.legacy_timebase {
        table.insert(
            "timebase".to_string(),
            toml::Value::try_from(timebase).context("failed to encode legacy timebase")?,
        );
    }
    toml::to_string_pretty(&value).map_err(Into::into)
}

pub(super) fn render_config_v4(config: &Config) -> Result<String> {
    toml::to_string_pretty(&normalized_config_v4(config)?).map_err(Into::into)
}

pub(super) fn render_config_v5(config: &Config) -> Result<String> {
    toml::to_string_pretty(&normalized_config_v5(config)?).map_err(Into::into)
}

fn normalized_config_v4(config: &Config) -> Result<NormalizedConfigV4> {
    let instruments = config
        .instruments
        .as_ref()
        .ok_or_else(|| anyhow!("version 4 normalized config has no oscilloscope"))?;
    let scope = ScopeOutputV4 {
        model: instruments.oscilloscope.model.clone(),
        connection: connection_uri(&instruments.oscilloscope.connection),
    };
    let generator = instruments
        .function_generator
        .as_ref()
        .map(|generator| GeneratorOutputV4 {
            model: generator.model.clone(),
            connection: connection_uri(&generator.connection),
        });
    let sensors = config
        .roles
        .sensor_ch
        .iter()
        .map(|&channel| sensor_output_v4(config, channel))
        .collect::<Result<Vec<_>>>()?;

    Ok(NormalizedConfigV4 {
        version: 4,
        scope,
        generator,
        data: DataOutputConfigV4 {
            output: match config.fetch.output {
                FetchOutput::Csv => DataOutputV4::Csv,
                FetchOutput::Raw => DataOutputV4::Raw,
                FetchOutput::CsvAndRaw => DataOutputV4::Both,
            },
            input: config.fetch.analysis_input,
            screenshot: config.screenshot.enabled,
        },
        sensors,
        pulse: PulseOutputV4 {
            background_before: config.pulse.bg_window_before,
            background_after: config.pulse.bg_window_after,
        },
        reference: ReferenceOutputV4 {
            channel: config.roles.reference_ch,
            fft_window: config.reference.fft_window,
            stride_samples: config.reference.stride_samples,
            window_samples: config.reference.window_samples,
        },
        lockin: lockin_output_v4(&config.lockin, &config.roles.signal_ch),
        phase: PhaseOutputV4 {
            offsets: config.phase.m_omega_t0_offset.clone(),
        },
        kerr: KerrOutputV4 {
            sensor: config.kerr.use_sensor_ch,
            method: config.kerr.kerr_type,
            factor: config.kerr.factor,
        },
        plot: plot_output_v4(&config.plot),
    })
}

fn normalized_config_v5(config: &Config) -> Result<NormalizedConfigV5> {
    let instruments = config
        .instruments
        .as_ref()
        .ok_or_else(|| anyhow!("version 5 normalized config has no oscilloscope"))?;
    let scope = ScopeOutputV4 {
        model: instruments.oscilloscope.model.clone(),
        connection: connection_uri(&instruments.oscilloscope.connection),
    };
    let generator = instruments
        .function_generator
        .as_ref()
        .map(|generator| GeneratorOutputV4 {
            model: generator.model.clone(),
            connection: connection_uri(&generator.connection),
        });
    let sensors = config
        .roles
        .sensor_ch
        .iter()
        .map(|&channel| sensor_output_v4(config, channel))
        .collect::<Result<Vec<_>>>()?;

    Ok(NormalizedConfigV5 {
        version: 5,
        scope,
        generator,
        data: DataOutputConfigV4 {
            output: match config.fetch.output {
                FetchOutput::Csv => DataOutputV4::Csv,
                FetchOutput::Raw => DataOutputV4::Raw,
                FetchOutput::CsvAndRaw => DataOutputV4::Both,
            },
            input: config.fetch.analysis_input,
            screenshot: config.screenshot.enabled,
        },
        sensors,
        pulse: PulseOutputV4 {
            background_before: config.pulse.bg_window_before,
            background_after: config.pulse.bg_window_after,
        },
        reference: ReferenceOutputV4 {
            channel: config.roles.reference_ch,
            fft_window: config.reference.fft_window,
            stride_samples: config.reference.stride_samples,
            window_samples: config.reference.window_samples,
        },
        lockin: lockin_output_v5(&config.lockin, &config.roles.signal_ch),
        phase: PhaseOutputV4 {
            offsets: config.phase.m_omega_t0_offset.clone(),
        },
        kerr: KerrOutputV4 {
            sensor: config.kerr.use_sensor_ch,
            method: config.kerr.kerr_type,
            factor: config.kerr.factor,
        },
        plot: plot_output_v4(&config.plot),
    })
}

fn sensor_output_v4(config: &Config, index: u8) -> Result<SensorOutputV4> {
    let channel = config
        .channels
        .iter()
        .find(|channel| channel.index == index)
        .ok_or_else(|| anyhow!("version 4 sensor channel {index} is not defined"))?;
    let scale = match (channel.factor, channel.scale_to_abs_max) {
        (Some(factor), None) => SensorScaleOutputV4::Factor { factor },
        (None, Some(target)) => SensorScaleOutputV4::MaxAbs {
            max_abs: target.abs(),
            polarity: if target.is_sign_negative() { -1 } else { 1 },
        },
        _ => bail!("version 4 sensor channel {index} has an invalid scale"),
    };
    Ok(SensorOutputV4 {
        channel: index,
        scale,
        label: channel
            .label
            .clone()
            .ok_or_else(|| anyhow!("version 4 sensor channel {index} has no label"))?,
        unit: channel
            .unit_out
            .clone()
            .ok_or_else(|| anyhow!("version 4 sensor channel {index} has no unit"))?,
    })
}

pub(crate) fn connection_uri(connection: &Connection) -> String {
    let uri = match connection {
        Connection::Tcpip { ip, port } => ConnectionUri::Tcp {
            host: ip.clone(),
            port: *port,
        },
        Connection::Usbtmc { resource } => ConnectionUri::Visa {
            resource: resource.clone(),
        },
        Connection::Gpib { board, address } => ConnectionUri::Gpib {
            board: *board,
            address: *address,
        },
        Connection::PrologixTcp {
            host,
            port,
            address,
            read_timeout_ms,
        } => ConnectionUri::PrologixTcp {
            host: host.clone(),
            port: *port,
            address: *address,
            read_timeout_ms: *read_timeout_ms,
        },
        Connection::PrologixSerial {
            path,
            address,
            baud_rate,
            read_timeout_ms,
        } => ConnectionUri::PrologixSerial {
            path: path.clone(),
            address: *address,
            baud_rate: *baud_rate,
            read_timeout_ms: *read_timeout_ms,
        },
    };
    uri.to_string()
}

fn lockin_output_v4(lockin: &Lockin, signal_channels: &[u8]) -> LockinOutputV4 {
    let filter = match lockin.lpf_kind {
        LockinLpfKind::BoxcarLegacy => LockinFilterOutputV4::BoxcarLegacy {
            half_window_cycles: lockin.lpf_half_window_cycles,
        },
    };
    LockinOutputV4 {
        signal_channels: signal_channels.to_vec(),
        workers: lockin.workers,
        stride_samples: lockin.stride_samples,
        filter,
        debug_output: lockin.lpf_debug_output,
        debug_label: lockin.lpf_debug_label.clone(),
        debug_overwrite: lockin.lpf_debug_overwrite,
        snr_background_window: lockin.snr_background_window,
        snr_signal_window: lockin.snr_signal_window,
        save_npy: lockin.save_npy,
    }
}

fn lockin_output_v5(lockin: &Lockin, signal_channels: &[u8]) -> LockinOutputV5 {
    let filter = match lockin.lpf_kind {
        LockinLpfKind::BoxcarLegacy => LockinFilterOutputV5::BoxcarLegacy {
            half_window_cycles: lockin.lpf_half_window_cycles,
        },
    };
    LockinOutputV5 {
        signal_channels: signal_channels.to_vec(),
        workers: lockin.workers,
        stride_samples: lockin.stride_samples,
        filter,
        debug_output: lockin.lpf_debug_output,
        debug_label: lockin.lpf_debug_label.clone(),
        debug_overwrite: lockin.lpf_debug_overwrite,
        snr_background_window: lockin.snr_background_window,
        snr_signal_window: lockin.snr_signal_window,
        save_npy: lockin.save_npy,
    }
}

fn legacy_timebase_is_required(config: &Config) -> Result<bool> {
    if config.legacy_timebase.is_none() {
        return Ok(false);
    }
    match config.fetch.analysis_input {
        FetchAnalysisInput::Raw => Ok(false),
        FetchAnalysisInput::Csv => Ok(!csv_has_recorded_time(
            &config.artifact_path(FETCHED_FNAME),
        )?),
        FetchAnalysisInput::Auto => {
            let metadata = config
                .artifact_path(RAW_WAVEFORM_DIR)
                .join(RAW_METADATA_FNAME);
            if metadata.exists() {
                Ok(false)
            } else {
                Ok(!csv_has_recorded_time(
                    &config.artifact_path(FETCHED_FNAME),
                )?)
            }
        }
    }
}

fn csv_has_recorded_time(path: &Path) -> Result<bool> {
    if !path.exists() {
        return Ok(false);
    }
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(path)
        .map_err(|error| {
            anyhow!(
                "failed to inspect CSV time column: {}: {error}",
                path.display()
            )
        })?;
    let headers = reader
        .headers()
        .map_err(|error| anyhow!("failed to read CSV header: {}: {error}", path.display()))?;
    Ok(headers.iter().any(|header| {
        matches!(
            header.trim().to_ascii_lowercase().as_str(),
            "time" | "time (s)" | "t" | "t (s)"
        )
    }))
}

fn plot_output_v4(plot: &Plot) -> PlotOutputV4 {
    let mode = match (plot.enabled, plot.save, plot.interactive) {
        (false, _, _) => PlotModeV4::Off,
        (true, true, true) => PlotModeV4::Both,
        (true, false, true) => PlotModeV4::Interactive,
        (true, true, false) => PlotModeV4::Save,
        (true, false, false) => PlotModeV4::Off,
    };
    PlotOutputV4 {
        mode,
        max_points: plot.max_points,
        decimation: plot.decimation,
        on_error: if plot.fail_on_error {
            PlotErrorModeV4::Fail
        } else {
            PlotErrorModeV4::Warn
        },
    }
}
