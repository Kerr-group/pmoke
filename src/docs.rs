use crate::cli::Cli;
use clap::{Arg, ArgAction, Command, CommandFactory};
use serde::Serialize;

pub const DOCS_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct CliReference {
    pub format_version: u32,
    pub pmoke_version: &'static str,
    pub feature_set: &'static str,
    pub command: CommandDoc,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct CommandDoc {
    pub name: String,
    pub path: String,
    pub summary: String,
    pub required_feature: Option<&'static str>,
    pub arguments: Vec<ArgumentDoc>,
    pub subcommands: Vec<CommandDoc>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ArgumentDoc {
    pub id: String,
    pub kind: ArgumentKind,
    pub short: Option<char>,
    pub long: Option<String>,
    pub value_names: Vec<String>,
    pub help: String,
    pub required: bool,
    pub global: bool,
    pub repeatable: bool,
    pub default_values: Vec<String>,
    pub possible_values: Vec<String>,
    pub conflicts_with: Vec<String>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArgumentKind {
    Flag,
    Option,
    Positional,
}

pub fn cli_reference() -> CliReference {
    let mut root = Cli::command();
    root.build();
    CliReference {
        format_version: DOCS_FORMAT_VERSION,
        pmoke_version: env!("CARGO_PKG_VERSION"),
        feature_set: "hw-core",
        command: command_doc(&root, "pmoke"),
    }
}

fn command_doc(command: &Command, path: &str) -> CommandDoc {
    let arguments = command
        .get_arguments()
        .filter(|argument| {
            !argument.is_hide_set() && (path == "pmoke" || !argument.is_global_set())
        })
        .map(|argument| argument_doc(command, argument))
        .collect();
    let subcommands = command
        .get_subcommands()
        .filter(|subcommand| subcommand.get_name() != "help" && !subcommand.is_hide_set())
        .map(|subcommand| {
            let child_path = format!("{path} {}", subcommand.get_name());
            command_doc(subcommand, &child_path)
        })
        .collect();

    CommandDoc {
        name: command.get_name().to_string(),
        path: path.to_string(),
        summary: command
            .get_long_about()
            .or_else(|| command.get_about())
            .map(ToString::to_string)
            .unwrap_or_default(),
        required_feature: required_feature(path),
        arguments,
        subcommands,
    }
}

fn argument_doc(command: &Command, argument: &Arg) -> ArgumentDoc {
    let takes_values = argument.get_action().takes_values();
    let kind = if argument.is_positional() {
        ArgumentKind::Positional
    } else if takes_values {
        ArgumentKind::Option
    } else {
        ArgumentKind::Flag
    };
    let mut conflicts_with = command
        .get_arg_conflicts_with(argument)
        .into_iter()
        .map(|conflict| conflict.get_id().to_string())
        .collect::<Vec<_>>();
    conflicts_with.sort();
    conflicts_with.dedup();

    ArgumentDoc {
        id: argument.get_id().to_string(),
        kind,
        short: argument.get_short(),
        long: argument.get_long().map(str::to_string),
        value_names: if takes_values {
            argument
                .get_value_names()
                .unwrap_or_default()
                .iter()
                .map(ToString::to_string)
                .collect()
        } else {
            Vec::new()
        },
        help: argument
            .get_long_help()
            .or_else(|| argument.get_help())
            .map(ToString::to_string)
            .unwrap_or_default(),
        required: argument.is_required_set(),
        global: argument.is_global_set(),
        repeatable: matches!(argument.get_action(), ArgAction::Append | ArgAction::Count),
        default_values: if takes_values {
            argument
                .get_default_values()
                .iter()
                .map(|value| value.to_string_lossy().into_owned())
                .collect()
        } else {
            Vec::new()
        },
        possible_values: argument
            .get_possible_values()
            .into_iter()
            .filter(|value| !value.is_hide_set())
            .map(|value| value.get_name().to_string())
            .collect(),
        conflicts_with,
    }
}

fn required_feature(path: &str) -> Option<&'static str> {
    let command = path.split_whitespace().nth(1)?;
    match command {
        "single" | "trigger" | "autoshot" | "fetch" | "screenshot" | "automeasure" | "process"
        | "auto" => Some("hw-core"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_reference_contains_nested_commands_and_constraints() {
        let reference = cli_reference();
        assert_eq!(reference.pmoke_version, env!("CARGO_PKG_VERSION"));
        let config = reference
            .command
            .subcommands
            .iter()
            .find(|command| command.path == "pmoke config")
            .unwrap();
        assert!(
            config
                .subcommands
                .iter()
                .any(|command| command.path == "pmoke config validate")
        );
        let migrate = config
            .subcommands
            .iter()
            .find(|command| command.path == "pmoke config migrate")
            .unwrap();
        let output = migrate
            .arguments
            .iter()
            .find(|argument| argument.id == "output")
            .unwrap();
        assert!(output.conflicts_with.contains(&"in_place".to_string()));
    }

    #[test]
    fn hardware_commands_report_their_feature() {
        let reference = cli_reference();
        let fetch = reference
            .command
            .subcommands
            .iter()
            .find(|command| command.path == "pmoke fetch")
            .unwrap();
        assert_eq!(fetch.required_feature, Some("hw-core"));
    }

    #[test]
    fn serialized_cli_reference_is_deterministic() {
        let first = serde_json::to_string_pretty(&cli_reference()).unwrap();
        let second = serde_json::to_string_pretty(&cli_reference()).unwrap();
        assert_eq!(first, second);
    }
}
