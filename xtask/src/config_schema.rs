use pmoke::config::{ConfigFieldDoc, ConfigReference};
use serde_json::{Map, Value, json};

pub fn build(reference: &ConfigReference) -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "$id": "https://kerr-group.github.io/pmoke/config.schema.json",
        "title": "pmoke config.toml",
        "description": format!("pmoke {} configuration schema v{}", reference.pmoke_version, reference.schema_version),
        "type": "object",
        "additionalProperties": false,
        "required": ["version", "scope", "data", "pulse", "reference", "lockin", "phase", "moke"],
        "properties": {
            "version": annotate(reference, "version", json!({"type": "integer", "enum": [6, 7]})),
            "scope": annotate(reference, "scope", instrument(reference, "scope", false)),
            "generator": annotate(reference, "generator", instrument(reference, "generator", true)),
            "data": annotate(reference, "data", object(
                &["output", "input"],
                [
                    ("output", enum_string(reference, "data.output")),
                    ("input", enum_string(reference, "data.input")),
                    ("screenshot", annotate(reference, "data.screenshot", json!({"type": "boolean", "default": false}))),
                ],
            )),
            "sensors": annotate(reference, "sensors", json!({
                "type": "array",
                "items": sensor(reference),
                "default": []
            })),
            "pulse": annotate(reference, "pulse", object(
                &["background_before", "background_after"],
                [
                    ("background_before", window(reference, "pulse.background_before")),
                    ("background_after", window(reference, "pulse.background_after")),
                ],
            )),
            "reference": annotate(reference, "reference", object(
                &["channel", "fft_window", "stride_samples", "window_samples"],
                [
                    ("channel", channel(reference, "reference.channel")),
                    ("fft_window", window(reference, "reference.fft_window")),
                    ("stride_samples", positive_integer(reference, "reference.stride_samples")),
                    ("window_samples", positive_integer(reference, "reference.window_samples")),
                ],
            )),
            "lockin": annotate(reference, "lockin", lockin(reference)),
            "phase": annotate(reference, "phase", object(
                &["offsets"],
                [("offsets", annotate(reference, "phase.offsets", json!({
                    "type": "array",
                    "minItems": 6,
                    "maxItems": 6,
                    "items": {"oneOf": [{"type": "number"}, {"type": "string"}]}
                })))],
            )),
            "moke": annotate(reference, "moke", object(
                &["sensor", "method", "factor"],
                [
                    ("sensor", channel(reference, "moke.sensor")),
                    ("method", enum_string(reference, "moke.method")),
                    ("factor", annotate(reference, "moke.factor", json!({"type": "number"}))),
                ],
            )),
            "signals": annotate(reference, "signals", json!({
                "type": "array",
                "items": signal(reference),
                "default": []
            })),
            "plot": annotate(reference, "plot", plot(reference)),
        },
        "x-pmoke": {
            "format_version": reference.format_version,
            "pmoke_version": reference.pmoke_version,
            "schema_version": reference.schema_version,
            "fields": reference.fields,
            "semantic_constraints": [
                "channel assignments must be unique across sensors, reference, lock-in signals, and signals",
                "moke.sensor must reference a configured sensor channel",
                "pulse background windows must not overlap",
                "v6 lockin.filter must use the active boxcar_legacy fields only",
                "v7 lockin selects either the filter contract or the window plus estimator contracts, never both",
                "joint_harmonic_gls requires fit/output harmonics, a noise mode, and one calibration digest per lock-in channel"
            ]
        }
    })
}

fn instrument(reference: &ConfigReference, prefix: &str, optional: bool) -> Value {
    let model = format!("{prefix}.model");
    let connection = format!("{prefix}.connection");
    let mut schema = object(
        &["model", "connection"],
        [
            (
                "model",
                annotate(reference, &model, json!({"type": "string", "minLength": 1})),
            ),
            (
                "connection",
                annotate(
                    reference,
                    &connection,
                    json!({
                        "type": "string",
                        "minLength": 1,
                        "pattern": "^(tcp://|visa:|gpib://|prologix-tcp://|prologix-serial://)"
                    }),
                ),
            ),
        ],
    );
    if optional {
        schema["description"] = Value::String(
            "Optional instrument table; required by commands that use the generator.".to_string(),
        );
    }
    schema
}

