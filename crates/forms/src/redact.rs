//! Fail-closed, exact-value redaction for model-facing text and JSON. The caller
//! must apply this boundary before *persisting* any model-visible text (tool
//! outputs, events, summaries) and again before emitting provider input. Emission-
//! time redaction alone does not cover rotated or removed values.
//! Apply `redact_value` before JSON serialization; use `redact` for unencoded
//! text. Single JSON-escaped values are also masked as defense in depth, not as
//! a general decoder for arbitrary encodings or repeated serialization.
//! PS11 (`reports/goal-permissions-secrets.md`) only establishes targeted Grok
//! sanitizers; this broader boundary is the plan's defense-in-depth choice.
use crate::vault::Vault;
use grokbot_engine::contracts::{Error, Result};
use serde_json::Value;
use std::collections::BTreeSet;

// Shorter in UTF-8 bytes than *any* valid secret, so repeated masking converges
// even if replacement boundaries accidentally form another known secret.
const MASK: &str = "[...]";

pub fn redact(vault: &Vault, text: &str) -> Result<String> {
    let values = known_patterns(vault)?;
    Ok(mask(text, &values))
}
pub fn redact_value(vault: &Vault, value: &Value) -> Result<Value> {
    // One fresh snapshot per output; never continue with an empty set on an
    // unavailable/corrupt vault, even if this particular output looks harmless.
    let values = known_patterns(vault)?;
    mask_value(value, &values)
}

fn known_patterns(vault: &Vault) -> Result<Vec<String>> {
    let mut patterns = BTreeSet::new();
    for value in vault.known_values()? {
        let encoded = serde_json::to_string(&value)?;
        // The serializer always adds ASCII quotes around a String. Retain the
        // interior so a tool result containing serialized JSON is also masked.
        patterns.insert(encoded[1..encoded.len() - 1].to_owned());
        patterns.insert(value);
    }
    Ok(patterns.into_iter().collect())
}

fn mask(text: &str, values: &[String]) -> String {
    let mut result = text.to_owned();
    loop {
        let mut output = String::new();
        let mut covered_end = 0;
        let mut matched = false;
        // Check every character boundary, including starts *inside* an earlier
        // match, to mask the union of overlapping secrets rather than leak tails.
        for (start, _) in result.char_indices() {
            let end = values
                .iter()
                .filter(|value| !value.is_empty() && result[start..].starts_with(value.as_str()))
                .map(|value| start + value.len())
                .max();
            if let Some(end) = end {
                if start >= covered_end {
                    output.push_str(&result[covered_end..start]);
                    output.push_str(MASK);
                }
                covered_end = covered_end.max(end);
                matched = true;
            }
        }
        if !matched {
            return result;
        }
        output.push_str(&result[covered_end..]);
        result = output;
    }
}

