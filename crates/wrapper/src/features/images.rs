//! Image generation for a drawing request. The OpenAI key stays in the account
//! vault and is sent only to the allowed image host. T1 does not validate the
//! closed aspect-ratio list or accept reference images.

use super::Context;
use crate::provider::error_parts;
use grokbot_attachments::staging::{self, MAX_IMAGE_BYTES};
use grokbot_engine::contracts::{ToolCall, ToolOutcome, ToolSpec};
use grokbot_forms::vault::Vault;
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;
use std::time::Duration;
use url::Url;

pub const TOOL: &str = "generate_image";
pub const INDICATOR: &str = "Génération d’une image…";
pub const CONNECT: &str = "Connecte ton compte OpenAI";
const SAFETY: &str = "La génération de cette image a été refusée pour des raisons de sécurité.";
const RESTRICTED: &str = "Ce modèle d’image est restreint pour ce compte.";
const MISSING: &str = "Décrivez l’image à générer.";
const FAILED: &str = "La génération de l’image a échoué.";
const UNUSABLE: &str = "L’image reçue n’est pas utilisable.";
const TOO_BIG: &str = "L’image générée dépasse 20 Mo et n’a pas été conservée.";
const UNAVAILABLE: &str = "Le service d’images est indisponible.";
const SECRET: &str = "OPENAI_API_KEY";
const PURPOSE: &str = "Compte OpenAI pour générer des images";
const IMAGE_HOST: &str = "https://api.openai.com/v1/images/generations";
/// Base64 of a 20 MiB image, plus a small JSON envelope.
const MAX_RESPONSE_BYTES: usize = MAX_IMAGE_BYTES * 4 / 3 + 1024 * 1024;

empty_hooks!(
    migrate,
    routes,
    state,
    tick,
    call_tool,
    context,
    entry,
    on_run_event,
    link_attachments
);

pub fn tools() -> Vec<ToolSpec> {
    vec![ToolSpec {
        name: TOOL.into(),
        description: "Appelle cet outil quand l’utilisateur demande un dessin ou une image à créer. description est la scène à dessiner. aspect_ratio est optionnel et vaut 1:1 s’il est omis. Ne l’utilise pas pour décrire une image déjà jointe. Aucune approbation n’est requise.".into(),
        schema: json!({"type":"object","additionalProperties":false,"properties":{"description":{"type":"string"},"aspect_ratio":{"type":"string"}},"required":["description"]}),
        requires_approval: false,
    }]
}

pub fn is_external(name: &str) -> bool {
    name == TOOL
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KeySource {
    Vault,
    Account,
    Missing,
}

/// Vault wins. The loopback provider never treats the chat bearer as an image key.
pub(crate) fn select_key(loopback: bool, vault_present: bool, account_present: bool) -> KeySource {
    if vault_present {
        KeySource::Vault
    } else if loopback || !account_present {
        KeySource::Missing
    } else {
        KeySource::Account
    }
}

/// Only the loopback test host or the production image host. ChatGPT’s chat
/// host is mapped onto api.openai.com and is not itself an image host.
pub(crate) fn image_endpoint(api: &str) -> Result<String, ()> {
    let url = Url::parse(api).map_err(|_| ())?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(());
    }
    let host = url.host_str().unwrap_or("");
    match (url.scheme(), host) {
        ("http", "127.0.0.1") => {
            let port = url.port().ok_or(())?;
            if !matches!(url.path(), "/v1" | "/v1/") {
                return Err(());
            }
            Ok(format!("http://127.0.0.1:{port}/v1/images/generations"))
        }
        ("https", "api.openai.com" | "chatgpt.com") if url.port().is_none() => {
            Ok(IMAGE_HOST.into())
        }
        _ => Err(()),
    }
}

