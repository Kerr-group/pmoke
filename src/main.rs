mod cli;
mod commands;
mod communications;
mod config;
mod lockin;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Command};

fn main() -> Result<()> {
    let args = Cli::parse();

    let cfg = config::from_path(&args.config)?;

    match args.command {
        Some(Command::Show) => commands::show::show(&cfg),
        Some(Command::Single) => commands::single::single(&cfg),
        Some(Command::Trigger) => commands::trigger::trigger(&cfg),
        Some(Command::Autoshot) => commands::autoshot::autoshot(&cfg),
        Some(Command::Fetch) => commands::fetch::fetch(&cfg),
        Some(Command::Analyze) => Ok(()),
        Some(Command::Li) => Ok(()),
        Some(Command::Phase { rad, auto, formula }) => {
            if formula {
                println!("Displaying the formula used for phase rotation...");
                // show_formula();
            } else if auto {
                println!("Automatically determining optimal phase rotation...");
                // run_auto_phase_detection();
            } else {
                println!("Rotating phase by {:.2} rad...", rad);
                // rotate_phase(degree);
            }
            Ok(())
        }
        Some(Command::Completions { shell }) => {
            commands::completions::install_completion(shell)?;
            Ok(())
        }
        None => commands::show::show(&cfg),
    }
}
