//! Fail-closed redaction boundary, implemented by gb-forms-vault.
//! All model-facing outputs pass here; only SecretRef and status may survive.
use crate::vault::Vault;
use grokbot_engine::contracts::{pending, Result};
use serde_json::Value;

pub fn redact(vault: &Vault, _text: &str) -> Result<String> {
    let _values = vault.known_values()?;
    pending("forms.redact")
}
pub fn redact_value(vault: &Vault, _value: &Value) -> Result<Value> {
    let _values = vault.known_values()?;
    pending("forms.redact")
}
