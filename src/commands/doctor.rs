use crate::config::{Config, ConfigDiagnostics, ConfigWarning, Connection, connection_uri};

use crate::ui;
use anyhow::{Context, Result, bail};
#[cfg(feature = "hw-core")]
use instruments::registry::{InstrumentCapability, TransportDiagnosticCapability};
use instruments::registry::{InstrumentRole, InstrumentSpec, TransportKind, find_instrument};
use pyo3::Python;
use pyo3::types::{PyAnyMethods, PyModule};
use serde::Serialize;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum CheckStatus {
    Pass,
    Warn,
    Skip,
    Fail,
}

impl CheckStatus {
    fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Warn => "WARN",
            Self::Skip => "SKIP",
            Self::Fail => "FAIL",
        }
    }
}

#[derive(Debug, Serialize)]
struct DoctorCheck {
    name: String,
    status: CheckStatus,
    detail: String,
}

#[derive(Debug, Serialize)]
struct DoctorReport {
    schema_version: u32,
    checks: Vec<DoctorCheck>,
}

pub fn run(cfg: &Config, warnings: &[ConfigWarning], json: bool, probe_fetch: bool) -> Result<()> {
    let mut checks = Vec::new();
    checks.push(DoctorCheck {
        name: "config".to_string(),
        status: CheckStatus::Pass,
        detail: format!("schema v{} ({})", cfg.version, cfg.source_path.display()),
    });
    checks.extend(warnings.iter().map(|warning| DoctorCheck {
        name: "config.warning".to_string(),
        status: CheckStatus::Warn,
        detail: warning.message.clone(),
    }));
    let free_bytes = check_storage(cfg, &mut checks);
    check_python(cfg, &mut checks);
    let predicted_bytes = check_hardware(cfg, probe_fetch, &mut checks);
    check_capacity(free_bytes, predicted_bytes, &mut checks);

    emit_report(checks, json)
}

pub fn run_diagnostics(diagnostics: &ConfigDiagnostics, json: bool) -> Result<()> {
    let mut checks = diagnostics
        .warnings
        .iter()
        .map(|warning| DoctorCheck {
            name: "config.warning".to_string(),
            status: CheckStatus::Warn,
            detail: warning.message.clone(),
        })
        .collect::<Vec<_>>();
    checks.extend(diagnostics.diagnostics.iter().map(|diagnostic| {
        DoctorCheck {
            name: diagnostic
                .path
                .as_ref()
                .map_or_else(|| "config".to_string(), |path| format!("config.{path}")),
            status: CheckStatus::Fail,
            detail: match &diagnostic.suggestion {
                Some(suggestion) => format!(
                    "{}: {}; suggestion: {suggestion}",
                    diagnostic.kind, diagnostic.message
                ),
                None => format!("{}: {}", diagnostic.kind, diagnostic.message),
            },
        }
    }));
    emit_report(checks, json)
}

fn emit_report(checks: Vec<DoctorCheck>, json: bool) -> Result<()> {
    let failed = checks.iter().any(|check| check.status == CheckStatus::Fail);
    let report = DoctorReport {
        schema_version: 1,
        checks,
    };
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&report).context("failed to encode doctor report")?
        );
    } else {
        ui::settings_table(
            "Doctor",
            report
                .checks
                .iter()
                .map(|check| {
                    (
                        check.name.clone(),
                        format!("{}  {}", check.status.label(), check.detail),
                    )
                })
                .collect(),
        );
    }
    if failed {
        bail!("doctor found required checks that failed");
    }
    if !json {
        ui::success("doctor checks completed");
    }
    Ok(())
}

