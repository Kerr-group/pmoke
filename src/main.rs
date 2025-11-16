mod cli;
mod commands;
mod communications;
mod config;
mod constants;
mod kerr;
mod lockin;
mod phase;
mod utils;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};

fn main() -> Result<()> {
    let args = Cli::parse();

    if let Some(Command::Completions { shell }) = args.command.as_ref() {
        commands::completions::install_completion(*shell)?;
        return Ok(());
    }

    let cfg = config::from_path(&args.config)?;

    match args.command {
        Some(Command::Show) => commands::show::show(&cfg),
        Some(Command::Single) => commands::single::single(&cfg),
        Some(Command::Trigger) => commands::trigger::trigger(&cfg),
        Some(Command::Autoshot) => commands::autoshot::autoshot(&cfg),
        Some(Command::Fetch) => commands::fetch::fetch(&cfg),
        Some(Command::Automeasure) => commands::automeasure::automeasure(&cfg),
        Some(Command::Reference) => commands::reference::reference(&cfg),
        Some(Command::Sensor) => commands::sensor::sensor(&cfg),
        Some(Command::Li) => commands::li::li(&cfg),
        Some(Command::Phase) => commands::phase::phase(&cfg),
        Some(Command::Kerr) => commands::kerr::kerr(&cfg),
        Some(Command::Analyze) => commands::analyze::analyze(&cfg),
        Some(Command::Process) => commands::process::process(&cfg),
        Some(Command::Auto) => commands::auto::auto(&cfg),
        Some(Command::Completions { .. }) => Ok(()),
        None => commands::show::show(&cfg),
    }
}