fn signal(reference: &ConfigReference) -> Value {
    object(
        &["channel", "label", "unit"],
        [
            ("channel", channel(reference, "signals[].channel")),
            (
                "label",
                annotate(
                    reference,
                    "signals[].label",
                    json!({"type": "string", "minLength": 1}),
                ),
            ),
            (
                "unit",
                annotate(
                    reference,
                    "signals[].unit",
                    json!({"type": "string", "minLength": 1}),
                ),
            ),
        ],
    )
}

fn sensor(reference: &ConfigReference) -> Value {
    object(
        &["channel", "scale", "label", "unit"],
        [
            ("channel", channel(reference, "sensors[].channel")),
            (
                "scale",
                annotate(
                    reference,
                    "sensors[].scale",
                    json!({
                        "oneOf": [
                            object(
                                &["factor"],
                                [("factor", annotate(reference, "sensors[].scale.factor", json!({
                                    "type": "number",
                                    "not": {"const": 0}
                                })))],
                            ),
                            object(
                                &["max_abs", "polarity"],
                                [
                                    ("max_abs", annotate(reference, "sensors[].scale.max_abs", json!({
                                        "type": "number",
                                        "exclusiveMinimum": 0
                                    }))),
                                    ("polarity", enum_integer(reference, "sensors[].scale.polarity")),
                                ],
                            )
                        ]
                    }),
                ),
            ),
            (
                "label",
                annotate(
                    reference,
                    "sensors[].label",
                    json!({"type": "string", "minLength": 1}),
                ),
            ),
            (
                "unit",
                annotate(
                    reference,
                    "sensors[].unit",
                    json!({"type": "string", "minLength": 1}),
                ),
            ),
        ],
    )
}

fn lockin(reference: &ConfigReference) -> Value {
    let mut schema = object(
        &["channels", "workers", "stride_samples"],
        [
            (
                "channels",
                annotate(
                    reference,
                    "lockin.channels",
                    json!({
                        "type": "array",
                        "minItems": 1,
                        "uniqueItems": true,
                        "items": {"type": "integer", "minimum": 1, "maximum": 8}
                    }),
                ),
            ),
            ("workers", positive_integer(reference, "lockin.workers")),
            (
                "stride_samples",
                positive_integer(reference, "lockin.stride_samples"),
            ),
            ("filter", filter(reference)),
            ("window", window_contract(reference)),
            ("estimator", estimator(reference)),
            (
                "debug_output",
                annotate(
                    reference,
                    "lockin.debug_output",
                    json!({"type": "boolean", "default": false}),
                ),
            ),
            (
                "debug_label",
                annotate(
                    reference,
                    "lockin.debug_label",
                    json!({
                        "type": "string",
                        "minLength": 1,
                        "maxLength": 64,
                        "pattern": "^(?!\\.{1,2}$)[A-Za-z0-9._-]+$"
                    }),
                ),
            ),
            (
                "debug_overwrite",
                annotate(
                    reference,
                    "lockin.debug_overwrite",
                    json!({"type": "boolean", "default": false}),
                ),
            ),
            (
                "snr_background_window",
                window(reference, "lockin.snr_background_window"),
            ),
            (
                "snr_signal_window",
                window(reference, "lockin.snr_signal_window"),
            ),
            (
                "save_npy",
                annotate(
                    reference,
                    "lockin.save_npy",
                    json!({"type": "boolean", "default": false}),
                ),
            ),
        ],
    );
    // v6 selects the filter contract, v7 selects window + estimator; mixing
    // both shapes matches neither branch exactly once and is rejected.
    schema["oneOf"] = json!([
        {"required": ["channels", "workers", "stride_samples", "filter"]},
        {"required": ["channels", "workers", "stride_samples", "window", "estimator"]},
    ]);
    schema
}

