mod analysis_results;
mod cli;
mod commands;
#[cfg(feature = "hw")]
mod communications;
pub mod config;
mod constants;
mod kerr;
pub mod lockin;
mod phase;
pub mod plot;
pub mod python;
#[cfg(test)]
mod test_support;
mod ui;
pub mod utils;

use anyhow::{Result, bail};
use clap::Parser;
use cli::{Cli, Command, ConfigCommand, ExportCommand, RawCommand};
use config::{ConfigLoad, ValidationTarget};

/// Parses command-line arguments and runs pmoke.
pub fn run() -> Result<()> {
    run_with(Cli::parse())
}

#[doc(hidden)]
pub fn run_analysis_pipeline(
    cfg: &config::Config,
    data: &utils::waveform::WaveformData,
) -> Result<()> {
    commands::analyze::run_analyze(cfg, data)
}

fn run_with(args: Cli) -> Result<()> {
    if let Some(Command::Completions { shell }) = args.command.as_ref() {
        commands::completions::install_completion(*shell)?;
        return Ok(());
    }

    if let Some(Command::Config { command }) = args.command.as_ref() {
        let check = matches!(command, ConfigCommand::Migrate { check: true, .. });
        match commands::config::run(&args.config, command) {
            Ok(outcome) if outcome.exit_code == 0 => return Ok(()),
            Ok(outcome) => std::process::exit(i32::from(outcome.exit_code)),
            Err(error) if check => {
                eprintln!("Config migration blocked: {error:#}");
                std::process::exit(2);
            }
            Err(error) => return Err(error),
        }
    }

    if let Some(Command::Raw {
        command: RawCommand::Verify { input: Some(input) },
    }) = args.command.as_ref()
    {
        return commands::raw::verify(input);
    }

    if let Some(Command::Export {
        command:
            ExportCommand::Csv {
                input: Some(input),
                output: Some(output),
            },
    }) = args.command.as_ref()
    {
        let mut load = config::load_from_path(&args.config);
        let mut cfg = match &mut load {
            ConfigLoad::Ready { config, .. } => config.clone(),
            ConfigLoad::Diagnostics(diag) => {
                if let Some(cfg) = &diag.normalized {
                    cfg.clone()
                } else {
                    return Err(anyhow::anyhow!("failed to parse configuration file"));
                }
            }
        };
        cfg.force = args.force;
        if let Some(run_dir) = &args.run_dir {
            cfg.set_artifact_root(run_dir.clone());
        }
        return commands::export::csv_with_canonical_lock(&cfg, input, output, args.force);
    }

    let mut load = config::load_from_path(&args.config);
    if let ConfigLoad::Ready { config, .. } = &mut load {
        config.force = args.force;
        if let Some(run_dir) = &args.run_dir {
            config.set_artifact_root(run_dir.clone());
        }
    }

    match args.command.as_ref() {
        Some(Command::Show) => return commands::show::show(&load),
        None | Some(Command::Monitor) => return commands::monitor::monitor(&args.config, load),
        _ => {}
    }

    if let (Some(Command::Doctor { json, .. }), ConfigLoad::Diagnostics(diagnostics)) =
        (args.command.as_ref(), &load)
    {
        return commands::doctor::run_diagnostics(diagnostics, *json);
    }

    if matches!(&load, ConfigLoad::Diagnostics(_)) {
        commands::show::show(&load)?;
        bail!("configuration is not runnable");
    }

    let (cfg, warnings) = load.into_ready()?;

    if let Some(Command::Raw { command }) = args.command.as_ref() {
        commands::show::print_warnings(&warnings);
        return commands::raw::run(&cfg, command);
    }
    if let Some(Command::Export { command }) = args.command.as_ref() {
        commands::show::print_warnings(&warnings);
        return commands::export::run(&cfg, command);
    }
    if let Some(Command::Doctor { json, probe_fetch }) = args.command.as_ref() {
        return commands::doctor::run(&cfg, &warnings, *json, *probe_fetch);
    }
    commands::show::print_warnings(&warnings);

    #[cfg(feature = "hw")]
    {
        match args.command.as_ref() {
            Some(
                Command::Show
                | Command::Monitor
                | Command::Config { .. }
                | Command::Raw { .. }
                | Command::Export { .. }
                | Command::Doctor { .. },
            ) => unreachable!(),
            Some(Command::Single) => {
                run_validated(&cfg, ValidationTarget::Single, commands::single::single)
            }
            Some(Command::Trigger) => {
                run_validated(&cfg, ValidationTarget::Trigger, commands::trigger::trigger)
            }
            Some(Command::Autoshot) => run_validated(
                &cfg,
                ValidationTarget::Autoshot,
                commands::autoshot::autoshot,
            ),
            Some(Command::Fetch { format, out }) => {
                config::validate_for_target(&cfg, ValidationTarget::Fetch)?;
                commands::fetch::fetch_with_options(&cfg, *format, out.as_deref())
            }
            Some(Command::Screenshot) => run_validated(
                &cfg,
                ValidationTarget::Screenshot,
                commands::screenshot::screenshot,
            ),
            Some(Command::Automeasure) => run_validated(
                &cfg,
                ValidationTarget::Automeasure,
                commands::automeasure::automeasure,
            ),
            Some(Command::Reference) => run_validated(
                &cfg,
                ValidationTarget::Reference,
                commands::reference::reference,
            ),
            Some(Command::Sensor) => {
                run_validated(&cfg, ValidationTarget::Sensor, commands::sensor::sensor)
            }
            Some(Command::Li) => run_validated(&cfg, ValidationTarget::Li, commands::li::li),
            Some(Command::Phase) => {
                run_validated(&cfg, ValidationTarget::Phase, commands::phase::phase)
            }
            Some(Command::Kerr) => {
                run_validated(&cfg, ValidationTarget::Kerr, commands::kerr::kerr)
            }
            Some(Command::Analyze) => {
                run_validated(&cfg, ValidationTarget::Analyze, commands::analyze::analyze)
            }
            Some(Command::Process) => {
                run_validated(&cfg, ValidationTarget::Process, commands::process::process)
            }
            Some(Command::Auto) => {
                run_validated(&cfg, ValidationTarget::Auto, commands::auto::auto)
            }
            Some(Command::Completions { .. }) => Ok(()),
            None => unreachable!(),
        }
    }

    #[cfg(not(feature = "hw"))]
    {
        match args.command.as_ref() {
            Some(
                Command::Show
                | Command::Monitor
                | Command::Config { .. }
                | Command::Raw { .. }
                | Command::Export { .. }
                | Command::Doctor { .. },
            ) => unreachable!(),
            Some(Command::Reference) => run_validated(
                &cfg,
                ValidationTarget::Reference,
                commands::reference::reference,
            ),
            Some(Command::Sensor) => {
                run_validated(&cfg, ValidationTarget::Sensor, commands::sensor::sensor)
            }
            Some(Command::Li) => run_validated(&cfg, ValidationTarget::Li, commands::li::li),
            Some(Command::Phase) => {
                run_validated(&cfg, ValidationTarget::Phase, commands::phase::phase)
            }
            Some(Command::Kerr) => {
                run_validated(&cfg, ValidationTarget::Kerr, commands::kerr::kerr)
            }
            Some(Command::Analyze) => {
                run_validated(&cfg, ValidationTarget::Analyze, commands::analyze::analyze)
            }
            Some(Command::Completions { .. }) => Ok(()),
            None => unreachable!(),
        }
    }
}

fn run_validated(
    cfg: &config::Config,
    target: ValidationTarget,
    command: impl FnOnce(&config::Config) -> Result<()>,
) -> Result<()> {
    config::validate_for_target(cfg, target)?;
    command(cfg)
}
