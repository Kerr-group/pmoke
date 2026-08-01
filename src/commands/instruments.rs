use crate::cli::{InstrumentsCommand, JsonOutput};
use crate::ui;
use anyhow::{Result, bail};
use instruments::registry::{InstrumentSpec, KNOWN_INSTRUMENTS};
use serde::Serialize;
use std::collections::BTreeSet;

#[derive(Debug, Serialize, PartialEq, Eq)]
struct InstrumentListItem {
    model: &'static str,
    role: &'static str,
    transports: Vec<&'static str>,
    protocols: Vec<&'static str>,
    capabilities: Vec<&'static str>,
    required_features: Vec<&'static str>,
    notes: Vec<&'static str>,
    description: &'static str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct InstrumentDetails {
    model: &'static str,
    role: &'static str,
    transports: Vec<&'static str>,
    protocols: Vec<&'static str>,
    capabilities: Vec<&'static str>,
    notes: Vec<&'static str>,
    connection_templates: Vec<ConnectionTemplateDetails>,
    description: &'static str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct ConnectionTemplateDetails {
    transport: &'static str,
    connection_template: &'static str,
    required_feature: Option<&'static str>,
    feature_note: Option<&'static str>,
}

pub fn run(command: &InstrumentsCommand) -> Result<()> {
    match command {
        InstrumentsCommand::List(JsonOutput { json }) => list(*json),
        InstrumentsCommand::Explain { model, json } => explain(model, *json),
    }
}

fn list(json: bool) -> Result<()> {
    let instruments = KNOWN_INSTRUMENTS.iter().map(list_item).collect::<Vec<_>>();
    if json {
        println!("{}", serde_json::to_string_pretty(&instruments)?);
        return Ok(());
    }

    ui::section("Supported Instruments");
    println!(
        "{}",
        ui::table(
            &[
                "Model",
                "Role",
                "Transports",
                "Protocols",
                "Features",
                "Notes"
            ],
            instruments
                .iter()
                .map(|item| {
                    vec![
                        item.model.to_string(),
                        item.role.to_string(),
                        item.transports.join(", "),
                        item.protocols.join(", "),
                        item.required_features.join(", "),
                        display_list(&item.notes),
                    ]
                })
                .collect(),
        )
    );
    Ok(())
}

fn explain(model: &str, json: bool) -> Result<()> {
    let Some(spec) = find_instrument_fuzzy(model) else {
        let models = KNOWN_INSTRUMENTS
            .iter()
            .map(|spec| spec.model)
            .collect::<Vec<_>>()
            .join(", ");
        bail!("unknown instrument model '{model}'. Known models: {models}");
    };
    let details = details(spec);
    if json {
        println!("{}", serde_json::to_string_pretty(&details)?);
        return Ok(());
    }

    ui::settings_table(
        details.model,
        vec![
            ("role".to_string(), details.role.to_string()),
            ("description".to_string(), details.description.to_string()),
            ("transports".to_string(), details.transports.join(", ")),
            ("protocols".to_string(), details.protocols.join(", ")),
            ("capabilities".to_string(), details.capabilities.join(", ")),
            ("notes".to_string(), display_list(&details.notes)),
        ],
    );
    if !details.connection_templates.is_empty() {
        ui::section("Connection Templates");
        println!(
            "{}",
            ui::table(
                &["Transport", "Connection template", "Feature", "Note"],
                details
                    .connection_templates
                    .iter()
                    .map(|example| {
                        vec![
                            example.transport.to_string(),
                            example.connection_template.to_string(),
                            example.required_feature.unwrap_or("-").to_string(),
                            example.feature_note.unwrap_or("-").to_string(),
                        ]
                    })
                    .collect(),
            )
        );
    }
    Ok(())
}

fn find_instrument_fuzzy(model: &str) -> Option<&'static InstrumentSpec> {
    instruments::registry::find_instrument(model).or_else(|| {
        KNOWN_INSTRUMENTS
            .iter()
            .find(|spec| spec.model.eq_ignore_ascii_case(model))
    })
}

