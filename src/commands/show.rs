use crate::config::{Config, ConfigDiagnostics, ConfigLoad, ConfigWarning};
use crate::ui;
use anyhow::Result;
use comfy_table::{
    modifiers::UTF8_ROUND_CORNERS, presets::UTF8_FULL, Attribute, Cell, Color, ContentArrangement,
    Table,
};

pub fn show(load: &ConfigLoad) -> Result<()> {
    match load {
        ConfigLoad::Ready { config, warnings } => {
            print_warnings(warnings);
            print_config_summary(config);
            let rendered = toml::to_string_pretty(config)?;
            ui::section_err("Normalized Config");
            println!("{rendered}");
        }
        ConfigLoad::Diagnostics(diag) => {
            print_diagnostics(diag);
            if let Some(config) = &diag.normalized {
                println!();
                println!("# Normalized Config");
                let rendered = toml::to_string_pretty(config)?;
                println!("{rendered}");
            }
        }
    }

    Ok(())
}

fn styled_table() -> Table {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .apply_modifier(UTF8_ROUND_CORNERS)
        .set_content_arrangement(ContentArrangement::Dynamic);
    table
}

fn header_cell(label: &str) -> Cell {
    Cell::new(label)
        .fg(Color::Cyan)
        .add_attribute(Attribute::Bold)
}

fn missing(value: Option<&str>) -> String {
    value.unwrap_or("-").to_string()
}

fn print_config_summary(config: &Config) {
    ui::section_err("Config Summary");

    let mut summary = styled_table();
    summary.set_header(vec![header_cell("Item"), header_cell("Value")]);
    summary.add_row(vec!["Version".to_string(), config.version.to_string()]);
    summary.add_row(vec![
        "Oscilloscope".to_string(),
        config
            .instruments
            .as_ref()
            .map(|inst| {
                format!(
                    "{} / {:?} / {} samples",
                    inst.oscilloscope.model,
                    inst.oscilloscope.connection,
                    inst.oscilloscope.memory_depth
                )
            })
            .unwrap_or_else(|| "not configured".to_string()),
    ]);
    summary.add_row(vec![
        "Function generator".to_string(),
        config
            .instruments
            .as_ref()
            .and_then(|inst| inst.function_generator.as_ref())
            .map(|fg| format!("{} / {:?}", fg.model, fg.connection))
            .unwrap_or_else(|| "not configured".to_string()),
    ]);
    summary.add_row(vec![
        "Roles".to_string(),
        format!(
            "sensor={:?}, reference=ch{}, signal={:?}",
            config.roles.sensor_ch, config.roles.reference_ch, config.roles.signal_ch
        ),
    ]);
    summary.add_row(vec![
        "Lock-in".to_string(),
        format!(
            "{:?}, workers={}, stride={}, debug={}",
            config.lockin.lpf_kind,
            config.lockin.workers,
            config.lockin.stride_samples,
            config.lockin.lpf_debug_output
        ),
    ]);
    summary.add_row(vec![
        "Kerr".to_string(),
        format!(
            "{:?}, sensor=ch{}, factor={}",
            config.kerr.kerr_type, config.kerr.use_sensor_ch, config.kerr.factor
        ),
    ]);
    eprintln!("{summary}");

    let mut channels = styled_table();
    channels.set_header(vec![
        header_cell("Channel"),
        header_cell("Role"),
        header_cell("Label"),
        header_cell("Unit"),
        header_cell("Factor"),
    ]);
    for channel in &config.channels {
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

        channels.add_row(vec![
            format!("ch{}", channel.index),
            if roles.is_empty() {
                "-".to_string()
            } else {
                roles.join(", ")
            },
            missing(channel.label.as_deref()),
            missing(channel.unit_out.as_deref()),
            channel
                .factor
                .map(|factor| factor.to_string())
                .unwrap_or_else(|| "-".to_string()),
        ]);
    }
    eprintln!("{channels}");
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
        let mut table = styled_table();
        table.set_header(vec![header_cell("Message")]);
        for warning in &diag.warnings {
            table.add_row(vec![warning.message.clone()]);
        }
        println!("{table}");
    }

    if !diag.diagnostics.is_empty() {
        ui::section("Diagnostics");
        let mut table = styled_table();
        table.set_header(vec![
            header_cell("Kind"),
            header_cell("Path"),
            header_cell("Message"),
            header_cell("Suggestion"),
        ]);
        for issue in &diag.diagnostics {
            table.add_row(vec![
                issue.kind.to_string(),
                missing(issue.path.as_deref()),
                issue.message.clone(),
                missing(issue.suggestion.as_deref()),
            ]);
        }
        println!("{table}");
    }
}