fn mask_value(value: &Value, values: &[String]) -> Result<Value> {
    Ok(match value {
        Value::String(text) => Value::String(mask(text, values)),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| mask_value(item, values))
                .collect::<Result<_>>()?,
        ),
        Value::Object(object) => {
            let mut output = serde_json::Map::new();
            for (key, value) in object {
                let key = mask(key, values);
                if output.insert(key, mask_value(value, values)?).is_some() {
                    // Silently collapsing keys could change tool semantics.
                    return Err(Error::InvalidInput(
                        "secret redaction would collide JSON keys".into(),
                    ));
                }
            }
            Value::Object(output)
        }
        Value::Number(number) => {
            let original = number.to_string();
            let redacted = mask(&original, values);
            if original == redacted {
                value.clone()
            } else {
                Value::String(redacted)
            }
        }
        Value::Null | Value::Bool(_) => value.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vault::test_support::TestVault;
    use serde_json::json;

    #[test]
    fn connector_envelopes_also_mask_long_values_at_the_global_boundary() {
        let test = TestVault::new();
        test.vault
            .set(
                "MCP_SERVER_ENV",
                crate::vault::MCP_ENV_PURPOSE,
                &json!({"variables":{"TOKEN":"global-private-key-777","SHORT":"x"}}).to_string(),
            )
            .unwrap();
        assert_eq!(
            redact(&test.vault, "Préfixe global-private-key-777 suffixe").unwrap(),
            "Préfixe [...] suffixe"
        );
        assert_eq!(
            redact_value(&test.vault, &json!({"result":"global-private-key-777"})).unwrap(),
            json!({"result":"[...]"})
        );
        // Short opaque flags cannot safely participate in global substring
        // redaction; the connector host masks them in server-controlled data.
        assert_eq!(redact(&test.vault, "texte x").unwrap(), "texte x");
    }
    #[test]
    fn redact_replaces_every_known_value() {
        let test = TestVault::new();
        let secrets = [
            "abcd1234",
            "abcd1234-long",
            "é🔒quoted\"\\value",
            "line-one\nline-two",
        ];
        for (index, secret) in secrets.iter().enumerate() {
            test.vault
                .set(&format!("TOKEN_{index}"), "", secret)
                .unwrap();
        }
        let text = format!(
            "before {} {} after {}",
            secrets.join(" / "),
            secrets[0],
            secrets[1]
        );
        let redacted = redact(&test.vault, &text).unwrap();
        for secret in secrets {
            assert!(!redacted.contains(secret));
        }
        assert!(!redacted.contains("-long"));
        assert!(redacted.starts_with("before "));
        assert!(redacted.ends_with(MASK));
        assert_eq!(redact(&test.vault, &redacted).unwrap(), redacted);
    }

    #[test]
    fn redact_masks_overlaps_and_replacement_boundary_matches() {
        let test = TestVault::new();
        for (name, value) in [
            ("FIRST", "abcdefgh"),
            ("SECOND", "defghijk"),
            ("THIRD", "[...]XYZ"),
        ] {
            test.vault.set(name, "", value).unwrap();
        }
        assert_eq!(redact(&test.vault, "abcdefghijk").unwrap(), MASK);
        assert_eq!(redact(&test.vault, "abcdefghXYZ").unwrap(), MASK);
        test.vault.set("REPEATED", "", "aaaaaaaa").unwrap();
        assert_eq!(redact(&test.vault, "aaaaaaaaaaa").unwrap(), MASK);
    }

    #[test]
    fn redact_value_masks_nested_strings_keys_and_numeric_secrets() {
        let test = TestVault::new();
        let secret = "quote\"slash\\newline\n🔒";
        test.vault.set("TOKEN", "", secret).unwrap();
        test.vault.set("NUMBER", "", "12345678").unwrap();
        let input = json!({
            "nested": [secret, { "content": format!("before {secret} after") }],
            secret: "value",
            "numeric": 12345678,
            "unchanged": [false, null, 42]
        });
        let redacted = redact_value(&test.vault, &input).unwrap();
        assert_eq!(redacted["nested"][0], MASK);
        assert_eq!(
            redacted["nested"][1]["content"],
            format!("before {MASK} after")
        );
        assert_eq!(redacted[MASK], "value");
        assert_eq!(redacted["numeric"], MASK);
        assert_eq!(redacted["unchanged"], input["unchanged"]);
        assert_eq!(input["nested"][0], secret); // input is not mutated
        let serialized = serde_json::to_string(&redacted).unwrap();
        assert!(!serialized.contains("12345678"));
        assert!(!serialized.contains("quote\\\"slash"));
    }

    #[test]
    fn redact_masks_json_escaped_secrets_in_text_and_nested_outputs() {
        let test = TestVault::new();
        let pem = "-----BEGIN PRIVATE KEY-----\nMIIEvQ-test-material\n-----END PRIVATE KEY-----";
        let password = "pa\"ss\\word99";
        test.vault.set("KEY", "", pem).unwrap();
        test.vault.set("PASSWORD", "", password).unwrap();
        let tool_output = json!({ "key": pem, "pw": password });
        let encoded = serde_json::to_string(&tool_output).unwrap();
        let sanitized = redact(&test.vault, &encoded).unwrap();
        let decoded: Value = serde_json::from_str(&sanitized).unwrap();
        assert_eq!(decoded, json!({ "key": MASK, "pw": MASK }));
        let nested =
            redact_value(&test.vault, &json!({ "function_call_output": encoded })).unwrap();
        let nested_decoded: Value =
            serde_json::from_str(nested["function_call_output"].as_str().unwrap()).unwrap();
        assert_eq!(nested_decoded, decoded);
        for secret in [pem, password] {
            let escaped = serde_json::to_string(secret).unwrap();
            assert!(!sanitized.contains(&escaped[1..escaped.len() - 1]));
        }
    }

    #[test]
    fn redact_before_persistence_survives_rotation_and_removal() {
        let test = TestVault::new();
        test.vault.set("TOKEN", "", "old-token-value").unwrap();
        let persisted = redact(&test.vault, "output: old-token-value").unwrap();
        test.vault.set("TOKEN", "", "new-token-value").unwrap();
        test.vault.remove("TOKEN").unwrap();
        assert_eq!(redact(&test.vault, &persisted).unwrap(), "output: [...]");
    }

    #[test]
    fn redact_value_refuses_key_collisions() {
        let test = TestVault::new();
        test.vault.set("TOKEN", "", "abcdefgh").unwrap();
        assert!(redact_value(&test.vault, &json!({ "abcdefgh": 1, MASK: 2 })).is_err());
    }

    #[test]
    fn redact_reads_latest_values_and_fails_closed_without_vault() {
        let test = TestVault::new();
        assert_eq!(redact(&test.vault, "unchanged").unwrap(), "unchanged");
        test.vault.set("TOKEN", "", "private-value").unwrap();
        assert_eq!(redact(&test.vault, "private-value").unwrap(), MASK);
        std::fs::write(test.vault.path(), b"broken").unwrap();
        assert!(redact(&test.vault, "private-value").is_err());
        assert!(redact_value(&test.vault, &json!({})).is_err());
        std::fs::remove_file(test.vault.path()).unwrap();
        assert!(redact(&test.vault, "").is_err());
    }
}
