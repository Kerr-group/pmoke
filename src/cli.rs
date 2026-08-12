use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

/// A simple CLI tool to inspect and validate experiment configuration files.
#[derive(Parser, Debug)]
#[command(
    name = "pmoke",
    version,
    author = "Soichiro Yamane",
    about = "A CLI tool to conduct pulsed MOKE experiments and analyze the data.",
    long_about = None
)]
pub struct Cli {
    /// Path to the configuration file (default: ./config.toml)
    #[arg(short, long, default_value = "config.toml", value_name = "FILE")]
    pub config: String,

    /// Store and read run artifacts under this directory
    #[arg(long, global = true, value_name = "DIR")]
    pub run_dir: Option<PathBuf>,

    /// Overwrite existing run artifacts without error
    #[arg(short, long, global = true)]
    pub force: bool,

    /// Subcommands for the tool
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Available subcommands
#[derive(Subcommand, Debug)]
pub enum Command {
    /// Display the contents of the configuration file
    Show,
    /// Open a live terminal dashboard for configuration and analysis artifacts
    Monitor,
    /// Inspect and migrate configuration files
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Inspect and verify stored RAW waveform data
    Raw {
        #[command(subcommand)]
        command: RawCommand,
    },
    /// Inspect supported instruments and hardware capabilities
    Instruments {
        #[command(subcommand)]
        command: InstrumentsCommand,
    },
    /// Benchmark instrument transport request latency
    Bench {
        #[command(subcommand)]
        command: BenchCommand,
    },
    /// Export stored data to interchange formats
    Export {
        #[command(subcommand)]
        command: ExportCommand,
    },
    /// Diagnose config, storage, Python, and connected instruments
    Doctor {
        /// Emit a machine-readable JSON report
        #[arg(long)]
        json: bool,
        /// Allow active checks such as stopping the oscilloscope
        #[arg(long)]
        probe_fetch: bool,
    },
    /// Set single mode to the oscilloscope
    #[cfg(feature = "hw-core")]
    Single,
    /// Send trigger signal from the function generator
    #[cfg(feature = "hw-core")]
    Trigger,
    /// Set single mode and send trigger signal
    #[cfg(feature = "hw-core")]
    Autoshot,
    /// Fetch data from the oscilloscope and save to a file
    #[cfg(feature = "hw-core")]
    Fetch {
        /// Override output format from config [fetch].output
        #[arg(long, value_enum)]
        format: Option<FetchFormat>,
    },
    /// Capture an oscilloscope screenshot directly to the PC
    #[cfg(feature = "hw-core")]
    Screenshot,
    /// Perform auto measurement (set single mode, trigger, fetch)
    #[cfg(feature = "hw-core")]
    Automeasure,
    /// Fit the recorded EOM-drive sine wave
    Reference,
    /// Analyze the sensor signal
    Sensor,
    /// Run numerical lock-in analysis
    Li,
    /// Rotate the reference phase for lock-in analysis
    Phase,
    /// Calculate the Kerr angle
    Kerr,
    /// Run all analysis steps: reference, sensor, lock-in, phase, Kerr
    Analyze,
    /// Automated analysis after manually triggering the pulse (fetch, lock-in, phase, Kerr)
    #[cfg(feature = "hw-core")]
    Process,
    /// Run the full automatic measurement and analysis
    #[cfg(feature = "hw-core")]
    Auto,
    /// Generate shell completion script
    Completions {
        /// Shell to generate for: bash, zsh, fish, powershell, elvish
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Subcommand, Debug)]
pub enum BenchCommand {
    /// Benchmark one SCPI query and save a compact reproducibility report
    ScpiQuery {
        /// Connection URI accepted by `pmoke instruments query`
        #[arg(long, value_name = "URI")]
        connection: String,

        /// SCPI query command to benchmark
        #[arg(long, default_value = "*IDN?", value_name = "COMMAND")]
        command: String,

        /// Measured query count
        #[arg(short = 'n', long, default_value_t = 50, value_name = "N")]
        iterations: usize,

        /// Unmeasured query count before measurement
        #[arg(long, default_value_t = 3, value_name = "N")]
        warmup: usize,

        /// Timeout used when the URI has no transport-specific timeout
        #[arg(long, default_value_t = 3000, value_name = "MS")]
        timeout_ms: u64,

        /// Save TOML to FILE instead of the run benchmark directory
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,

        /// Emit the compact report as JSON after saving TOML
        #[arg(long)]
        json: bool,
    },