fn check_storage(cfg: &Config, checks: &mut Vec<DoctorCheck>) -> Option<u64> {
    let raw_path = cfg.paths().acquisition_dir();
    let parent = raw_path.parent().unwrap_or_else(|| Path::new("."));
    let probe_parent = match existing_storage_ancestor(parent) {
        Ok(probe_parent) => probe_parent,
        Err(error) => {
            checks.push(DoctorCheck {
                name: "storage.write".to_string(),
                status: CheckStatus::Fail,
                detail: format!("{error:#}"),
            });
            return None;
        }
    };
    match writable_probe(&probe_parent) {
        Ok(()) => checks.push(DoctorCheck {
            name: "storage.write".to_string(),
            status: CheckStatus::Pass,
            detail: if probe_parent == parent {
                parent.display().to_string()
            } else {
                format!(
                    "{} will be created under writable {}",
                    parent.display(),
                    probe_parent.display()
                )
            },
        }),
        Err(error) => checks.push(DoctorCheck {
            name: "storage.write".to_string(),
            status: CheckStatus::Fail,
            detail: format!("{error:#}"),
        }),
    }
    let free_bytes = match fs2::available_space(&probe_parent) {
        Ok(bytes) => {
            checks.push(DoctorCheck {
                name: "storage.free".to_string(),
                status: CheckStatus::Pass,
                detail: format!("{:.2} GiB", bytes as f64 / 1024.0_f64.powi(3)),
            });
            Some(bytes)
        }
        Err(error) => {
            checks.push(DoctorCheck {
                name: "storage.free".to_string(),
                status: CheckStatus::Warn,
                detail: error.to_string(),
            });
            None
        }
    };
    let staging = staging_path(&raw_path);
    checks.push(if staging.exists() {
        DoctorCheck {
            name: "storage.staging".to_string(),
            status: CheckStatus::Warn,
            detail: format!("incomplete acquisition exists: {}", staging.display()),
        }
    } else {
        DoctorCheck {
            name: "storage.staging".to_string(),
            status: CheckStatus::Pass,
            detail: "none".to_string(),
        }
    });
    free_bytes
}

fn existing_storage_ancestor(path: &Path) -> Result<PathBuf> {
    let mut candidate = if path.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        path.to_path_buf()
    };
    loop {
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) if metadata.file_type().is_dir() => return Ok(candidate),
            Ok(_) => bail!(
                "storage path ancestor is not a regular directory: {}",
                candidate.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let Some(parent) = candidate.parent() else {
                    bail!("no existing storage ancestor found for {}", path.display());
                };
                candidate = if parent.as_os_str().is_empty() {
                    PathBuf::from(".")
                } else {
                    parent.to_path_buf()
                };
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("failed to inspect storage path: {}", candidate.display())
                });
            }
        }
    }
}

fn check_capacity(
    free_bytes: Option<u64>,
    predicted_bytes: Option<u64>,
    checks: &mut Vec<DoctorCheck>,
) {
    let (Some(free_bytes), Some(predicted_bytes)) = (free_bytes, predicted_bytes) else {
        checks.push(DoctorCheck {
            name: "storage.capacity".to_string(),
            status: CheckStatus::Skip,
            detail: "free space or acquisition size is unavailable".to_string(),
        });
        return;
    };
    let margin = predicted_bytes / 20 + 64 * 1024 * 1024;
    let recommended = predicted_bytes.saturating_add(margin);
    let (status, detail) = if free_bytes < predicted_bytes {
        (
            CheckStatus::Fail,
            format!(
                "predicted RAW {:.2} GiB exceeds free space {:.2} GiB",
                gibibytes(predicted_bytes),
                gibibytes(free_bytes)
            ),
        )
    } else if free_bytes < recommended {
        (
            CheckStatus::Warn,
            format!(
                "free {:.2} GiB covers RAW {:.2} GiB but is below the recommended {:.2} GiB",
                gibibytes(free_bytes),
                gibibytes(predicted_bytes),
                gibibytes(recommended)
            ),
        )
    } else {
        (
            CheckStatus::Pass,
            format!(
                "free {:.2} GiB, predicted RAW {:.2} GiB",
                gibibytes(free_bytes),
                gibibytes(predicted_bytes)
            ),
        )
    };
    checks.push(DoctorCheck {
        name: "storage.capacity".to_string(),
        status,
        detail,
    });
}

fn gibibytes(bytes: u64) -> f64 {
    bytes as f64 / 1024.0_f64.powi(3)
}

