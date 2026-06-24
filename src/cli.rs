#[cfg(feature = "hw")]
use std::path::PathBuf;

#[cfg(feature = "hw")]
use clap::ValueEnum;
use clap::{Parser, Subcommand};
use clap_complete::Shell;

/// A simple CLI tool to inspect and validate experiment configuration files.
#[derive(Parser, Debug)]
#[command(
    name = "pmoke",
    version = "0.1.8",
    author = "Soichiro Yamane",
    about = "A CLI tool to conduct pulsed MOKE",
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

#[cfg(feature = "hw")]
#[derive(Copy, Clone, Debug, Eq, PartialEq, ValueEnum)]
pub enum FetchFormat {
    Csv,
    Raw,
    CsvAndRaw,
}