    /// Measure text request/response latency for a connection URI
    Transport {
        /// Connection URI accepted by `pmoke instruments query`
        #[arg(long, value_name = "URI")]
        connection: String,

        /// Text protocol used to validate each request
        #[arg(long, value_enum, default_value_t = BenchProtocol::Scpi)]
        protocol: BenchProtocol,

        /// Request to benchmark; repeat for multiple requests
        #[arg(
            short = 'r',
            long = "request",
            visible_alias = "command",
            default_value = "*IDN?",
            value_name = "TEXT"
        )]
        requests: Vec<String>,

        /// Measured request count per request
        #[arg(short = 'n', long, default_value_t = 50, value_name = "N")]
        iterations: usize,

        /// Unmeasured request count before each measurement
        #[arg(long, default_value_t = 3, value_name = "N")]
        warmup: usize,

        /// Timeout used when the URI has no transport-specific timeout
        #[arg(long, default_value_t = 3000, value_name = "MS")]
        timeout_ms: u64,

        /// Save the complete JSON report to a file
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,

        /// Emit the complete report as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Copy, Debug, ValueEnum, PartialEq, Eq)]
pub enum BenchProtocol {
    /// Validate requests as one SCPI query
    Scpi,
    /// Send one non-empty text line and expect one text response line
    Line,
}

#[derive(Subcommand, Debug)]
pub enum InstrumentsCommand {
    /// List supported instrument models
    List(JsonOutput),
    /// Explain a supported instrument model
    Explain {
        /// Instrument model name, for example Keithley2010
        model: String,
        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,
    },
    /// Send one SCPI text query to a connection URI
    Query {
        /// Connection URI, for example prologix-tcp://host:1234?addr=17
        #[arg(long, value_name = "URI")]
        connection: String,

        /// Timeout used when the URI does not include a transport-specific timeout
        #[arg(long, default_value_t = 3000, value_name = "MS")]
        timeout_ms: u64,

        /// Emit machine-readable JSON
        #[arg(long)]
        json: bool,

        /// SCPI query command, for example *IDN?
        command: String,
    },
}

#[derive(Args, Debug)]
pub struct JsonOutput {
    /// Emit machine-readable JSON
    #[arg(long)]
    pub json: bool,
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
    /// Create a starter config file
    Init {
        /// Write the template to FILE instead of --config; use '-' for standard output
        #[arg(long, value_name = "FILE")]
        output: Option<PathBuf>,

        /// Overwrite an existing output file
        #[arg(short, long)]
        force: bool,
    },

    /// Validate the config file without running an analysis command
    Validate,

    /// Explain config sections and fields
    Explain {
        /// Field or section path to explain, for example lockin.filter
        #[arg(value_name = "PATH")]
        path: Option<String>,
    },

