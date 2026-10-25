//! Validateur du sous-ensemble de JSON Schema que nos outils utilisent.
//! Les mots-clés hors de `VALIDATED` sont ignorés, jamais une cause de refus.
use serde_json::Value;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaError {
    /// Pointeur JSON : "" pour la racine, "/date", "/items/2/to".
    pub path: String,
    pub message: String,
}

const VALIDATED: &[&str] = &[
    "type",
    "required",
    "properties",
    "additionalProperties",
    "enum",
    "const",
    "items",
    "minItems",
    "maxItems",
    "minLength",
    "maxLength",
    "minimum",
    "maximum",
];
const ANNOTATIONS: &[&str] = &["description", "title", "default", "examples", "$schema"];

pub fn validate(schema: &Value, value: &Value) -> Vec<SchemaError> {
    let mut errors = Vec::new();
    walk(schema, value, "", &mut errors);
    errors
}

pub fn unsupported_keywords(schema: &Value) -> Vec<String> {
    let mut found = Vec::new();
    collect_unsupported(schema, "", &mut found);
    found.sort();
    found
}

/// Cuts serialized JSON at `max` characters, and appends "…" when it cut. A cut inside a
/// string token goes back to before its opening quote, so a string is echoed whole or
/// not at all: a prefix of a forgotten text would escape the exact-match scrub.
pub fn cut_json(json: &str, max: usize) -> String {
    let mut string_start = None;
    let mut escaped = false;
    for (count, (at, ch)) in json.char_indices().enumerate() {
        if count == max {
            return format!("{}…", &json[..string_start.unwrap_or(at)]);
        }
        match (string_start, ch) {
            (Some(_), _) if escaped => escaped = false,
            (Some(_), '\\') => escaped = true,
            (Some(_), '"') => string_start = None,
            (None, '"') => string_start = Some(at),
            _ => {}
        }
    }
    json.to_owned()
}

/// `Value`'s own equality tells `3` from `3.0`; JSON Schema does not.
fn same_json(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => a.as_f64() == b.as_f64(),
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same_json(a, b))
        }
        (Value::Object(a), Value::Object(b)) => {
            a.len() == b.len()
                && a.iter()
                    .all(|(k, a)| b.get(k).is_some_and(|b| same_json(a, b)))
        }
        _ => a == b,
    }
}

fn child(pointer: &str, key: &str) -> String {
    format!("{pointer}/{}", key.replace('~', "~0").replace('/', "~1"))
}

fn push(errors: &mut Vec<SchemaError>, path: String, message: impl Into<String>) {
    errors.push(SchemaError {
        path,
        message: message.into(),
    });
}