fn filter(reference: &ConfigReference) -> Value {
    let half_window = annotate(
        reference,
        "lockin.filter.half_window_cycles",
        json!({"type": "number", "exclusiveMinimum": 0}),
    );
    let kind = |value: &str| {
        annotate(
            reference,
            "lockin.filter.kind",
            json!({"type": "string", "const": value}),
        )
    };
    annotate(
        reference,
        "lockin.filter",
        json!({
            "oneOf": [
                object(
                    &["kind", "half_window_cycles"],
                    [("kind", kind("boxcar_legacy")), ("half_window_cycles", half_window)],
                )
            ]
        }),
    )
}

fn window_contract(reference: &ConfigReference) -> Value {
    annotate(
        reference,
        "lockin.window",
        object(
            &["kind", "half_window_cycles", "edge_policy"],
            [
                ("kind", enum_string(reference, "lockin.window.kind")),
                (
                    "half_window_cycles",
                    annotate(
                        reference,
                        "lockin.window.half_window_cycles",
                        json!({"type": "number", "exclusiveMinimum": 0}),
                    ),
                ),
                (
                    "edge_policy",
                    enum_string(reference, "lockin.window.edge_policy"),
                ),
            ],
        ),
    )
}

fn estimator(reference: &ConfigReference) -> Value {
    let legacy = object(
        &["kind"],
        [(
            "kind",
            annotate(
                reference,
                "lockin.estimator.kind",
                json!({"type": "string", "const": "boxcar_legacy"}),
            ),
        )],
    );
    let gls = object(
        &["kind", "fit_harmonics", "output_harmonics", "noise_mode"],
        [
            (
                "kind",
                annotate(
                    reference,
                    "lockin.estimator.kind",
                    json!({"type": "string", "const": "joint_harmonic_gls"}),
                ),
            ),
            (
                "fit_harmonics",
                annotate(
                    reference,
                    "lockin.estimator.fit_harmonics",
                    json!({
                        "type": "array",
                        "minItems": 1,
                        "uniqueItems": true,
                        "items": {"type": "integer", "minimum": 1}
                    }),
                ),
            ),
            (
                "output_harmonics",
                annotate(
                    reference,
                    "lockin.estimator.output_harmonics",
                    json!({
                        "type": "array",
                        "minItems": 6,
                        "maxItems": 6,
                        "items": {"type": "integer", "minimum": 1}
                    }),
                ),
            ),
            (
                "envelope_degree",
                annotate(
                    reference,
                    "lockin.estimator.envelope_degree",
                    json!({"type": "integer", "const": 0, "default": 0}),
                ),
            ),
            (
                "noise_mode",
                enum_string(reference, "lockin.estimator.noise_mode"),
            ),
            (
                "covariance_output",
                with_default(
                    enum_string(reference, "lockin.estimator.covariance_output"),
                    "diagonal",
                ),
            ),
            (
                "failure_policy",
                with_default(
                    enum_string(reference, "lockin.estimator.failure_policy"),
                    "error",
                ),
            ),
            (
                "calibrations",
                annotate(
                    reference,
                    "lockin.estimator.calibrations",
                    json!({
                        "type": "array",
                        "default": [],
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "required": ["channel", "path", "sha256"],
                            "properties": {
                                "channel": {"type": "integer", "minimum": 1, "maximum": 8},
                                "path": {"type": "string", "minLength": 1},
                                "sha256": {"type": "string", "pattern": "^[0-9a-f]{64}$"}
                            }
                        }
                    }),
                ),
            ),
        ],
    );
    annotate(
        reference,
        "lockin.estimator",
        json!({ "oneOf": [legacy, gls] }),
    )
}

