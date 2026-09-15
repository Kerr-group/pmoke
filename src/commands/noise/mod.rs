//! `pmoke noise` command surface (PN-M1, Issue #246).

pub mod diagnose;
pub mod plan;

use crate::cli::NoiseCommand;

/// Dispatches the `noise` subcommands.
pub fn run(command: &NoiseCommand) -> Result<(), anyhow::Error> {
    match command {
        NoiseCommand::Diagnose { request, output } => {
            diagnose::run_diagnose(request, output.as_deref())
        }
    }
}