pub fn execute(ctx: &Context<'_>, call: &ToolCall) -> ToolOutcome {
    let description = call.arguments["description"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_owned();
    if description.is_empty() {
        return failed(MISSING);
    }
    let aspect = call.arguments["aspect_ratio"]
        .as_str()
        .filter(|value| !value.is_empty())
        .unwrap_or("1:1");
    let Ok(endpoint) = image_endpoint(&ctx.provider.image_api_base()) else {
        return failed(UNAVAILABLE);
    };
    let key = match load_key(ctx, endpoint.starts_with("http://127.0.0.1:")) {
        Ok(Some(key)) => key,
        Ok(None) => return failed(CONNECT),
        Err(()) => return failed(UNAVAILABLE),
    };
    let (status, result) = post(&endpoint, &key, &description, aspect);
    let run_id = ctx.run_id.unwrap_or(TOOL);
    crate::upstream::turn(
        ctx.db_path.parent().unwrap_or(Path::new(".")),
        run_id,
        status,
        "image",
    );
    match result {
        Ok(bytes) => store(ctx, call, &bytes, aspect),
        Err(reason) => failed(reason),
    }
}

pub fn present(
    ctx: &Context<'_>,
    conversation: &str,
    message_id: &str,
    outcome: Option<&Value>,
    message: &mut crate::Message,
) {
    match outcome.and_then(|value| value["status"].as_str()) {
        None => {
            message.body = INDICATOR.into();
            message.attachments.clear();
        }
        Some("failed") => {
            message.body = outcome
                .and_then(|value| value["reason"].as_str())
                .unwrap_or(FAILED)
                .to_owned();
            message.attachments.clear();
        }
        Some("done") => {
            message.body.clear();
            message.attachments = super::attachments::for_message(ctx, conversation, message_id);
        }
        Some(_) => {
            message.body = FAILED.into();
            message.attachments.clear();
        }
    }
}

/// Drop the image key copied from the connected account. A key that was never
/// stored is left untouched, and sign-out does not create a vault file.
pub fn forget_account_key(db: &Path) {
    let path = db.with_extension("secrets.json");
    if !path.is_file() {
        return;
    }
    if let Ok(vault) = Vault::open_app(&path) {
        let _ = vault.remove(SECRET);
    }
}

fn load_key(ctx: &Context<'_>, loopback: bool) -> Result<Option<String>, ()> {
    let vault = Vault::open_app(&ctx.db_path.with_extension("secrets.json")).map_err(|_| ())?;
    let stored = vault.get(SECRET).map_err(|_| ())?;
    let account = ctx.provider.account_access_token();
    match select_key(loopback, stored.is_some(), account.is_some()) {
        KeySource::Vault => Ok(stored),
        KeySource::Missing => Ok(None),
        KeySource::Account => {
            let token = account.ok_or(())?;
            vault.set(SECRET, PURPOSE, &token).map_err(|_| ())?;
            Ok(Some(token))
        }
    }
}

fn post(
    endpoint: &str,
    key: &str,
    description: &str,
    aspect: &str,
) -> (u16, Result<Vec<u8>, &'static str>) {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(Duration::from_secs(60))
        .timeout_write(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .redirects(0)
        .build();
    let response = agent
        .post(endpoint)
        .set("Authorization", &format!("Bearer {key}"))
        .send_json(json!({
            "model": "gpt-image",
            "prompt": description,
            "aspect_ratio": aspect,
            "n": 1,
        }));
    let response = match response {
        Ok(response) => response,
        Err(ureq::Error::Status(_, response)) => response,
        Err(_) => return (0, Err(FAILED)),
    };
    let status = response.status();
    let mut body = Vec::new();
    if response
        .into_reader()
        .take(MAX_RESPONSE_BYTES as u64 + 1)
        .read_to_end(&mut body)
        .is_err()
        || body.len() > MAX_RESPONSE_BYTES
    {
        return (status, Err(FAILED));
    }
    (status, interpret(status, &body))
}

fn interpret(status: u16, body: &[u8]) -> Result<Vec<u8>, &'static str> {
    let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    if !(200..300).contains(&status) || value.get("error").is_some_and(|error| !error.is_null()) {
        return Err(phrase(&error_parts(&value).0));
    }
    let Some(encoded) = value["data"]
        .get(0)
        .and_then(|item| item["b64_json"].as_str())
    else {
        return Err(UNUSABLE);
    };
    let bytes = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded.trim())
        .map_err(|_| UNUSABLE)?;
    if bytes.is_empty() {
        return Err(UNUSABLE);
    }
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(TOO_BIG);
    }
    staging::sniff(&bytes).map_err(|_| UNUSABLE)?;
    Ok(bytes)
}

