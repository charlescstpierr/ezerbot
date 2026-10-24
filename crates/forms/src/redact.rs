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
use std::path::{Path, PathBuf};

// Shorter in UTF-8 bytes than *any* valid secret, so repeated masking converges
// even if replacement boundaries accidentally form another known secret.
pub const MASK: &str = "[...]";

pub fn redact(vault: &Vault, text: &str) -> Result<String> {
    Patterns::of(vault)?.text(text)
}

/// The secrets known at one instant, read from the vault once. Redacting many
/// fragments of one rendering (a state read lists hundreds of replies) with the
/// same snapshot costs one vault read instead of one per fragment; the snapshot
/// is as fail-closed as `redact`, since taking it fails when the vault does.
pub struct Patterns(Vec<String>);

impl Patterns {
    pub fn of(vault: &Vault) -> Result<Self> {
        known_patterns(vault).map(Self)
    }
    pub fn text(&self, text: &str) -> Result<String> {
        Ok(mask(text, &self.0))
    }
    pub fn value(&self, value: &Value) -> Result<Value> {
        mask_value(value, &self.0)
    }
}

/// A rendering made of many masked outputs (a state read, #377) masks them all with one
/// content of the vault. While a `Hold` lives, the first read of a vault on this thread is
/// kept, and every later redaction of that vault on this thread (`redact`, `redact_value`,
/// `Patterns::of`, whatever the caller) reuses it. Nothing else changes: outside a hold every
/// redaction still reads the vault. A hold taken inside another one joins it: the outer
/// hold keeps the reads and checks them.
pub fn hold() -> Hold {
    let outermost = HELD.with_borrow_mut(|held| {
        let outermost = held.is_none();
        held.get_or_insert_with(Vec::new);
        outermost
    });
    Hold { outermost }
}

/// See `hold`. Dropping it, even while a panic unwinds, ends the hold.
pub struct Hold {
    outermost: bool,
}

