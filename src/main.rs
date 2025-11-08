mod cli;
mod commands;
mod config;
mod lockin;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};

fn main() -> Result<()> {
    let args = Cli::parse();

    let cfg = config::from_path(&args.config)?;

    lockin::reference::println_ref();

    match args.command {
        Some(Command::Show) => commands::show::run(&cfg),
        Some(Command::Single) => {}
        Some(Command::Trigger) => {}
        Some(Command::Shot) => {}
        Some(Command::Fetch) => {}
        Some(Command::Analyze) => {}
        Some(Command::Li) => {}
        Some(Command::Phase) => {}
        None => commands::show::run(&cfg),
    }

    Ok(())
}