fn phrase(code: &str) -> &'static str {
    match code {
        "content_safety_blocked" => SAFETY,
        "model_restricted" => RESTRICTED,
        _ => FAILED,
    }
}

fn store(ctx: &Context<'_>, call: &ToolCall, bytes: &[u8], aspect: &str) -> ToolOutcome {
    let Some(conversation) = ctx.conversation_id else {
        return failed(FAILED);
    };
    let Ok(message_id) = tool_message_id(ctx, &call.call_id) else {
        return failed(FAILED);
    };
    let Ok(tx) = super::write_transaction(ctx.connection) else {
        return failed(FAILED);
    };
    let staged = match staging::stage(
        &tx,
        ctx.db_path,
        bytes,
        i64::try_from(ctx.now_ms).unwrap_or(0),
    ) {
        Ok(attachment) => attachment,
        Err(_) => return failed(FAILED),
    };
    if staging::link_attachments(
        &tx,
        conversation,
        &message_id,
        std::slice::from_ref(&staged.id),
    )
    .is_err()
    {
        return failed(FAILED);
    }
    if tx.commit().is_err() {
        return failed(FAILED);
    }
    ToolOutcome::Done {
        output: json!({
            "attachment_id": staged.id,
            "mime_type": staged.image_type.mime_type(),
            "aspect_ratio": aspect,
        }),
    }
}

fn tool_message_id(ctx: &Context<'_>, call_id: &str) -> Result<String, ()> {
    let run_id = ctx.run_id.ok_or(())?;
    let seq: i64 = ctx
        .connection
        .query_row(
            "SELECT seq FROM engine_run_events WHERE run_id=?1 AND kind='tool_call' AND json_extract(payload,'$.call_id')=?2",
            params![run_id, call_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| ())?
        .ok_or(())?;
    Ok(format!("{run_id}:{seq}"))
}

fn failed(reason: &str) -> ToolOutcome {
    ToolOutcome::Failed {
        reason: reason.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_accepts_only_loopback_and_the_production_image_host() {
        assert_eq!(
            image_endpoint("http://127.0.0.1:1455/v1").unwrap(),
            "http://127.0.0.1:1455/v1/images/generations"
        );
        assert_eq!(
            image_endpoint("https://api.openai.com/v1").unwrap(),
            IMAGE_HOST
        );
        assert_eq!(
            image_endpoint("https://chatgpt.com/backend-api/codex").unwrap(),
            IMAGE_HOST
        );
        for rejected in [
            "http://user:secret@127.0.0.1:1455/v1",
            "https://127.0.0.1:1455/v1",
            "http://api.openai.com/v1",
            "https://api.openai.com:8443/v1",
            "https://evil.example/v1",
            "http://127.0.0.1:1455/v1?x=1",
            "http://127.0.0.1/v1",
        ] {
            assert!(image_endpoint(rejected).is_err(), "{rejected}");
        }
    }

    #[test]
    fn key_source_keeps_the_chat_bearer_off_the_loopback() {
        assert_eq!(select_key(true, true, true), KeySource::Vault);
        assert_eq!(select_key(true, false, true), KeySource::Missing);
        assert_eq!(select_key(false, false, true), KeySource::Account);
        assert_eq!(select_key(false, true, false), KeySource::Vault);
        assert_eq!(select_key(false, false, false), KeySource::Missing);
    }

    #[test]
    fn provider_errors_become_french_and_a_url_is_not_fetched() {
        let safety = br#"{"error":{"code":"content_safety_blocked","message":"blocked"}}"#;
        assert_eq!(interpret(400, safety).unwrap_err(), SAFETY);
        let restricted = br#"{"error":{"code":"model_restricted","message":"restricted"}}"#;
        assert_eq!(interpret(400, restricted).unwrap_err(), RESTRICTED);
        let remote = br#"{"data":[{"url":"https://evil.example/secret.png"}]}"#;
        assert_eq!(interpret(200, remote).unwrap_err(), UNUSABLE);
        let garbage = br#"{"data":[{"b64_json":"bm90IGFuIGltYWdl"}]}"#;
        assert_eq!(interpret(200, garbage).unwrap_err(), UNUSABLE);
    }
}
