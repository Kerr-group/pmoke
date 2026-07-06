mod cli;
mod commands;
#[cfg(feature = "hw")]
mod communications;
mod config;
mod constants;
mod kerr;
mod lockin;
mod phase;
mod plot;
mod python;
mod ui;
mod utils;

use anyhow::Result;
use anyhow::bail;
use clap::Parser;
use cli::{Cli, Command};
use config::{ConfigLoad, ValidationTarget};

fn main() -> Result<()> {
    let args = Cli::parse();

    if let Some(Command::Completions { shell }) = args.command.as_ref() {
        commands::completions::install_completion(*shell)?;
        return Ok(());
    }

    let load = config::load_from_path(&args.config);

    match args.command.as_ref() {
        Some(Command::Show) => return commands::show::show(&load),
        None | Some(Command::Monitor) => return commands::monitor::monitor(&args.config, load),
        _ => {}
    }

    if matches!(&load, ConfigLoad::Diagnostics(_)) {
        commands::show::show(&load)?;
        bail!("configuration is not runnable");
    }

    let (cfg, warnings) = load.into_ready()?;
    commands::show::print_warnings(&warnings);

    #[cfg(feature = "hw")]
    {
        match args.command.as_ref() {
            Some(Command::Show) => unreachable!(),
            Some(Command::Monitor) => unreachable!(),
            Some(Command::Single) => {
                config::validate_for_target(&cfg, ValidationTarget::Single)?;
                commands::single::single(&cfg)
            }
            Some(Command::Trigger) => {
                config::validate_for_target(&cfg, ValidationTarget::Trigger)?;
                commands::trigger::trigger(&cfg)
            }
            Some(Command::Autoshot) => {
                config::validate_for_target(&cfg, ValidationTarget::Autoshot)?;
                commands::autoshot::autoshot(&cfg)
            }
            Some(Command::Fetch { format, out }) => {
                config::validate_for_target(&cfg, ValidationTarget::Fetch)?;
                commands::fetch::fetch_with_options(&cfg, *format, out.as_deref())
            }
            Some(Command::Image) => {
                config::validate_for_target(&cfg, ValidationTarget::Image)?;
                commands::image::image(&cfg)
            }
            Some(Command::Automeasure) => {
                config::validate_for_target(&cfg, ValidationTarget::Automeasure)?;
                commands::automeasure::automeasure(&cfg)
            }
            Some(Command::Reference) => {
                config::validate_for_target(&cfg, ValidationTarget::Reference)?;
                commands::reference::reference(&cfg)
            }
            Some(Command::Sensor) => {
                config::validate_for_target(&cfg, ValidationTarget::Sensor)?;
                commands::sensor::sensor(&cfg)
            }
            Some(Command::Li) => {
                config::validate_for_target(&cfg, ValidationTarget::Li)?;
                commands::li::li(&cfg)
            }
            Some(Command::Phase) => {
                config::validate_for_target(&cfg, ValidationTarget::Phase)?;
                commands::phase::phase(&cfg)
            }
            Some(Command::Kerr) => {
                config::validate_for_target(&cfg, ValidationTarget::Kerr)?;
                commands::kerr::kerr(&cfg)
            }
            Some(Command::Analyze) => {
                config::validate_for_target(&cfg, ValidationTarget::Analyze)?;
                commands::analyze::analyze(&cfg)
            }
            Some(Command::Process) => {
                config::validate_for_target(&cfg, ValidationTarget::Process)?;
                commands::process::process(&cfg)
            }
            Some(Command::Auto) => {
                config::validate_for_target(&cfg, ValidationTarget::Auto)?;
                commands::auto::auto(&cfg)
            }
            Some(Command::Completions { .. }) => Ok(()),
            None => unreachable!(),
        }
    }

    #[cfg(not(feature = "hw"))]
    {
        match args.command.as_ref() {
            Some(Command::Show) => unreachable!(),
            Some(Command::Monitor) => unreachable!(),
            Some(Command::Reference) => {
                config::validate_for_target(&cfg, ValidationTarget::Reference)?;
                commands::reference::reference(&cfg)
            }
            Some(Command::Sensor) => {
                config::validate_for_target(&cfg, ValidationTarget::Sensor)?;
                commands::sensor::sensor(&cfg)
            }
            Some(Command::Li) => {
                config::validate_for_target(&cfg, ValidationTarget::Li)?;
                commands::li::li(&cfg)
            }
            Some(Command::Phase) => {
                config::validate_for_target(&cfg, ValidationTarget::Phase)?;
                commands::phase::phase(&cfg)
            }
            Some(Command::Kerr) => {
                config::validate_for_target(&cfg, ValidationTarget::Kerr)?;
                commands::kerr::kerr(&cfg)
            }
            Some(Command::Analyze) => {
                config::validate_for_target(&cfg, ValidationTarget::Analyze)?;
                commands::analyze::analyze(&cfg)
            }
            Some(Command::Completions { .. }) => Ok(()),
            None => unreachable!(),
        }
    }
}
