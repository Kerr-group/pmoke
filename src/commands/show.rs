use crate::config::{
    Channel, Config, ConfigDiagnostics, ConfigLoad, ConfigWarning, render_normalized_config,
};
use crate::ui;
use anyhow::Result;

pub fn show(load: &ConfigLoad) -> Result<()> {
    match load {
        ConfigLoad::Ready { config, warnings } => {
            print_warnings(warnings);
            print_config_summary(config);
            let rendered = render_normalized_config(config)?;
            ui::section("Normalized Config");
            println!("{rendered}");
        }
        ConfigLoad::Diagnostics(diag) => {
            print_diagnostics(diag);
            if let Some(config) = &diag.normalized {
                println!();
                println!("# Normalized Config");
                let rendered = render_normalized_config(config)?;
                println!("{rendered}");
            }
        }
    }

    Ok(())
}

fn missing(value: Option<&str>) -> String {
    value.unwrap_or("-").to_string()
}

fn print_config_summary(config: &Config) {
    ui::summary_table(
        "Config Summary",
        &["Item", "Value"],
        vec![
            vec!["Version".to_string(), config.version.to_string()],
            vec![
                "Oscilloscope".to_string(),
                config
                    .instruments
                    .as_ref()
                    .map(|inst| {
                        format!(
                            "{} / {:?}",
                            inst.oscilloscope.model, inst.oscilloscope.connection
                        )
                    })
                    .unwrap_or_else(|| "not configured".to_string()),
            ],
            vec![
                "Function generator".to_string(),
                config
                    .instruments
                    .as_ref()
                    .and_then(|inst| inst.function_generator.as_ref())
                    .map(|fg| format!("{} / {:?}", fg.model, fg.connection))
                    .unwrap_or_else(|| "not configured".to_string()),
            ],
            vec![
                "Roles".to_string(),
                format!(
                    "sensor={:?}, reference=ch{}, signal={:?}",
                    config.roles.sensor_ch, config.roles.reference_ch, config.roles.signal_ch
                ),
            ],
            vec![
                "Lock-in".to_string(),
                format!(
                    "{:?}, workers={}, stride={}, debug={}",
                    config.lockin.lpf_kind,
                    config.lockin.workers,
                    config.lockin.stride_samples,
                    config.lockin.lpf_debug_output
                ),
            ],
            vec![
                "Kerr".to_string(),
                format!(
                    "{:?}, sensor=ch{}, factor={}",
                    config.kerr.kerr_type, config.kerr.use_sensor_ch, config.kerr.factor
                ),
            ],
        ],
    );

    ui::settings_table(
        "Channels",
        config
            .channels
            .iter()
            .map(|channel| channel_row(config, channel))
            .collect(),
    );
}

fn channel_row(config: &Config, channel: &Channel) -> (String, String) {
    let mut roles = Vec::new();
    if config.roles.sensor_ch.contains(&channel.index) {
        roles.push("sensor");
    }
    if config.roles.reference_ch == channel.index {
        roles.push("reference");
    }
    if config.roles.signal_ch.contains(&channel.index) {
        roles.push("signal");
    }

    let role = if roles.is_empty() {
        "-".to_string()
    } else {
        roles.join(", ")
    };
    let scale = match (channel.factor, channel.scale_to_abs_max) {
        (Some(factor), None) => format!("factor={factor}"),
        (None, Some(target)) => format!(
            "max_abs={}, polarity={}",
            target.abs(),
            if target.is_sign_negative() { -1 } else { 1 }
        ),
        _ => "scale=-".to_string(),
    };

    (
        format!("ch{}", channel.index),
        format!(
            "role={}, label={}, unit={}, {}",
            role,
            missing(channel.label.as_deref()),
            missing(channel.unit_out.as_deref()),
            scale
        ),
    )
}

pub fn print_warnings(warnings: &[ConfigWarning]) {
    for warning in warnings {
        ui::warn(&warning.message);
    }
}

fn print_diagnostics(diag: &ConfigDiagnostics) {
    match diag.version {
        Some(version) => println!("Config version: {version}"),
        None => println!("Config version: <unavailable>"),
    }

    if !diag.warnings.is_empty() {
        ui::section("Warnings");
        let table = ui::table(
            &["Message"],
            diag.warnings
                .iter()
                .map(|warning| vec![warning.message.clone()])
                .collect(),
        );
        println!("{table}");
    }

    if !diag.diagnostics.is_empty() {
        ui::section("Diagnostics");
        let table = ui::table(
            &["Kind", "Path", "Message", "Suggestion"],
            diag.diagnostics
                .iter()
                .map(|issue| {
                    vec![
                        issue.kind.to_string(),
                        missing(issue.path.as_deref()),
                        issue.message.clone(),
                        missing(issue.suggestion.as_deref()),
                    ]
                })
                .collect(),
        );
        println!("{table}");
    }
}