fn writable_probe(parent: &Path) -> Result<()> {
    if !parent.is_dir() {
        bail!("output parent does not exist: {}", parent.display());
    }
    let path = parent.join(format!(".pmoke-doctor-{}.tmp", std::process::id()));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .with_context(|| format!("output parent is not writable: {}", parent.display()))?;
    drop(file);
    fs::remove_file(&path)
        .with_context(|| format!("failed to remove doctor probe: {}", path.display()))
}

fn staging_path(output: &Path) -> PathBuf {
    let parent = output.parent().unwrap_or_else(|| Path::new(""));
    let mut name = OsString::from(".");
    name.push(output.file_name().unwrap_or_default());
    name.push(".tmp");
    parent.join(name)
}

fn check_python(cfg: &Config, checks: &mut Vec<DoctorCheck>) {
    Python::attach(|py| {
        for module in ["numpy", "scipy", "lmfit", "gsplot"] {
            checks.push(match PyModule::import(py, module) {
                Ok(_) => DoctorCheck {
                    name: format!("python.{module}"),
                    status: CheckStatus::Pass,
                    detail: "imported".to_string(),
                },
                Err(error) => DoctorCheck {
                    name: format!("python.{module}"),
                    status: CheckStatus::Fail,
                    detail: error.to_string(),
                },
            });
        }
        if cfg.plot.enabled {
            checks.push(
                match PyModule::import(py, "matplotlib")
                    .and_then(|module| module.call_method0("get_backend"))
                    .and_then(|backend| backend.extract::<String>())
                {
                    Ok(backend) => DoctorCheck {
                        name: "python.matplotlib".to_string(),
                        status: CheckStatus::Pass,
                        detail: format!("backend={backend}"),
                    },
                    Err(error) => DoctorCheck {
                        name: "python.matplotlib".to_string(),
                        status: CheckStatus::Fail,
                        detail: error.to_string(),
                    },
                },
            );
        } else {
            checks.push(DoctorCheck {
                name: "python.matplotlib".to_string(),
                status: CheckStatus::Skip,
                detail: "plotting disabled".to_string(),
            });
        }
    });
}

fn check_hardware(cfg: &Config, probe_fetch: bool, checks: &mut Vec<DoctorCheck>) -> Option<u64> {
    let Some(configured) = cfg.instruments.as_ref() else {
        checks.push(DoctorCheck {
            name: "hardware".to_string(),
            status: CheckStatus::Skip,
            detail: "no instruments configured".to_string(),
        });
        return None;
    };

    let scope_ready = inspect_instrument(
        "scope",
        InstrumentRole::Oscilloscope,
        &configured.oscilloscope.model,
        &configured.oscilloscope.connection,
        checks,
    );
    let predicted_bytes = scope_ready.and_then(|spec| probe_scope(cfg, probe_fetch, spec, checks));

    if let Some(generator) = &configured.function_generator {
        if let Some(spec) = inspect_instrument(
            "generator",
            InstrumentRole::FunctionGenerator,
            &generator.model,
            &generator.connection,
            checks,
        ) {
            probe_generator(&generator.model, &generator.connection, spec, checks);
        }
    } else {
        checks.push(DoctorCheck {
            name: "generator".to_string(),
            status: CheckStatus::Skip,
            detail: "not configured".to_string(),
        });
    }

    predicted_bytes
}

