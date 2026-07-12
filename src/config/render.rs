use super::*;

pub fn render_normalized_config(config: &Config) -> Result<String> {
    if config.version == 4 {
        render_config_v4(config)
    } else {
        toml::to_string_pretty(config).map_err(Into::into)
    }
}

pub(super) fn render_config_v4(config: &Config) -> Result<String> {
    toml::to_string_pretty(&normalized_config_v4(config)?).map_err(Into::into)
}

fn normalized_config_v4(config: &Config) -> Result<NormalizedConfigV4> {
    let instruments = config
        .instruments
        .as_ref()
        .ok_or_else(|| anyhow!("version 4 normalized config has no oscilloscope"))?;
    let scope = ScopeOutputV4 {
        model: instruments.oscilloscope.model.clone(),
        connection: connection_string_v4(&instruments.oscilloscope.connection),
    };
    let generator = instruments
        .function_generator
        .as_ref()
        .map(|generator| GeneratorOutputV4 {
            model: generator.model.clone(),
            connection: connection_string_v4(&generator.connection),
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

fn connection_string_v4(connection: &Connection) -> String {
    match connection {
        Connection::Tcpip { ip, port } if ip.contains(':') => format!("tcp://[{ip}]:{port}"),
        Connection::Tcpip { ip, port } => format!("tcp://{ip}:{port}"),
        Connection::Usbtmc { resource } => format!("visa:{resource}"),
        Connection::Gpib { board, address } => format!("gpib://{board}/{address}"),
    }
}

fn lockin_output_v4(lockin: &Lockin, signal_channels: &[u8]) -> LockinOutputV4 {
    let filter = match lockin.lpf_kind {
        LockinLpfKind::BoxcarLegacy => LockinFilterOutputV4::BoxcarLegacy {
            half_window_cycles: lockin.lpf_half_window_cycles,
        },
        LockinLpfKind::FirBoxcarEnbw => LockinFilterOutputV4::FirBoxcarEnbw {
            half_window_cycles: lockin.lpf_half_window_cycles,
        },
        LockinLpfKind::FirZeroPhase => LockinFilterOutputV4::FirZeroPhase {
            half_window_cycles: lockin.lpf_half_window_cycles,
            cutoff_hz: lockin.lpf_cutoff_hz,
            cutoff_ref_ratio: lockin.lpf_cutoff_ref_ratio,
            stopband_atten_db: lockin.lpf_stopband_atten_db,
        },
        LockinLpfKind::SyncIirZeroPhase => LockinFilterOutputV4::SyncIirZeroPhase {
            half_window_cycles: lockin.lpf_half_window_cycles,
            cutoff_hz: lockin.lpf_cutoff_hz,
            cutoff_ref_ratio: lockin.lpf_cutoff_ref_ratio,
            sync_average_cycles: lockin.lpf_sync_average_cycles,
            iir_order: lockin.lpf_iir_order,
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
    }
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
