//! The canonical form `JsonSchema` stores for the JSON Schema input an MCP
//! server may send: `type` arrays and boolean subschemas.
use dim_agent_types::JsonSchema;
use serde_json::{Value, json};

fn schema(value: Value) -> Value {
    let schema: JsonSchema = serde_json::from_value(value.clone())
        .unwrap_or_else(|error| panic!("{value} rejected: {error}"));
    serde_json::to_value(schema).unwrap()
}

fn rejected(value: Value) -> String {
    serde_json::from_value::<JsonSchema>(value.clone())
        .map(|_| panic!("{value} accepted"))
        .unwrap_err()
        .to_string()
}

#[test]
fn a_type_array_becomes_any_of() {
    assert_eq!(
        schema(json!({
            "type": "object",
            "properties": {"space_id": {"type": ["string", "null"]}},
            "required": ["space_id"]
        })),
        json!({
            "type": "object",
            "properties": {"space_id": {"anyOf": [{"type": "string"}, {"type": "null"}]}},
            "required": ["space_id"]
        })
    );
    assert_eq!(
        schema(json!({"type": ["string", "integer"]})),
        json!({"anyOf": [{"type": "string"}, {"type": "integer"}]})
    );
}

#[test]
fn one_name_keeps_type() {
    assert_eq!(
        schema(json!({"type": ["string"]})),
        json!({"type": "string"})
    );
}

#[test]
fn a_type_array_beside_any_of_goes_under_all_of() {
    assert_eq!(
        schema(json!({
            "type": ["string", "null"],
            "anyOf": [{"minLength": 1}, {"pattern": "^a"}]
        })),
        json!({
            "anyOf": [{"minLength": 1}, {"pattern": "^a"}],
            "allOf": [{"anyOf": [{"type": "string"}, {"type": "null"}]}]
        })
    );
}

#[test]
fn a_type_array_appends_to_an_existing_all_of() {
    assert_eq!(
        schema(json!({
            "type": ["string", "null"],
            "anyOf": [{"minLength": 1}],
            "allOf": [{"maxLength": 5}]
        })),
        json!({
            "anyOf": [{"minLength": 1}],
            "allOf": [
                {"maxLength": 5},
                {"anyOf": [{"type": "string"}, {"type": "null"}]}
            ]
        })
    );
}

#[test]
fn a_type_array_without_any_of_keeps_all_of_a_sibling() {
    assert_eq!(
        schema(json!({"type": ["string", "null"], "allOf": [{"maxLength": 5}]})),
        json!({
            "allOf": [{"maxLength": 5}],
            "anyOf": [{"type": "string"}, {"type": "null"}]
        })
    );
}

#[test]
fn boolean_subschemas_become_schemas() {
    assert_eq!(
        schema(json!({"properties": {"x": true}, "items": false})),
        json!({"properties": {"x": {}}, "items": {"not": {}}})
    );
}

#[test]
fn keywords_the_struct_does_not_type_stay_verbatim() {
    let unchanged = json!({
        "type": "object",
        "$defs": {"Nullable": {"type": ["string", "null"]}},
        "properties": {
            "rule": {"anyOf": [{"type": ["string", "null"]}, {"const": "manual"}]}
        },
        "additionalProperties": false
    });
    assert_eq!(schema(unchanged.clone()), unchanged);
}

#[test]
fn the_canonical_form_round_trips() {
    let stored = schema(json!({
        "properties": {
            "space_id": {"type": ["string", "null"]},
            "tags": {"type": "array", "items": {"type": ["string", "null"]}}
        }
    }));
    assert_eq!(schema(stored.clone()), stored);
}

#[test]
fn an_empty_type_array_is_rejected() {
    assert_eq!(
        rejected(json!({"properties": {"x": {"type": []}}})),
        "inputSchema.properties.x.type: a type array must name a type"
    );
}

#[test]
fn a_repeated_type_name_is_rejected() {
    assert_eq!(
        rejected(json!({"type": ["string", "string"]})),
        "inputSchema.type[1]: duplicate type name `string`"
    );
}

#[test]
fn an_unknown_type_name_is_rejected_with_its_path() {
    let message = rejected(json!({
        "type": "object",
        "properties": {"x": {"type": ["string", "file"]}}
    }));
    assert_eq!(
        message,
        "inputSchema.properties.x.type[1]: unknown variant `file`, expected one of `object`, `array`, `string`, `number`, `integer`, `boolean`, `null`"
    );
}

#[test]
fn shapes_outside_the_standard_are_still_rejected() {
    assert_eq!(
        rejected(json!({"items": [{"type": "string"}]})),
        "inputSchema.items: invalid type: sequence, expected a schema object or a boolean"
    );
    assert_eq!(
        rejected(json!({"properties": {"x": null}})),
        "inputSchema.properties.x: invalid type: null, expected a schema object or a boolean"
    );
    assert_eq!(
        rejected(json!({"properties": null})),
        "inputSchema.properties: invalid type: null, expected a map"
    );
    assert_eq!(
        rejected(json!({"items": null})),
        "inputSchema.items: invalid type: null, expected a schema object or a boolean"
    );
    assert_eq!(
        rejected(json!({"required": null})),
        "inputSchema.required: invalid type: null, expected a sequence"
    );
    assert_eq!(
        rejected(json!({"description": null})),
        "inputSchema.description: invalid type: null, expected a string"
    );
    assert_eq!(
        rejected(json!({"type": "any"})),
        "inputSchema.type: unknown variant `any`, expected one of `object`, `array`, `string`, `number`, `integer`, `boolean`, `null`"
    );
    assert_eq!(
        rejected(json!(true)),
        "inputSchema: invalid type: boolean, expected a JSON Schema object"
    );
}
