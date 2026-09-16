//! `pmoke noise` command surface (PN-M1/M2, Issue #246).

pub mod compare;
pub mod diagnose;
pub mod diagnostics;
pub mod plan;

use crate::cli::NoiseCommand;

/// Dispatches the `noise` subcommands.
pub fn run(command: &NoiseCommand) -> Result<(), anyhow::Error> {
    match command {
        NoiseCommand::Diagnose { request, output } => {
            diagnose::run_diagnose(request, output.as_deref())
        }
        NoiseCommand::Compare { request, output } => {
            compare::run_compare(request, output.as_deref(), None)
        }
    }
}