fn list_item(spec: &InstrumentSpec) -> InstrumentListItem {
    InstrumentListItem {
        model: spec.model,
        role: spec.role.as_str(),
        transports: transport_names(spec),
        protocols: protocol_names(spec),
        capabilities: capability_names(spec),
        required_features: required_features(spec),
        notes: transport_notes(spec),
        description: spec.description,
    }
}

fn details(spec: &InstrumentSpec) -> InstrumentDetails {
    InstrumentDetails {
        model: spec.model,
        role: spec.role.as_str(),
        transports: transport_names(spec),
        protocols: protocol_names(spec),
        capabilities: capability_names(spec),
        notes: transport_notes(spec),
        connection_templates: connection_templates(spec),
        description: spec.description,
    }
}

fn transport_names(spec: &InstrumentSpec) -> Vec<&'static str> {
    spec.transports
        .iter()
        .map(|transport| transport.as_str())
        .collect()
}

fn protocol_names(spec: &InstrumentSpec) -> Vec<&'static str> {
    spec.protocols
        .iter()
        .map(|protocol| protocol.as_str())
        .collect()
}

fn capability_names(spec: &InstrumentSpec) -> Vec<&'static str> {
    spec.capabilities
        .iter()
        .map(|capability| capability.as_str())
        .collect()
}

fn connection_templates(spec: &InstrumentSpec) -> Vec<ConnectionTemplateDetails> {
    spec.transports
        .iter()
        .map(|transport| ConnectionTemplateDetails {
            transport: transport.as_str(),
            connection_template: transport.connection_template(),
            required_feature: transport.required_feature(),
            feature_note: transport.feature_note(),
        })
        .collect()
}

fn required_features(spec: &InstrumentSpec) -> Vec<&'static str> {
    spec.transports
        .iter()
        .filter_map(|transport| transport.required_feature())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn transport_notes(spec: &InstrumentSpec) -> Vec<&'static str> {
    spec.transports
        .iter()
        .filter_map(|transport| transport.feature_note())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn display_list(values: &[&str]) -> String {
    if values.is_empty() {
        "-".to_string()
    } else {
        values.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_includes_registry_metadata() {
        let item = list_item(instruments::registry::find_instrument("Keithley2010").unwrap());

        assert_eq!(item.role, "multimeter");
        assert!(item.transports.contains(&"prologix_tcp"));
        assert!(item.protocols.contains(&"scpi"));
        assert!(item.capabilities.contains(&"scpi_identify"));
    }

    #[test]
    fn explain_accepts_case_insensitive_model_names() {
        let spec = find_instrument_fuzzy("keithley2010").unwrap();

        assert_eq!(spec.model, "Keithley2010");
    }

    #[test]
    fn details_generate_connection_templates_from_transports() {
        let spec = instruments::registry::find_instrument("Keithley2010").unwrap();
        let details = details(spec);

        assert!(details.connection_templates.iter().any(|example| {
            example.transport == "prologix_tcp"
                && example.connection_template == "prologix-tcp://<host>:1234?addr=<addr>"
                && example.required_feature == Some("hw-prologix-tcp")
        }));
    }

    #[test]
    fn usbtmc_notes_are_separate_from_feature_names() {
        let spec = instruments::registry::find_instrument("DHO5108").unwrap();
        let item = list_item(spec);
        let details = details(spec);

        assert!(item.required_features.contains(&"hw-gpib"));
        assert!(!item.required_features.contains(&"hw-gpib on Windows"));
        assert_eq!(item.notes, vec!["Windows + NI-VISA"]);
        assert!(details.connection_templates.iter().any(|example| {
            example.transport == "usbtmc"
                && example.required_feature == Some("hw-gpib")
                && example.feature_note == Some("Windows + NI-VISA")
        }));
    }

    #[test]
    fn dummy_instrument_has_no_required_feature_name() {
        let spec = instruments::registry::find_instrument("DummyInstrument").unwrap();
        let item = list_item(spec);
        let details = details(spec);

        assert!(item.required_features.is_empty());
        assert!(
            details.connection_templates.iter().any(|example| {
                example.transport == "dummy" && example.required_feature.is_none()
            })
        );
    }
}
