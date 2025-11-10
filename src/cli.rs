use clap::{Parser, Subcommand};
use clap_complete::Shell;

/// A simple CLI tool to inspect and validate experiment configuration files.
#[derive(Parser, Debug)]
#[command(
    name = "pMOKE",
    version = "0.1.0",
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
    /// Set single mode to the oscilloscope
    Single,
    /// Send trigger signal from the function generator
    Trigger,
    /// Set single mode and send trigger signal
    Autoshot,
    /// Fetch data from the oscilloscope and save to a file
    Fetch,
    /// Perform auto measurement (set single mode, trigger, fetch)
    Automeasure,
    /// Analyze the reference signal
    Reference,
    /// Analyze the data
    Analyze,
    /// Run numerical lock-in analysis
    Li,
    /// Rotate the reference phase for lock-in analysis
    ///
    /// This command adjusts the reference phase used after the lock-in detection.
    /// You can either specify the phase angle manually (in rad),
    /// or let the program determine it automatically using an internal algorithm.
    Phase {
        /// Phase offset to apply (in rad).
        /// Positive values rotate the phase counterclockwise.
        #[arg(short, long, value_name = "RAD")]
        #[arg(conflicts_with = "auto")]
        #[arg(conflicts_with = "formula")]
        rad: f64,

        /// Automatically determine the optimal phase offset.
        /// If set, the `rad` option is ignored.
        #[arg(short, long)]
        #[arg(conflicts_with = "rad")]
        #[arg(conflicts_with = "formula")]
        auto: bool,

        /// Use formula-based phase adjustment.
        #[arg(short, long)]
        #[arg(conflicts_with = "rad")]
        #[arg(conflicts_with = "auto")]
        formula: bool,
    },
    /// Generate shell completion script
    Completions {
        /// Shell to generate for: bash, zsh, fish, powershell, elvish
        #[arg(value_enum)]
        shell: Shell,
    },
}
