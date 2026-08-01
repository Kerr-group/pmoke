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
    description: &'static str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct InstrumentDetails {
    model: &'static str,
    role: &'static str,
    transports: Vec<&'static str>,
    protocols: Vec<&'static str>,
    capabilities: Vec<&'static str>,
    examples: Vec<ConnectionExampleDetails>,
    description: &'static str,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
struct ConnectionExampleDetails {
    transport: &'static str,
    connection: &'static str,
    required_feature: &'static str,
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
            &["Model", "Role", "Transports", "Protocols", "Features"],
            instruments
                .iter()
                .map(|item| {
                    vec![
                        item.model.to_string(),
                        item.role.to_string(),
                        item.transports.join(", "),
                        item.protocols.join(", "),
                        item.required_features.join(", "),
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
        ],
    );
    if !details.examples.is_empty() {
        ui::section("Connection Examples");
        println!(
            "{}",
            ui::table(
                &["Transport", "Connection", "Feature"],
                details
                    .examples
                    .iter()
                    .map(|example| {
                        vec![
                            example.transport.to_string(),
                            example.connection.to_string(),
                            example.required_feature.to_string(),
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
        transports: spec
            .transports
            .iter()
            .map(|transport| transport.as_str())
            .collect(),
        protocols: spec
            .protocols
            .iter()
            .map(|protocol| protocol.as_str())
            .collect(),
        capabilities: spec
            .capabilities
            .iter()
            .map(|capability| capability.as_str())
            .collect(),
        required_features: required_features(spec),
        description: spec.description,
    }
}

fn details(spec: &InstrumentSpec) -> InstrumentDetails {
    InstrumentDetails {
        model: spec.model,
        role: spec.role.as_str(),
        transports: spec
            .transports
            .iter()
            .map(|transport| transport.as_str())
            .collect(),
        protocols: spec
            .protocols
            .iter()
            .map(|protocol| protocol.as_str())
            .collect(),
        capabilities: spec
            .capabilities
            .iter()
            .map(|capability| capability.as_str())
            .collect(),
        examples: spec
            .examples
            .iter()
            .map(|example| ConnectionExampleDetails {
                transport: example.transport.as_str(),
                connection: example.connection,
                required_feature: example.required_feature,
            })
            .collect(),
        description: spec.description,
    }
}

fn required_features(spec: &InstrumentSpec) -> Vec<&'static str> {
    spec.examples
        .iter()
        .map(|example| example.required_feature)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
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
    fn details_include_connection_examples() {
        let spec = instruments::registry::find_instrument("Keithley2010").unwrap();
        let details = details(spec);

        assert!(details.examples.iter().any(|example| {
            example.transport == "prologix_tcp"
                && example.connection == "prologix-tcp://10.249.11.17:1234?addr=17"
                && example.required_feature == "hw-prologix-tcp"
        }));
    }
}