fn type_matches(kind: &str, value: &Value) -> bool {
    match kind {
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => {
            value.is_i64() || value.is_u64() || value.as_f64().is_some_and(|n| n.fract() == 0.0)
        }
        "boolean" => value.is_boolean(),
        "array" => value.is_array(),
        "object" => value.is_object(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn type_label(kind: &str) -> &str {
    match kind {
        "string" => "texte",
        "number" => "nombre",
        "integer" => "nombre entier",
        "boolean" => "booléen",
        "array" => "liste",
        "object" => "objet",
        other => other,
    }
}

fn walk(schema: &Value, value: &Value, pointer: &str, errors: &mut Vec<SchemaError>) {
    let Some(schema) = schema.as_object() else {
        return;
    };
    let kinds: Vec<&str> = match schema.get("type") {
        Some(Value::String(kind)) => vec![kind],
        Some(Value::Array(kinds)) => kinds.iter().filter_map(Value::as_str).collect(),
        _ => vec![],
    };
    if !kinds.is_empty() && !kinds.iter().any(|kind| type_matches(kind, value)) {
        let expected: Vec<_> = kinds.iter().map(|kind| type_label(kind)).collect();
        let received = cut_json(&value.to_string(), 60);
        let message = format!("{} attendu, reçu {received}", expected.join(" ou "));
        return push(errors, pointer.to_owned(), message);
    }
    let not_enumerated = schema
        .get("enum")
        .and_then(Value::as_array)
        .is_some_and(|allowed| !allowed.iter().any(|one| same_json(one, value)));
    if not_enumerated
        || schema
            .get("const")
            .is_some_and(|expected| !same_json(expected, value))
    {
        push(errors, pointer.to_owned(), "valeur non autorisée");
    }
    let bound = |key: &str| schema.get(key).and_then(Value::as_u64);
    let size = match value {
        Value::String(text) => Some((text.chars().count(), "caractères", "minLength", "maxLength")),
        Value::Array(items) => Some((items.len(), "éléments", "minItems", "maxItems")),
        _ => None,
    };
    if let Some((size, unit, min, max)) = size.map(|(n, unit, min, max)| (n as u64, unit, min, max))
    {
        if let Some(min) = bound(min).filter(|min| size < *min) {
            push(errors, pointer.to_owned(), format!("au moins {min} {unit}"));
        }
        if let Some(max) = bound(max).filter(|max| size > *max) {
            push(errors, pointer.to_owned(), format!("au plus {max} {unit}"));
        }
    }
    if let Some(number) = value.as_f64() {
        let limit = |key: &str| schema.get(key).and_then(Value::as_f64);
        if let Some(min) = limit("minimum").filter(|min| number < *min) {
            push(errors, pointer.to_owned(), format!("valeur minimale {min}"));
        }
        if let Some(max) = limit("maximum").filter(|max| number > *max) {
            push(errors, pointer.to_owned(), format!("valeur maximale {max}"));
        }
    }
    let item_schema = schema
        .get("items")
        .filter(|_| !schema.contains_key("prefixItems"));
    if let (Some(items), Some(item_schema)) = (value.as_array(), item_schema) {
        for (index, item) in items.iter().enumerate() {
            walk(
                item_schema,
                item,
                &child(pointer, &index.to_string()),
                errors,
            );
        }
    }
    if let Some(fields) = value.as_object() {
        let required = schema.get("required").and_then(Value::as_array);
        for name in required.into_iter().flatten().filter_map(Value::as_str) {
            if !fields.contains_key(name) {
                push(errors, child(pointer, name), "champ requis manquant");
            }
        }
        let properties = schema.get("properties").and_then(Value::as_object);
        let closed = schema.get("additionalProperties") == Some(&Value::Bool(false))
            && !schema.contains_key("patternProperties");
        for (name, field) in fields {
            match properties.and_then(|properties| properties.get(name)) {
                Some(field_schema) => walk(field_schema, field, &child(pointer, name), errors),
                None if closed => push(errors, child(pointer, name), "champ non autorisé"),
                None => {}
            }
        }
    }
}

fn collect_unsupported(schema: &Value, location: &str, found: &mut Vec<String>) {
    let Some(schema) = schema.as_object() else {
        return;
    };
    for (keyword, value) in schema {
        let ignored = match keyword.as_str() {
            "additionalProperties" => !value.is_boolean(),
            "items" => !value.is_object(),
            keyword => !VALIDATED.contains(&keyword) && !ANNOTATIONS.contains(&keyword),
        };
        if ignored {
            found.push(format!("{location}/{keyword}"));
        }
    }
    if let Some(properties) = schema.get("properties").and_then(Value::as_object) {
        for (name, property) in properties {
            collect_unsupported(property, &format!("{location}/properties/{name}"), found);
        }
    }
    if let Some(items) = schema.get("items") {
        collect_unsupported(items, &format!("{location}/items"), found);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn paths(schema: Value, value: Value) -> Vec<String> {
        validate(&schema, &value)
            .into_iter()
            .map(|e| e.path)
            .collect()
    }

    #[test]
    fn paths_are_json_pointers_with_escaped_keys_and_array_indexes() {
        let schema = json!({"type":"object", "properties":{
            "a/b~c": {"type":"integer"},
            "items": {"type":"array", "items":{"type":"object", "required":["to"]}}}});
        let value = json!({"a/b~c":"x", "items":[{"to":"a"}, {"to":"b"}, {}]});
        assert_eq!(paths(schema, value), ["/a~1b~0c", "/items/2/to"]);
    }

    #[test]
    fn integer_accepts_a_zero_fraction_and_number_accepts_both() {
        let integer = json!({"type":"integer"});
        assert!(validate(&integer, &json!(3.0)).is_empty());
        assert_eq!(paths(integer, json!(1.5)), [""]);
        assert!(validate(&json!({"type":["number","null"]}), &json!(1.5)).is_empty());
        assert!(validate(&json!({"enum":[3, [1]]}), &json!(3.0)).is_empty());
        assert!(validate(&json!({"enum":[3, [1]]}), &json!([1.0])).is_empty());
        assert!(validate(&json!({"const":{"n":3}}), &json!({"n":3.0})).is_empty());
        assert_eq!(paths(json!({"const":3}), json!(3.5)), [""]);
    }

    #[test]
    fn every_validated_keyword_rejects_a_violation_and_a_bare_schema_accepts_all() {
        let cases = [
            (json!({"type":"string"}), json!(1)),
            (json!({"required":["a"]}), json!({})),
            (
                json!({"properties":{"a":{"type":"string"}}}),
                json!({"a":1}),
            ),
            (json!({"additionalProperties":false}), json!({"a":1})),
            (json!({"enum":["a"]}), json!("b")),
            (json!({"const":"a"}), json!("b")),
            (json!({"items":{"type":"string"}}), json!([1])),
            (json!({"minItems":2}), json!([1])),
            (json!({"maxItems":1}), json!([1, 2])),
            (json!({"minLength":2}), json!("é")),
            (json!({"maxLength":1}), json!("éé")),
            (json!({"minimum":2}), json!(1)),
            (json!({"maximum":1}), json!(2)),
        ];
        for keyword in VALIDATED {
            assert!(
                cases
                    .iter()
                    .any(|(schema, _)| schema.get(*keyword).is_some()),
                "no case for {keyword}"
            );
        }
        for (schema, value) in cases {
            assert_eq!(validate(&schema, &value).len(), 1, "{schema}");
        }
        assert!(validate(&json!(true), &json!(1)).is_empty());
    }

    #[test]
    fn unsupported_keywords_walk_properties_and_items_and_skip_annotations() {
        let schema = json!({"type":"object", "title":"t", "$schema":"x", "properties":{
            "when": {"type":"string", "format":"date"},
            "list": {"type":"array", "items":{"oneOf":[]}}}});
        assert_eq!(
            unsupported_keywords(&schema),
            ["/properties/list/items/oneOf", "/properties/when/format"]
        );
    }

    #[test]
    fn a_string_crossing_the_cut_is_dropped_whole_and_short_strings_are_kept() {
        let long = "é".repeat(80);
        let json = json!({"a":"court", "b":long, "c":1}).to_string();
        assert_eq!(cut_json(&json, 60), r#"{"a":"court","b":…"#);
        assert_eq!(cut_json(&json, 1_000), json);
        let escaped = json!({"a":"x\"y\\"}).to_string();
        assert_eq!(cut_json(&escaped, 9), r#"{"a":…"#);
    }
}