    /// Migrate the config to the latest executable schema
    Migrate {
        /// Write the migrated TOML to FILE; use '-' for standard output
        #[arg(long, value_name = "FILE", conflicts_with_all = ["in_place", "check"])]
        output: Option<PathBuf>,

        /// Atomically replace the source config after creating a versioned backup
        #[arg(long, conflicts_with_all = ["output", "check"])]
        in_place: bool,

        /// Only report whether a migration is required
        #[arg(long, conflicts_with_all = ["output", "in_place"])]
        check: bool,

        /// Accept migration steps that can change legacy behavior
        #[arg(long)]
        accept_lossy: bool,

        /// Require a specific target version instead of the latest executable version
        #[arg(long, value_name = "VERSION")]
        to: Option<u32>,
    },
}

#[derive(Subcommand, Debug)]
pub enum RawCommand {
    /// Verify RAW metadata, file sizes, and available checksums
    Verify {
        /// RAW acquisition directory (defaults to acquisition/ with legacy fallback)
        #[arg(long, value_name = "DIR")]
        input: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ExportCommand {
    /// Convert a verified RAW waveform directory to CSV
    Csv {
        /// RAW acquisition directory (defaults to acquisition/ with legacy fallback)
        #[arg(long, value_name = "DIR")]
        input: Option<PathBuf>,
        /// CSV destination (defaults to acquisition/waveforms/waveform.csv)
        #[arg(long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
    /// Convert analysis result CSV files to NumPy tables
    Npy {
        /// Destination directory (defaults to NPY files beside canonical analysis CSVs)
        #[arg(long, value_name = "DIR")]
        output: Option<PathBuf>,
    },
}

#[cfg(feature = "hw-core")]
#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum FetchFormat {
    Csv,
    Raw,
    CsvAndRaw,
}

#[cfg(all(test, feature = "hw-core"))]
mod tests {
    use super::*;

    #[test]
    fn screenshot_command_replaces_image_command() {
        let cli = Cli::try_parse_from(["pmoke", "screenshot"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Screenshot)));
        assert!(Cli::try_parse_from(["pmoke", "image"]).is_err());
    }

    #[test]
    fn fetch_uses_only_canonical_outputs() {
        let cli = Cli::try_parse_from(["pmoke", "fetch", "--format", "raw"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Fetch {
                format: Some(FetchFormat::Raw)
            })
        ));
        assert!(Cli::try_parse_from(["pmoke", "fetch", "--out", "custom.csv"]).is_err());
    }
}

#[cfg(test)]
mod config_command_tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn cli_version_matches_the_package_version() {
        assert_eq!(
            Cli::command().get_version(),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn parses_config_migrate_options_without_hardware_feature() {
        let cli = Cli::try_parse_from([
            "pmoke",
            "--config",
            "old.toml",
            "config",
            "migrate",
            "--output",
            "new.toml",
            "--accept-lossy",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                command: ConfigCommand::Migrate {
                    output: Some(_),
                    accept_lossy: true,
                    ..
                }
            })
        ));
    }

    #[test]
    fn parses_config_init_validate_and_explain() {
        let init = Cli::try_parse_from([
            "pmoke",
            "--config",
            "config.toml",
            "config",
            "init",
            "--force",
        ])
        .unwrap();
        assert!(matches!(
            init.command,
            Some(Command::Config {
                command: ConfigCommand::Init { force: true, .. }
            })
        ));

        let validate = Cli::try_parse_from(["pmoke", "config", "validate"]).unwrap();
        assert!(matches!(
            validate.command,
            Some(Command::Config {
                command: ConfigCommand::Validate
            })
        ));

        let explain = Cli::try_parse_from(["pmoke", "config", "explain", "lockin.filter"]).unwrap();
        assert!(matches!(
            explain.command,
            Some(Command::Config {
                command: ConfigCommand::Explain { path: Some(_) }
            })
        ));
    }

    #[test]
    fn parses_instruments_list_and_explain_without_hardware_feature() {
        let list = Cli::try_parse_from(["pmoke", "instruments", "list", "--json"]).unwrap();
        assert!(matches!(
            list.command,
            Some(Command::Instruments {
                command: InstrumentsCommand::List(JsonOutput { json: true })
            })
        ));

        let explain =
            Cli::try_parse_from(["pmoke", "instruments", "explain", "Keithley2010"]).unwrap();
        assert!(matches!(
            explain.command,
            Some(Command::Instruments {
                command: InstrumentsCommand::Explain { model, json: false }
            }) if model == "Keithley2010"
        ));
    }

