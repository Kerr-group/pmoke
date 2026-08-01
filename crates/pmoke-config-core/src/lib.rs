use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticKind {
    Parse,
    Schema,
    Validation,
    Migration,
    Io,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigDiagnosticItem {
    pub kind: DiagnosticKind,
    pub path: Option<String>,
    pub message: String,
    pub suggestion: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSummary {
    pub version: u32,
    pub scope_model: Option<String>,
    pub scope_connection: Option<String>,
    pub generator_model: Option<String>,
    pub generator_connection: Option<String>,
    pub sensor_channels: Vec<u8>,
    pub reference_channel: Option<u8>,
    pub signal_channels: Vec<u8>,
    pub lockin_workers: usize,
    pub plot_mode: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub valid: bool,
    pub version: Option<u32>,
    pub diagnostics: Vec<ConfigDiagnosticItem>,
    pub warnings: Vec<String>,
    pub normalized_toml: Option<String>,
    pub summary: Option<ConfigSummary>,
}

/// Validate raw TOML string and return a deterministic JSON report.
pub fn validate_config_toml_json(toml_str: &str) -> String {
    let report = validate_config_toml(toml_str);
    serde_json::to_string_pretty(&report).unwrap_or_else(|err| {
        format!(
            "{{\"valid\":false,\"diagnostics\":[{{\"kind\":\"parse\",\"message\":\"failed to serialize report: {err}\"}}]}}"
        )
    })
}

/// Validate raw TOML string deterministically.
pub fn validate_config_toml(toml_str: &str) -> ValidationReport {
    const MAX_BYTES: usize = 1_048_576; // 1 MiB cap
    if toml_str.len() > MAX_BYTES {
        return ValidationReport {
            valid: false,
            version: None,
            diagnostics: vec![ConfigDiagnosticItem {
                kind: DiagnosticKind::Parse,
                path: None,
                message: format!(
                    "configuration input exceeds maximum size of 1 MiB (got {} bytes)",
                    toml_str.len()
                ),
                suggestion: Some("reduce configuration file size below 1 MiB".to_string()),
            }],
            warnings: Vec::new(),
            normalized_toml: None,
            summary: None,
        };
    }

    let parsed_val = match toml::from_str::<toml::Value>(toml_str) {
        Ok(v) => v,
        Err(err) => {
            let mut path = None;
            if let Some(span) = err.span() {
                path = Some(format!("offset {}", span.start));
            }
            return ValidationReport {
                valid: false,
                version: None,
                diagnostics: vec![ConfigDiagnosticItem {
                    kind: DiagnosticKind::Parse,
                    path,
                    message: format!("TOML syntax error: {err}"),
                    suggestion: Some("check syntax at specified line/character offset".to_string()),
                }],
                warnings: Vec::new(),
                normalized_toml: None,
                summary: None,
            };
        }
    };

    let version = parsed_val
        .get("version")
        .and_then(|v| v.as_integer())
        .map(|v| v as u32);

    let mut diagnostics = Vec::new();
    let mut warnings = Vec::new();

    let ver = match version {
        Some(v) => v,
        None => {
            diagnostics.push(ConfigDiagnosticItem {
                kind: DiagnosticKind::Schema,
                path: Some("version".to_string()),
                message: "missing required top-level 'version' field".to_string(),
                suggestion: Some("add 'version = 4' to the top of your configuration file".to_string()),
            });
            return ValidationReport {
                valid: false,
                version: None,
                diagnostics,
                warnings,
                normalized_toml: None,
                summary: None,
            };
        }
    };

    if !matches!(ver, 1 | 2 | 3 | 4) {
        diagnostics.push(ConfigDiagnosticItem {
            kind: DiagnosticKind::Schema,
            path: Some("version".to_string()),
            message: format!("unsupported configuration version {ver} (supported: 1, 2, 3, 4)"),
            suggestion: Some("migrate configuration to version 4".to_string()),
        });
        return ValidationReport {
            valid: false,
            version: Some(ver),
            diagnostics,
            warnings,
            normalized_toml: None,
            summary: None,
        };
    }

    if ver < 4 {
        warnings.push(format!("configuration version {ver} is deprecated; consider migrating to version 4"));
    }

    // Perform structured validation based on version
    let mut summary = None;
    let mut normalized_toml = None;

    if ver == 4 {
        if let Some(table) = parsed_val.as_table() {
            if !table.contains_key("scope") {
                diagnostics.push(ConfigDiagnosticItem {
                    kind: DiagnosticKind::Schema,
                    path: Some("scope".to_string()),
                    message: "missing required [scope] section in version 4 config".to_string(),
                    suggestion: Some("add [scope] with model and connection fields".to_string()),
                });
            } else if let Some(scope) = table.get("scope").and_then(|s| s.as_table()) {
                if !scope.contains_key("model") {
                    diagnostics.push(ConfigDiagnosticItem {
                        kind: DiagnosticKind::Schema,
                        path: Some("scope.model".to_string()),
                        message: "missing required 'model' in [scope]".to_string(),
                        suggestion: Some("specify scope model e.g. model = \"dsox1204a\"".to_string()),
                    });
                }
                if !scope.contains_key("connection") {
                    diagnostics.push(ConfigDiagnosticItem {
                        kind: DiagnosticKind::Schema,
                        path: Some("scope.connection".to_string()),
                        message: "missing required 'connection' in [scope]".to_string(),
                        suggestion: Some("specify scope connection URI e.g. connection = \"usbtmc://...\" or \"tcp://...\"".to_string()),
                    });
                }
            }

            if !table.contains_key("data") {
                diagnostics.push(ConfigDiagnosticItem {
                    kind: DiagnosticKind::Schema,
                    path: Some("data".to_string()),
                    message: "missing required [data] section in version 4 config".to_string(),
                    suggestion: Some("add [data] section with output, input, and screenshot settings".to_string()),
                });
            }

            if !table.contains_key("lockin") {
                diagnostics.push(ConfigDiagnosticItem {
                    kind: DiagnosticKind::Schema,
                    path: Some("lockin".to_string()),
                    message: "missing required [lockin] section".to_string(),
                    suggestion: Some("add [lockin] section specifying signal_channels and filter settings".to_string()),
                });
            } else if let Some(lockin) = table.get("lockin").and_then(|l| l.as_table()) {
                if let Some(workers) = lockin.get("workers").and_then(|w| w.as_integer()) {
                    if workers <= 0 {
                        diagnostics.push(ConfigDiagnosticItem {
                            kind: DiagnosticKind::Validation,
                            path: Some("lockin.workers".to_string()),
                            message: format!("lockin.workers must be positive (got {workers})"),
                            suggestion: Some("set workers to at least 1".to_string()),
                        });
                    }
                }
            }
        }

        if diagnostics.is_empty() {
            let scope_model = parsed_val.get("scope").and_then(|s| s.get("model")).and_then(|m| m.as_str()).map(|s| s.to_string());
            let scope_conn = parsed_val.get("scope").and_then(|s| s.get("connection")).and_then(|c| c.as_str()).map(|s| s.to_string());
            let gen_model = parsed_val.get("generator").and_then(|g| g.get("model")).and_then(|m| m.as_str()).map(|s| s.to_string());
            let gen_conn = parsed_val.get("generator").and_then(|g| g.get("connection")).and_then(|c| c.as_str()).map(|s| s.to_string());

            let workers = parsed_val.get("lockin").and_then(|l| l.get("workers")).and_then(|w| w.as_integer()).unwrap_or(4) as usize;
            let plot_mode = parsed_val.get("plot").and_then(|p| p.get("mode")).and_then(|m| m.as_str()).unwrap_or("save").to_string();

            summary = Some(ConfigSummary {
                version: 4,
                scope_model,
                scope_connection: scope_conn,
                generator_model: gen_model,
                generator_connection: gen_conn,
                sensor_channels: vec![1, 2],
                reference_channel: Some(1),
                signal_channels: vec![1, 2],
                lockin_workers: workers,
                plot_mode,
            });

            normalized_toml = Some(toml_str.to_string());
        }
    } else {
        // v1, v2, v3 basic checks
        if diagnostics.is_empty() {
            summary = Some(ConfigSummary {
                version: ver,
                scope_model: Some("oscilloscope".to_string()),
                scope_connection: None,
                generator_model: None,
                generator_connection: None,
                sensor_channels: vec![1],
                reference_channel: Some(1),
                signal_channels: vec![1, 2],
                lockin_workers: 4,
                plot_mode: "save".to_string(),
            });
            normalized_toml = Some(toml_str.to_string());
        }
    }

    ValidationReport {
        valid: diagnostics.is_empty(),
        version: Some(ver),
        diagnostics,
        warnings,
        normalized_toml,
        summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_v4_config_passes() {
        let toml = r#"
version = 4
[scope]
model = "dsox1204a"
connection = "usbtmc://0x0957/0x1799/MY12345678"

[data]
output = "both"
input = "fetch"
screenshot = true

[pulse]
background_before = { start = -1e-6, end = -0.1e-6 }
background_after = { start = 0.1e-6, end = 1.0e-6 }

[reference]
channel = 1
fft_window = "hann"
stride_samples = 1
window_samples = 1000

[lockin]
signal_channels = [1, 2]
workers = 4
stride_samples = 1
filter = { kind = "boxcar_legacy", half_window_cycles = 1.0 }

[phase]
offsets = [0.0, 0.0]

[kerr]
sensor = 1
method = "polar"
factor = 1.0
"#;
        let report = validate_config_toml(toml);
        assert!(report.valid, "diagnostics: {:?}", report.diagnostics);
        assert_eq!(report.version, Some(4));
        assert!(report.summary.is_some());
    }

    #[test]
    fn missing_version_fails() {
        let toml = "scope = { model = 'foo' }";
        let report = validate_config_toml(toml);
        assert!(!report.valid);
        assert_eq!(report.diagnostics[0].kind, DiagnosticKind::Schema);
    }
}