fn inspect_instrument(
    name: &str,
    expected_role: InstrumentRole,
    model: &str,
    connection: &Connection,
    checks: &mut Vec<DoctorCheck>,
) -> Option<&'static InstrumentSpec> {
    let transport = transport_kind(connection);
    checks.push(DoctorCheck {
        name: format!("{name}.connection"),
        status: CheckStatus::Pass,
        detail: connection_uri(connection),
    });
    checks.push(DoctorCheck {
        name: format!("{name}.timeout"),
        status: CheckStatus::Pass,
        detail: timeout_detail(name, connection),
    });

    let spec = match find_instrument(model) {
        Some(spec) if spec.role != expected_role => {
            checks.push(DoctorCheck {
                name: format!("{name}.registry"),
                status: CheckStatus::Fail,
                detail: format!(
                    "model {model} is registered as {}, expected {}",
                    spec.role.as_str(),
                    expected_role.as_str()
                ),
            });
            None
        }
        Some(spec) if !spec.transports.contains(&transport) => {
            checks.push(DoctorCheck {
                name: format!("{name}.registry"),
                status: CheckStatus::Fail,
                detail: format!(
                    "model {model} does not support {}; supported transports: {}",
                    transport.as_str(),
                    spec.transports
                        .iter()
                        .map(|value| value.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            });
            None
        }
        Some(spec) => {
            checks.push(DoctorCheck {
                name: format!("{name}.registry"),
                status: CheckStatus::Pass,
                detail: format!("{model} via {}", transport.as_str()),
            });
            Some(spec)
        }
        None => {
            checks.push(DoctorCheck {
                name: format!("{name}.registry"),
                status: CheckStatus::Fail,
                detail: format!("unknown instrument model {model}"),
            });
            None
        }
    };

    let feature_available = transport_feature_available(transport);
    let (status, detail) = match transport.required_feature() {
        Some(feature) if feature_available => (
            CheckStatus::Pass,
            format!("Cargo feature {feature} enabled"),
        ),
        Some(feature) => {
            let note = transport
                .feature_note()
                .map_or(String::new(), |note| format!("; requires {note}"));
            (
                CheckStatus::Fail,
                format!(
                    "{} requires Cargo feature {feature}{note}; rebuild with --features {feature}",
                    transport.as_str()
                ),
            )
        }
        None => (CheckStatus::Pass, "no Cargo feature required".to_string()),
    };
    checks.push(DoctorCheck {
        name: format!("{name}.feature"),
        status,
        detail,
    });

    if feature_available { spec } else { None }
}

fn transport_kind(connection: &Connection) -> TransportKind {
    match connection {
        Connection::Gpib { .. } => TransportKind::Gpib,
        Connection::Tcpip { .. } => TransportKind::Tcpip,
        Connection::Usbtmc { .. } => TransportKind::Usbtmc,
        Connection::PrologixTcp { .. } => TransportKind::PrologixTcp,
        Connection::PrologixSerial { .. } => TransportKind::PrologixSerial,
    }
}

fn transport_feature_available(transport: TransportKind) -> bool {
    match transport {
        TransportKind::Dummy => true,
        TransportKind::Gpib => cfg!(feature = "hw-gpib"),
        TransportKind::Tcpip => cfg!(feature = "hw-core"),
        TransportKind::Usbtmc => cfg!(all(target_os = "windows", feature = "hw-gpib")),
        TransportKind::PrologixTcp => cfg!(feature = "hw-prologix-tcp"),
        TransportKind::PrologixSerial => cfg!(feature = "hw-prologix-serial"),
    }
}

fn timeout_detail(name: &str, connection: &Connection) -> String {
    match connection {
        Connection::Tcpip { .. } if name == "scope" => "connect=5s, read/write=30s".to_string(),
        Connection::Tcpip { .. } => "connection-specific timeout".to_string(),
        Connection::Usbtmc { .. } => "read/write=30s".to_string(),
        Connection::Gpib { .. } => "read/write=10s".to_string(),
        Connection::PrologixTcp {
            read_timeout_ms, ..
        }
        | Connection::PrologixSerial {
            read_timeout_ms, ..
        } => format!(
            "GPIB read={read_timeout_ms}ms, host read/write={}ms",
            instruments::transport::prologix_host_io_timeout(*read_timeout_ms).as_millis()
        ),
    }
}

#[cfg(feature = "hw-core")]
fn probe_scope(
    cfg: &Config,
    probe_fetch: bool,
    spec: &InstrumentSpec,
    checks: &mut Vec<DoctorCheck>,
) -> Option<u64> {
    use crate::communications::oscilloscope::OscilloscopeHandler;
    use crate::utils::channels::build_channel_list;
    use instruments::rigol::DhoTriggerStatus;

    let mut predicted_bytes = None;
    match OscilloscopeHandler::initialize(cfg) {
        Ok(mut scope) => {
            if spec
                .capabilities
                .contains(&InstrumentCapability::ScpiIdentify)
            {
                record_identity(
                    "scope",
                    spec.model,
                    &cfg.instruments.as_ref()?.oscilloscope.connection,
                    scope.identify(),
                    false,
                    checks,
                );
            } else {
                checks.push(DoctorCheck {
                    name: "scope.idn".to_string(),
                    status: CheckStatus::Skip,
                    detail: "instrument has no SCPI identify capability".to_string(),
                });
            }
            if probe_fetch && let Err(error) = scope.stop() {
                checks.push(failed("scope.stop", error));
            }
            match scope.query_trigger_status() {
                Ok(status) => checks.push(DoctorCheck {
                    name: "scope.state".to_string(),
                    status: if status == DhoTriggerStatus::Stop {
                        CheckStatus::Pass
                    } else {
                        CheckStatus::Warn
                    },
                    detail: format!("{status:?}"),
                }),
                Err(error) => checks.push(failed("scope.state", error)),
            }
            match scope.query_memory_depth() {
                Ok(depth) => {
                    let channels = build_channel_list(cfg).map_or(0, |channels| channels.len());
                    let bytes = u64::try_from(depth)
                        .unwrap_or(u64::MAX)
                        .saturating_mul(u64::try_from(channels).unwrap_or(u64::MAX))
                        .saturating_mul(2);
                    predicted_bytes = Some(bytes);
                    checks.push(DoctorCheck {
                        name: "scope.memory".to_string(),
                        status: CheckStatus::Pass,
                        detail: format!(
                            "{depth} samples/channel, predicted RAW {:.2} GiB",
                            bytes as f64 / 1024.0_f64.powi(3)
                        ),
                    });
                }
                Err(error) => checks.push(failed("scope.memory", error)),
            }
        }
        Err(error) => checks.push(failed("scope.open", error)),
    }
    predicted_bytes
}

#[cfg(feature = "hw-core")]
fn failed(name: &str, error: impl std::fmt::Display) -> DoctorCheck {
    DoctorCheck {
        name: name.to_string(),
        status: CheckStatus::Fail,
        detail: error.to_string(),
    }
}

#[cfg(not(feature = "hw-core"))]
fn probe_scope(
    _cfg: &Config,
    _probe_fetch: bool,
    _spec: &InstrumentSpec,
    checks: &mut Vec<DoctorCheck>,
) -> Option<u64> {
    checks.push(DoctorCheck {
        name: "scope.probe".to_string(),
        status: CheckStatus::Skip,
        detail: "built without hw-core".to_string(),
    });
    None
}

#[cfg(feature = "hw-core")]
fn probe_generator(
    model: &str,
    connection: &Connection,
    spec: &InstrumentSpec,
    checks: &mut Vec<DoctorCheck>,
) {
    use crate::communications::function_generator::scpi_connection;
    use instruments::transport::open_scpi_transport;

    let has_scpi_identity = spec
        .capabilities
        .contains(&InstrumentCapability::ScpiIdentify);
    let has_transport_diagnostic = !transport_kind(connection)
        .diagnostic_capabilities()
        .is_empty();
    if !has_scpi_identity && !has_transport_diagnostic {
        checks.push(DoctorCheck {
            name: "generator.idn".to_string(),
            status: CheckStatus::Skip,
            detail: "instrument has no SCPI identify capability".to_string(),
        });
        return;
    }

    let scpi_connection = match scpi_connection(connection) {
        Ok(connection) => connection,
        Err(error) => {
            checks.push(failed("generator.open", error));
            return;
        }
    };
    let mut transport = match open_scpi_transport(&scpi_connection) {
        Ok(transport) => transport,
        Err(error) => {
            checks.push(failed("generator.open", error));
            return;
        }
    };

    probe_scpi_identity(
        "generator",
        model,
        connection,
        spec,
        transport.as_mut(),
        checks,
    );
}

#[cfg(feature = "hw-core")]
fn probe_scpi_identity(
    name: &str,
    model: &str,
    connection: &Connection,
    spec: &InstrumentSpec,
    transport: &mut dyn instruments::transport::ScpiTransport,
    checks: &mut Vec<DoctorCheck>,
) {
    let needs_controller_version = transport_kind(connection)
        .diagnostic_capabilities()
        .contains(&TransportDiagnosticCapability::PrologixControllerVersion);
    let controller_reachable = if needs_controller_version {
        match transport.controller_version() {
            Ok(Some(version)) if !version.trim().is_empty() => {
                checks.push(DoctorCheck {
                    name: format!("{name}.controller"),
                    status: CheckStatus::Pass,
                    detail: version,
                });
                true
            }
            Ok(Some(_)) => {
                checks.push(DoctorCheck {
                    name: format!("{name}.controller"),
                    status: CheckStatus::Fail,
                    detail: "Prologix controller returned an empty ++ver response".to_string(),
                });
                false
            }
            Ok(None) => {
                checks.push(DoctorCheck {
                    name: format!("{name}.controller"),
                    status: CheckStatus::Fail,
                    detail: "transport does not expose the required ++ver diagnostic".to_string(),
                });
                false
            }
            Err(error) => {
                checks.push(failed(&format!("{name}.controller"), error));
                false
            }
        }
    } else {
        false
    };

    if needs_controller_version && !controller_reachable {
        return;
    }
    if !spec
        .capabilities
        .contains(&InstrumentCapability::ScpiIdentify)
    {
        checks.push(DoctorCheck {
            name: format!("{name}.idn"),
            status: CheckStatus::Skip,
            detail: "instrument has no SCPI identify capability".to_string(),
        });
        return;
    }
    record_identity(
        name,
        model,
        connection,
        transport.query_line("*IDN?").map_err(Into::into),
        controller_reachable,
        checks,
    );
}

#[cfg(not(feature = "hw-core"))]
fn probe_generator(
    _model: &str,
    _connection: &Connection,
    _spec: &InstrumentSpec,
    checks: &mut Vec<DoctorCheck>,
) {
    checks.push(DoctorCheck {
        name: "generator.probe".to_string(),
        status: CheckStatus::Skip,
        detail: "built without hw-core".to_string(),
    });
}

#[cfg(any(feature = "hw-core", test))]
fn record_identity(
    name: &str,
    model: &str,
    connection: &Connection,
    result: Result<String>,
    controller_reachable: bool,
    checks: &mut Vec<DoctorCheck>,
) {
    match result {
        Ok(idn) if idn.trim().is_empty() => checks.push(DoctorCheck {
            name: format!("{name}.idn"),
            status: CheckStatus::Fail,
            detail: "instrument returned an empty *IDN? response".to_string(),
        }),
        Ok(idn) => {
            let reported_model = scpi_idn_model(&idn);
            let matches = reported_model.is_some_and(|value| value.eq_ignore_ascii_case(model));
            checks.push(DoctorCheck {
                name: format!("{name}.idn"),
                status: CheckStatus::Pass,
                detail: idn.clone(),
            });
            checks.push(DoctorCheck {
                name: format!("{name}.model"),
                status: if matches {
                    CheckStatus::Pass
                } else {
                    CheckStatus::Fail
                },
                detail: if matches {
                    format!("response matches configured model {model}")
                } else if let Some(reported_model) = reported_model {
                    format!(
                        "configured model {model} does not match reported model {reported_model}: {idn}"
                    )
                } else {
                    format!("*IDN? response has no valid model field for {model}: {idn}")
                },
            });
        }
        Err(error) => checks.push(DoctorCheck {
            name: format!("{name}.idn"),
            status: CheckStatus::Fail,
            detail: identity_error_detail(connection, controller_reachable, &error),
        }),
    }
}

#[cfg(any(feature = "hw-core", test))]
fn scpi_idn_model(idn: &str) -> Option<&str> {
    idn.split(',')
        .nth(1)
        .map(str::trim)
        .filter(|model| !model.is_empty())
}

#[cfg(any(feature = "hw-core", test))]
fn identity_error_detail(
    connection: &Connection,
    controller_reachable: bool,
    error: &anyhow::Error,
) -> String {
    match (connection, controller_reachable) {
        (Connection::PrologixTcp { address, .. }, true)
        | (Connection::PrologixSerial { address, .. }, true) => format!(
            "Prologix controller is reachable, but GPIB address {address} did not answer *IDN?: {error:#}; verify the instrument PAD, cable, and power"
        ),
        _ => format!("{error:#}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(feature = "hw-core")]
    struct MockScpiTransport {
        controller_version: Option<instruments::Result<Option<String>>>,
        identity: Option<instruments::Result<String>>,
        queries: usize,
    }

    #[cfg(feature = "hw-core")]
    impl instruments::transport::ScpiTransport for MockScpiTransport {
        fn write_line(&mut self, _command: &str) -> instruments::Result<()> {
            Ok(())
        }

        fn query_line(&mut self, command: &str) -> instruments::Result<String> {
            assert_eq!(command, "*IDN?");
            self.queries += 1;
            self.identity.take().expect("unexpected identity query")
        }

        fn controller_version(&mut self) -> instruments::Result<Option<String>> {
            self.controller_version
                .take()
                .expect("unexpected controller version query")
        }
    }

    fn temporary_directory() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("pmoke-doctor-{}-{nonce}", std::process::id()))
    }

    #[test]
    fn staging_directory_is_a_hidden_sibling() {
        assert_eq!(
            staging_path(Path::new("shot/raw_waveform")),
            PathBuf::from("shot/.raw_waveform.tmp")
        );
    }

    #[test]
    fn writable_probe_leaves_the_directory_unchanged() {
        let directory = temporary_directory();
        fs::create_dir(&directory).unwrap();

        writable_probe(&directory).unwrap();

        assert!(fs::read_dir(&directory).unwrap().next().is_none());
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn writable_probe_rejects_a_missing_directory() {
        let directory = temporary_directory();
        let error = writable_probe(&directory).unwrap_err();
        assert!(error.to_string().contains("does not exist"));
    }

    #[test]
    fn missing_output_directories_use_the_nearest_existing_ancestor() {
        let directory = temporary_directory();
        fs::create_dir(&directory).unwrap();
        let planned = directory.join("shot/raw_waveform");

        let ancestor = existing_storage_ancestor(&planned).unwrap();

        assert_eq!(ancestor, directory);
        assert!(!planned.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn capacity_check_fails_before_an_acquisition_that_cannot_fit() {
        let mut checks = Vec::new();
        check_capacity(Some(999), Some(1_000), &mut checks);
        assert_eq!(checks[0].status, CheckStatus::Fail);
        assert!(checks[0].detail.contains("exceeds free space"));
    }

    #[test]
    fn capacity_check_warns_when_only_the_safety_margin_is_missing() {
        let mut checks = Vec::new();
        let gib = 1024_u64.pow(3);
        check_capacity(Some(2 * gib), Some(2 * gib - 1), &mut checks);
        assert_eq!(checks[0].status, CheckStatus::Warn);
    }

    #[test]
    fn capacity_check_passes_with_headroom() {
        let mut checks = Vec::new();
        check_capacity(Some(2_000_000_000), Some(1_000_000_000), &mut checks);
        assert_eq!(checks[0].status, CheckStatus::Pass);
    }

    #[test]
    fn prologix_timeout_and_uri_are_reported_from_config() {
        let connection = Connection::PrologixTcp {
            host: "10.249.11.17".to_string(),
            port: 1234,
            address: 17,
            read_timeout_ms: 2500,
        };

        assert_eq!(
            connection_uri(&connection),
            "prologix-tcp://10.249.11.17:1234?addr=17&read_timeout_ms=2500"
        );
        assert_eq!(
            timeout_detail("generator", &connection),
            "GPIB read=2500ms, host read/write=2750ms"
        );
        assert_eq!(transport_kind(&connection), TransportKind::PrologixTcp);
    }

    #[test]
    fn identity_mismatch_is_a_failure() {
        let mut checks = Vec::new();
        record_identity(
            "scope",
            "DHO5108",
            &Connection::Tcpip {
                ip: "10.249.11.19".to_string(),
                port: 5555,
            },
            Ok("RIGOL TECHNOLOGIES,DHO924S,serial,firmware".to_string()),
            false,
            &mut checks,
        );

        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].status, CheckStatus::Pass);
        assert_eq!(checks[1].name, "scope.model");
        assert_eq!(checks[1].status, CheckStatus::Fail);
    }

    #[test]
    fn identity_model_requires_an_exact_scpi_model_field() {
        let connection = Connection::Tcpip {
            ip: "10.249.11.19".to_string(),
            port: 5555,
        };
        let mut checks = Vec::new();

        record_identity(
            "scope",
            "DHO5108",
            &connection,
            Ok("RIGOL,DHO5108A,serial,firmware".to_string()),
            false,
            &mut checks,
        );

        assert_eq!(checks[1].status, CheckStatus::Fail);
        assert!(checks[1].detail.contains("reported model DHO5108A"));
        assert_eq!(
            scpi_idn_model("RIGOL TECHNOLOGIES, DHO5108 ,serial,firmware"),
            Some("DHO5108")
        );
    }

    #[test]
    fn reachable_prologix_reports_pad_specific_identity_errors() {
        let detail = identity_error_detail(
            &Connection::PrologixTcp {
                host: "10.249.11.17".to_string(),
                port: 1234,
                address: 17,
                read_timeout_ms: 2500,
            },
            true,
            &anyhow::anyhow!("timed out"),
        );

        assert!(detail.contains("controller is reachable"));
        assert!(detail.contains("GPIB address 17"));
        assert!(detail.contains("verify the instrument PAD"));
    }

    #[cfg(feature = "hw-core")]
    #[test]
    fn prologix_probe_separates_controller_and_instrument_identity() {
        let connection = Connection::PrologixTcp {
            host: "10.249.11.17".to_string(),
            port: 1234,
            address: 17,
            read_timeout_ms: 2500,
        };
        let spec = find_instrument("WF1946B").unwrap();
        let mut transport = MockScpiTransport {
            controller_version: Some(Ok(Some("Prologix controller 6.101".to_string()))),
            identity: Some(Ok("NF Corporation,WF1946B,0,1".to_string())),
            queries: 0,
        };
        let mut checks = Vec::new();

        probe_scpi_identity(
            "generator",
            "WF1946B",
            &connection,
            spec,
            &mut transport,
            &mut checks,
        );

        assert_eq!(transport.queries, 1);
        assert_eq!(checks.len(), 3);
        assert_eq!(checks[0].name, "generator.controller");
        assert_eq!(checks[0].status, CheckStatus::Pass);
        assert_eq!(checks[1].name, "generator.idn");
        assert_eq!(checks[2].name, "generator.model");
        assert_eq!(checks[2].status, CheckStatus::Pass);
    }

    #[cfg(feature = "hw-core")]
    #[test]
    fn instruments_without_scpi_identity_are_not_queried() {
        let spec = find_instrument("DummyInstrument").unwrap();
        let mut transport = MockScpiTransport {
            controller_version: None,
            identity: None,
            queries: 0,
        };
        let mut checks = Vec::new();

        probe_scpi_identity(
            "generator",
            spec.model,
            &Connection::Gpib {
                board: 0,
                address: 1,
            },
            spec,
            &mut transport,
            &mut checks,
        );

        assert_eq!(transport.queries, 0);
        assert_eq!(checks.len(), 1);
        assert_eq!(checks[0].status, CheckStatus::Skip);
    }

    #[cfg(feature = "hw-core")]
    #[test]
    fn prologix_controller_is_diagnosed_for_non_scpi_instruments() {
        let spec = find_instrument("DummyInstrument").unwrap();
        let mut transport = MockScpiTransport {
            controller_version: Some(Ok(Some("Prologix controller 6.101".to_string()))),
            identity: None,
            queries: 0,
        };
        let mut checks = Vec::new();

        probe_scpi_identity(
            "generator",
            spec.model,
            &Connection::PrologixTcp {
                host: "10.249.11.17".to_string(),
                port: 1234,
                address: 17,
                read_timeout_ms: 2500,
            },
            spec,
            &mut transport,
            &mut checks,
        );

        assert_eq!(transport.queries, 0);
        assert_eq!(checks.len(), 2);
        assert_eq!(checks[0].name, "generator.controller");
        assert_eq!(checks[0].status, CheckStatus::Pass);
        assert_eq!(checks[1].name, "generator.idn");
        assert_eq!(checks[1].status, CheckStatus::Skip);
    }
}