    #[test]
    fn parses_instruments_query_without_hardware_feature() {
        let cli = Cli::try_parse_from([
            "pmoke",
            "instruments",
            "query",
            "--connection",
            "prologix-tcp://10.249.11.17:1234?addr=17",
            "--timeout-ms",
            "2500",
            "--json",
            "*IDN?",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Instruments {
                command: InstrumentsCommand::Query {
                    connection,
                    timeout_ms: 2500,
                    json: true,
                    command,
                }
            }) if connection == "prologix-tcp://10.249.11.17:1234?addr=17" && command == "*IDN?"
        ));
    }

    #[test]
    fn parses_transport_benchmark_defaults() {
        let cli = Cli::try_parse_from([
            "pmoke",
            "bench",
            "transport",
            "--connection",
            "tcp://127.0.0.1:5025",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Some(Command::Bench {
                command: BenchCommand::Transport {
                    connection,
                    protocol: BenchProtocol::Scpi,
                    requests,
                    iterations: 50,
                    warmup: 3,
                    timeout_ms: 3000,
                    output: None,
                    json: false,
                }
            }) if connection == "tcp://127.0.0.1:5025" && requests == ["*IDN?"]
        ));
    }

    #[test]
    fn parses_scpi_query_benchmark_defaults() {
        let cli = Cli::try_parse_from([
            "pmoke",
            "--run-dir",
            "shot-001",
            "bench",
            "scpi-query",
            "--connection",
            "prologix-tcp://10.249.11.17:1234?addr=17",
        ])
        .unwrap();

        assert_eq!(cli.run_dir, Some(PathBuf::from("shot-001")));
        assert!(matches!(
            cli.command,
            Some(Command::Bench {
                command: BenchCommand::ScpiQuery {
                    connection,
                    command,
                    iterations: 50,
                    warmup: 3,
                    timeout_ms: 3000,
                    output: None,
                    json: false,
                }
            }) if connection == "prologix-tcp://10.249.11.17:1234?addr=17"
                && command == "*IDN?"
        ));
    }

    #[test]
    fn parses_transport_benchmark_with_multiple_line_requests() {
        let cli = Cli::try_parse_from([
            "pmoke",
            "bench",
            "transport",
            "--connection",
            "tcp://127.0.0.1:1234",
            "--protocol",
            "line",
            "--request",
            "status",
            "--request",
            "value",
            "--iterations",
            "12",
            "--warmup",
            "0",
            "--timeout-ms",
            "800",
            "--output",
            "bench.json",
            "--json",
        ])
        .unwrap();

        assert!(matches!(
            cli.command,
            Some(Command::Bench {
                command: BenchCommand::Transport {
                    protocol: BenchProtocol::Line,
                    requests,
                    iterations: 12,
                    warmup: 0,
                    timeout_ms: 800,
                    output: Some(output),
                    json: true,
                    ..
                }
            }) if requests == ["status", "value"] && output == std::path::Path::new("bench.json")
        ));
    }

    #[test]
    fn rejects_conflicting_migration_destinations() {
        assert!(
            Cli::try_parse_from([
                "pmoke",
                "config",
                "migrate",
                "--output",
                "new.toml",
                "--in-place",
            ])
            .is_err()
        );
    }

    #[test]
    fn rejects_unpublished_upgrade_command_name() {
        assert!(Cli::try_parse_from(["pmoke", "config", "upgrade"]).is_err());
    }

    #[test]
    fn parses_raw_verify_without_hardware_feature() {
        let cli = Cli::try_parse_from(["pmoke", "raw", "verify", "--input", "shot/raw_waveform"])
            .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Raw {
                command: RawCommand::Verify { input: Some(_) }
            })
        ));
    }

    #[test]
    fn parses_doctor_options_without_hardware_feature() {
        let cli = Cli::try_parse_from(["pmoke", "doctor", "--json", "--probe-fetch"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Doctor {
                json: true,
                probe_fetch: true,
            })
        ));
    }

    #[test]
    fn parses_explicit_raw_csv_export() {
        let cli = Cli::try_parse_from([
            "pmoke",
            "export",
            "csv",
            "--input",
            "shot/raw_waveform",
            "--output",
            "shot/raw.csv",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Export {
                command: ExportCommand::Csv {
                    input: Some(_),
                    output: Some(_),
                }
            })
        ));
    }

    #[test]
    fn parses_run_directory_as_a_global_option() {
        let cli = Cli::try_parse_from(["pmoke", "--run-dir", "shot_000123", "analyze"]).unwrap();
        assert_eq!(cli.run_dir, Some(PathBuf::from("shot_000123")));
        assert!(matches!(cli.command, Some(Command::Analyze)));

        let cli = Cli::try_parse_from(["pmoke", "analyze", "--run-dir", "shot_000124"]).unwrap();
        assert_eq!(cli.run_dir, Some(PathBuf::from("shot_000124")));
    }

    #[test]
    fn parses_analysis_npy_export() {
        let cli =
            Cli::try_parse_from(["pmoke", "export", "npy", "--output", "shot/analysis"]).unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Export {
                command: ExportCommand::Npy { output: Some(_) }
            })
        ));
    }
}