fn plot(reference: &ConfigReference) -> Value {
    object(
        &[],
        [
            (
                "mode",
                with_default(enum_string(reference, "plot.mode"), "save"),
            ),
            (
                "output_dir",
                annotate(reference, "plot.output_dir", json!({"type": "string"})),
            ),
            (
                "max_points",
                with_default(positive_integer(reference, "plot.max_points"), 100_000),
            ),
            (
                "decimation",
                with_default(enum_string(reference, "plot.decimation"), "stride"),
            ),
            (
                "on_error",
                with_default(enum_string(reference, "plot.on_error"), "warn"),
            ),
        ],
    )
}

fn window(reference: &ConfigReference, path: &str) -> Value {
    annotate(
        reference,
        path,
        object(
            &["start", "end"],
            [
                ("start", json!({"type": "number"})),
                ("end", json!({"type": "number"})),
            ],
        ),
    )
}

fn channel(reference: &ConfigReference, path: &str) -> Value {
    annotate(
        reference,
        path,
        json!({"type": "integer", "minimum": 1, "maximum": 8}),
    )
}

fn positive_integer(reference: &ConfigReference, path: &str) -> Value {
    annotate(reference, path, json!({"type": "integer", "minimum": 1}))
}

fn enum_string(reference: &ConfigReference, path: &str) -> Value {
    let values = metadata(reference, path).valid_values;
    annotate(reference, path, json!({"type": "string", "enum": values}))
}

fn enum_integer(reference: &ConfigReference, path: &str) -> Value {
    let values = metadata(reference, path)
        .valid_values
        .iter()
        .map(|value| value.parse::<i64>().expect("integer config enum"))
        .collect::<Vec<_>>();
    annotate(reference, path, json!({"type": "integer", "enum": values}))
}

fn object<const N: usize>(required: &[&str], properties: [(&str, Value); N]) -> Value {
    let properties = properties
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect::<Map<_, _>>();
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": required,
        "properties": properties,
    })
}

fn annotate(reference: &ConfigReference, path: &str, mut schema: Value) -> Value {
    let metadata = metadata(reference, path);
    let object = schema.as_object_mut().expect("schema must be an object");
    object.insert(
        "title".to_string(),
        Value::String(metadata.summary_en.trim_end_matches('.').to_string()),
    );
    object.insert(
        "description".to_string(),
        Value::String(format!("{} {}", metadata.summary_en, metadata.details_en)),
    );
    object.insert("x-pmoke-path".to_string(), Value::String(path.to_string()));
    if let Some(units) = metadata.units {
        object.insert("x-units".to_string(), Value::String(units.to_string()));
    }
    if !metadata.constraints.is_empty() {
        object.insert("x-constraints".to_string(), json!(metadata.constraints));
    }
    schema
}

fn metadata<'a>(reference: &'a ConfigReference, path: &str) -> &'a ConfigFieldDoc {
    reference
        .fields
        .iter()
        .find(|field| field.path == path)
        .unwrap_or_else(|| panic!("missing config metadata for {path}"))
}

fn with_default(mut schema: Value, default: impl Into<Value>) -> Value {
    schema
        .as_object_mut()
        .expect("schema must be an object")
        .insert("default".to_string(), default.into());
    schema
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_schema_uses_registry_metadata() {
        let reference = pmoke::config::config_reference();
        let schema = build(&reference);
        assert_eq!(
            schema["properties"]["version"]["enum"],
            serde_json::json!([6, 7])
        );
        assert_eq!(
            schema["properties"]["lockin"]["properties"]["filter"]["oneOf"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(
            schema["properties"]["lockin"]["oneOf"]
                .as_array()
                .unwrap()
                .len()
                == 2
        );
        assert_eq!(
            schema["properties"]["lockin"]["properties"]["estimator"]["oneOf"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            schema["x-pmoke"]["fields"].as_array().unwrap().len(),
            reference.fields.len()
        );
    }
}
