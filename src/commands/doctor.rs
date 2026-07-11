use crate::config::{Config, ConfigDiagnostics, ConfigWarning};

use crate::ui;
use anyhow::{Context, Result, bail};
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

#[cfg(feature = "hw")]
fn check_hardware(cfg: &Config, probe_fetch: bool, checks: &mut Vec<DoctorCheck>) -> Option<u64> {
    use crate::communications::function_generator::FGHandler;
    use crate::communications::oscilloscope::OscilloscopeHandler;
    use crate::utils::channels::build_channel_list;
    use instruments::rigol::DhoTriggerStatus;

    let mut predicted_bytes = None;
    match OscilloscopeHandler::initialize(cfg) {
        Ok(mut scope) => {
            match scope.identify() {
                Ok(idn) => checks.push(DoctorCheck {
                    name: "scope.idn".to_string(),
                    status: CheckStatus::Pass,
                    detail: idn,
                }),
                Err(error) => checks.push(failed("scope.idn", error)),
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
        Err(error) => checks.push(failed("scope.connection", error)),
    }

    if cfg
        .instruments
        .as_ref()
        .and_then(|instruments| instruments.function_generator.as_ref())
        .is_some()
    {
        match FGHandler::initialize(cfg).and_then(|mut generator| generator.identify()) {
            Ok(idn) => checks.push(DoctorCheck {
                name: "generator.idn".to_string(),
                status: CheckStatus::Pass,
                detail: idn,
            }),
            Err(error) => checks.push(failed("generator.idn", error)),
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

#[cfg(feature = "hw")]
fn failed(name: &str, error: impl std::fmt::Display) -> DoctorCheck {
    DoctorCheck {
        name: name.to_string(),
        status: CheckStatus::Fail,
        detail: error.to_string(),
    }
}

#[cfg(not(feature = "hw"))]
fn check_hardware(_cfg: &Config, _probe_fetch: bool, checks: &mut Vec<DoctorCheck>) -> Option<u64> {
    checks.push(DoctorCheck {
        name: "hardware".to_string(),
        status: CheckStatus::Skip,
        detail: "built without hw feature".to_string(),
    });
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

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
}
