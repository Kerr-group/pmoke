use std::path::PathBuf;

#[cfg(feature = "hw")]
use clap::ValueEnum;
use clap::{Parser, Subcommand};
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
    #[cfg(feature = "hw")]
    Single,
    /// Send trigger signal from the function generator
    #[cfg(feature = "hw")]
    Trigger,
    /// Set single mode and send trigger signal
    #[cfg(feature = "hw")]
    Autoshot,
    /// Fetch data from the oscilloscope and save to a file
    #[cfg(feature = "hw")]
    Fetch {
        /// Override output format from config [fetch].output
        #[arg(long, value_enum)]
        format: Option<FetchFormat>,
        /// Output path. CSV defaults to raw.csv; raw defaults to raw_waveform/
        #[arg(long, value_name = "PATH")]
        out: Option<PathBuf>,
    },
    /// Capture an oscilloscope screenshot directly to the PC
    #[cfg(feature = "hw")]
    Screenshot,
    /// Perform auto measurement (set single mode, trigger, fetch)
    #[cfg(feature = "hw")]
    Automeasure,
    /// Analyze the reference signal
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
    #[cfg(feature = "hw")]
    Process,
    /// Run the full automatic measurement and analysis
    #[cfg(feature = "hw")]
    Auto,
    /// Generate shell completion script
    Completions {
        /// Shell to generate for: bash, zsh, fish, powershell, elvish
        #[arg(value_enum)]
        shell: Shell,
    },
}

#[derive(Subcommand, Debug)]
pub enum ConfigCommand {
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
        /// RAW waveform directory (defaults to the configured raw_waveform artifact)
        #[arg(long, value_name = "DIR")]
        input: Option<PathBuf>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ExportCommand {
    /// Convert a verified RAW waveform directory to CSV
    Csv {
        /// RAW waveform directory (defaults to the configured raw_waveform artifact)
        #[arg(long, value_name = "DIR")]
        input: Option<PathBuf>,
        /// CSV destination (defaults to the configured raw.csv artifact)
        #[arg(long, value_name = "FILE")]
        output: Option<PathBuf>,
    },
    /// Convert analysis result CSV files to NumPy tables
    Npy {
        /// Destination directory (defaults to analysis_npy)
        #[arg(long, value_name = "DIR")]
        output: Option<PathBuf>,
    },
}

#[cfg(feature = "hw")]
#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum FetchFormat {
    Csv,
    Raw,
    CsvAndRaw,
}

#[cfg(all(test, feature = "hw"))]
mod tests {
    use super::*;

    #[test]
    fn screenshot_command_replaces_image_command() {
        let cli = Cli::try_parse_from(["pmoke", "screenshot"]).unwrap();
        assert!(matches!(cli.command, Some(Command::Screenshot)));
        assert!(Cli::try_parse_from(["pmoke", "image"]).is_err());
    }
}

#[cfg(test)]
mod config_command_tests {
    use super::*;

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