impl Hold {
    /// Ends the hold and reads every vault it kept again: true when each still holds exactly
    /// what was used to mask. False means a secret was stored or removed meanwhile, and what
    /// was masked during the hold must be rendered again. Nothing read means nothing masked.
    pub fn settled(self) -> Result<bool> {
        if !self.outermost {
            return Ok(true);
        }
        let kept = HELD.take().unwrap_or_default();
        for held in &kept {
            if read_patterns(&held.vault)? != held.patterns {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        if self.outermost {
            HELD.take();
        }
    }
}

struct Held {
    vault: Vault,
    patterns: Vec<String>,
    /// The `app_patterns` paths that opened this vault during the hold.
    opened_as: Vec<PathBuf>,
}

/// `Patterns::of(&Vault::open_app(path)?)`. Under a hold, a vault already read through this
/// path is not opened again: opening locks the file and lists its directory, which a state
/// read would otherwise do once per reply of the history.
pub fn app_patterns(path: &Path) -> Result<Patterns> {
    let kept = HELD.with_borrow(|held| {
        held.as_ref()?
            .iter()
            .find(|read| read.opened_as.iter().any(|opened| opened == path))
            .map(|read| read.patterns.clone())
    });
    if let Some(patterns) = kept {
        return Ok(Patterns(patterns));
    }
    let vault = Vault::open_app(path)?;
    let patterns = known_patterns(&vault)?;
    HELD.with_borrow_mut(|held| {
        if let Some(read) = held
            .as_mut()
            .and_then(|kept| kept.iter_mut().find(|read| read.vault.same_store(&vault)))
        {
            read.opened_as.push(path.to_owned());
        }
    });
    Ok(Patterns(patterns))
}

thread_local! {
    static HELD: std::cell::RefCell<Option<Vec<Held>>> = const { std::cell::RefCell::new(None) };
}
/// Redact a cumulative live snapshot and withhold enough trailing characters
/// to cover a secret split across reads, including its JSON-escaped form.
/// Publish the complete text with `redact` once the stream finishes.
pub fn redact_live(vault: &Vault, text: &str) -> Result<String> {
    let values = known_patterns(vault)?;
    Ok(mask_live(text, &values))
}

fn mask_live(text: &str, values: &[String]) -> String {
    let mut output = mask(text, values);
    let tail = values
        .iter()
        .map(|value| value.chars().count())
        .max()
        .unwrap_or(0);
    let count = output.chars().count().saturating_sub(tail);
    let end = output
        .char_indices()
        .nth(count)
        .map_or(output.len(), |(index, _)| index);
    output.truncate(end);
    output
}

/// Retains every observed pattern for the lifetime of cumulative task output.
#[derive(Default)]
pub struct SnapshotRedactor {
    known: BTreeSet<String>,
}

impl SnapshotRedactor {
    pub fn redact(&mut self, vault: &Vault, value: &Value, live: bool) -> Result<Value> {
        // Always refresh, failing closed even when older patterns are available.
        self.known.extend(known_patterns(vault)?);
        let patterns: Vec<_> = self.known.iter().cloned().collect();
        let mut value = value.clone();
        if live {
            for field in ["stdout", "stderr"] {
                if let Some(text) = value["output"][field].as_str() {
                    value["output"][field] = Value::String(mask_live(text, &patterns));
                }
            }
        }
        mask_value(&value, &patterns)
    }
}

pub fn redact_value(vault: &Vault, value: &Value) -> Result<Value> {
    // One fresh snapshot per output; never continue with an empty set on an
    // unavailable/corrupt vault, even if this particular output looks harmless.
    Patterns::of(vault)?.value(value)
}

/// What the vault holds now, or what it held at its first read under the current `hold`.
fn known_patterns(vault: &Vault) -> Result<Vec<String>> {
    HELD.with_borrow_mut(|held| {
        let Some(kept) = held else {
            return read_patterns(vault);
        };
        if let Some(read) = kept.iter().find(|read| read.vault.same_store(vault)) {
            return Ok(read.patterns.clone());
        }
        let patterns = read_patterns(vault)?;
        kept.push(Held {
            vault: vault.handle(),
            patterns: patterns.clone(),
            opened_as: Vec::new(),
        });
        Ok(patterns)
    })
}

fn read_patterns(vault: &Vault) -> Result<Vec<String>> {
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
        let mut mask_end = 0;
        for (start, _) in result.char_indices() {
            // Site fields can be shorter than MASK. Never mask the replacement
            // itself, or a field such as "." would prevent convergence.
            if result[start..].starts_with(MASK) {
                mask_end = start + MASK.len();
            }
            let end = values
                .iter()
                .filter(|value| {
                    !value.is_empty()
                        && (start >= mask_end || value.len() > MASK.len())
                        && result[start..].starts_with(value.as_str())
                })
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

    #[test]
    fn a_snapshot_masks_like_redact_and_fails_closed_when_taken_without_vault() {
        let test = TestVault::new();
        test.vault.set("TOKEN", "", "private-value").unwrap();
        let snapshot = Patterns::of(&test.vault).unwrap();
        let value = json!({"a": ["x private-value y"], "b": "private-value"});
        assert_eq!(
            snapshot.text("a private-value b").unwrap(),
            redact(&test.vault, "a private-value b").unwrap()
        );
        assert_eq!(
            snapshot.value(&value).unwrap(),
            redact_value(&test.vault, &value).unwrap()
        );
        // The snapshot is a point in time; a new one sees what was added since.
        test.vault.set("OTHER", "", "another-value").unwrap();
        assert_eq!(snapshot.text("another-value").unwrap(), "another-value");
        assert_eq!(
            Patterns::of(&test.vault)
                .unwrap()
                .text("another-value")
                .unwrap(),
            MASK
        );
        std::fs::write(test.vault.path(), b"broken").unwrap();
        assert!(Patterns::of(&test.vault).is_err());
    }

    #[test]
    fn a_hold_masks_with_its_first_read_and_knows_when_the_vault_moved() {
        let test = TestVault::new();
        test.vault.set("TOKEN", "", "private-value").unwrap();
        // Nothing read under a hold: nothing was masked, nothing to check.
        assert!(hold().settled().unwrap());
        // Every redaction under the hold, whatever its entry point, uses the first read.
        let held = hold();
        assert_eq!(redact(&test.vault, "private-value").unwrap(), MASK);
        test.vault.set("OTHER", "", "another-value").unwrap();
        assert_eq!(
            redact(&test.vault, "another-value").unwrap(),
            "another-value"
        );
        assert_eq!(
            redact_value(&test.vault, &json!("another-value")).unwrap(),
            json!("another-value")
        );
        // A hold taken inside joins it and leaves the check to the outer one.
        assert!(hold().settled().unwrap());
        assert_eq!(
            redact(&test.vault, "another-value").unwrap(),
            "another-value"
        );
        assert!(
            !held.settled().unwrap(),
            "a secret was stored during the hold"
        );
        // The hold is over: redaction reads the vault again.
        assert_eq!(redact(&test.vault, "another-value").unwrap(), MASK);
        // The same content stored again is no change.
        let held = hold();
        redact(&test.vault, "").unwrap();
        test.vault.set("OTHER", "", "another-value").unwrap();
        assert!(held.settled().unwrap());
        // A hold dropped by a panic ends too.
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _held = hold();
            redact(&test.vault, "").unwrap();
            panic!("rendering failed");
        }));
        assert!(unwound.is_err());
        test.vault.set("THIRD", "", "third-value").unwrap();
        assert_eq!(redact(&test.vault, "third-value").unwrap(), MASK);
        // Under a hold, a vault read through its path is not opened again (opening would fail
        // here), and the check still sees the change.
        let held = hold();
        assert_eq!(
            app_patterns(test.vault.path())
                .unwrap()
                .text("third-value")
                .unwrap(),
            MASK
        );
        let content = std::fs::read(test.vault.path()).unwrap();
        std::fs::write(test.vault.path(), b"broken").unwrap();
        let reused = app_patterns(test.vault.path());
        std::fs::write(test.vault.path(), content).unwrap();
        assert_eq!(reused.unwrap().text("third-value").unwrap(), MASK);
        test.vault.set("FOURTH", "", "fourth-value").unwrap();
        assert_eq!(
            app_patterns(test.vault.path())
                .unwrap()
                .text("fourth-value")
                .unwrap(),
            "fourth-value"
        );
        assert!(!held.settled().unwrap());
        assert_eq!(
            app_patterns(test.vault.path())
                .unwrap()
                .text("fourth-value")
                .unwrap(),
            MASK
        );
        // Fail closed: a vault unreadable at the check refuses the rendering.
        let held = hold();
        redact(&test.vault, "").unwrap();
        std::fs::write(test.vault.path(), b"broken").unwrap();
        assert!(held.settled().is_err());
    }
}
