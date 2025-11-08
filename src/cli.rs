use clap::{Parser, Subcommand};

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
    Shot,
    /// Fetch data from the oscilloscope and save to a file
    Fetch,
    /// Analyze the data
    Analyze,
    /// Run numerical lock-in analysis
    Li,
    /// Rotate phase of lock-in analysis
    Phase,
}
