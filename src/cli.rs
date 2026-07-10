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
    /// Upgrade the config to the latest compatible schema
    Upgrade {
        /// Write the upgraded TOML to FILE; use '-' for standard output
        #[arg(long, value_name = "FILE", conflicts_with_all = ["in_place", "check"])]
        output: Option<PathBuf>,

        /// Atomically replace the source config after creating a versioned backup
        #[arg(long, conflicts_with_all = ["output", "check"])]
        in_place: bool,

        /// Only report whether an upgrade is required
        #[arg(long, conflicts_with_all = ["output", "in_place"])]
        check: bool,

        /// Accept migration steps that can change legacy behavior
        #[arg(long)]
        accept_lossy: bool,

        /// Target config version (only version 4 is currently supported)
        #[arg(
            long,
            default_value_t = crate::config::LATEST_CONFIG_VERSION,
            value_name = "VERSION"
        )]
        to: u32,
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
    fn parses_config_upgrade_options_without_hardware_feature() {
        let cli = Cli::try_parse_from([
            "pmoke",
            "--config",
            "old.toml",
            "config",
            "upgrade",
            "--output",
            "new.toml",
            "--accept-lossy",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Config {
                command: ConfigCommand::Upgrade {
                    output: Some(_),
                    accept_lossy: true,
                    ..
                }
            })
        ));
    }

    #[test]
    fn rejects_conflicting_upgrade_destinations() {
        assert!(
            Cli::try_parse_from([
                "pmoke",
                "config",
                "upgrade",
                "--output",
                "new.toml",
                "--in-place",
            ])
            .is_err()
        );
    }
}
