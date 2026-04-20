use crate::config::{ConfigDiagnostics, ConfigLoad, ConfigWarning};
use anyhow::Result;

pub fn show(load: &ConfigLoad) -> Result<()> {
    match load {
        ConfigLoad::Ready { config, warnings } => {
            print_warnings(warnings);
            let rendered = toml::to_string_pretty(config)?;
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

pub fn print_warnings(warnings: &[ConfigWarning]) {
    for warning in warnings {
        eprintln!("⚠️ {}", warning.message);
    }
}

fn print_diagnostics(diag: &ConfigDiagnostics) {
    match diag.version {
        Some(version) => println!("Config version: {version}"),
        None => println!("Config version: <unavailable>"),
    }

    if !diag.warnings.is_empty() {
        println!();
        println!("Warnings:");
        for warning in &diag.warnings {
            println!("- {}", warning.message);
        }
    }

    if !diag.diagnostics.is_empty() {
        println!();
        println!("Diagnostics:");
        for issue in &diag.diagnostics {
            let path = issue
                .path
                .as_deref()
                .map(|p| format!(" [{p}]"))
                .unwrap_or_default();
            println!("- {}{}: {}", issue.kind, path, issue.message);
            if let Some(suggestion) = &issue.suggestion {
                println!("  suggestion: {suggestion}");
            }
        }
    }
}
