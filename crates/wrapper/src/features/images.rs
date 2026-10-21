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
    let key = match load_key(ctx) {
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

/// Drop the image key stored in the account vault. A key that was never stored
/// is left untouched, and sign-out does not create a vault file. A deletion
/// that cannot be confirmed is returned so sign-out is not reported as done
/// while the secret remains.
pub fn forget_account_key(db: &Path) -> Result<(), crate::ApiError> {
    let path = db.with_extension("secrets.json");
    if !path.is_file() {
        return Ok(());
    }
    let vault = Vault::open_app(&path)?;
    vault.remove(SECRET)?;
    Ok(())
}

/// The image key is the account vault secret. Auth files (`*.chatgpt-auth.json`,
/// `*.codex-auth.json`) are never read and their bearer is never sent.
fn load_key(ctx: &Context<'_>) -> Result<Option<String>, ()> {
    let vault = Vault::open_app(&ctx.db_path.with_extension("secrets.json")).map_err(|_| ())?;
    vault.get(SECRET).map_err(|_| ())
}

fn post(
    endpoint: &str,
    key: &str,
    description: &str,
    aspect: &str,
) -> (u16, Result<Vec<u8>, &'static str>) {
    #[cfg(test)]
    {
        tests::note_post(key);
        if tests::block_post() {
            return (0, Err(FAILED));
        }
    }
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
    use crate::features::Context;
    use crate::{Endpoints, Provider};
    use grokbot_engine::contracts::{ToolCall, ToolOutcome};
    use rusqlite::Connection;
    use serde_json::json;
    use std::cell::{Cell, RefCell};
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    thread_local! {
        static BLOCK_POST: Cell<bool> = const { Cell::new(false) };
        static POST_COUNT: Cell<u32> = const { Cell::new(0) };
        static POST_KEY: RefCell<String> = const { RefCell::new(String::new()) };
    }

    pub(super) fn note_post(key: &str) {
        POST_COUNT.with(|count| count.set(count.get() + 1));
        POST_KEY.with(|slot| *slot.borrow_mut() = key.to_owned());
    }

    pub(super) fn block_post() -> bool {
        BLOCK_POST.with(|block| block.get())
    }

    fn reset_posts() {
        BLOCK_POST.with(|block| block.set(true));
        POST_COUNT.with(|count| count.set(0));
        POST_KEY.with(|slot| slot.borrow_mut().clear());
    }

    fn post_count() -> u32 {
        POST_COUNT.with(|count| count.get())
    }

    fn post_key() -> String {
        POST_KEY.with(|slot| slot.borrow().clone())
    }

    const VAULT_KEY: &str = "vault-image-key-9f3c1a7e";
    const CHAT_TOKEN: &str = "chatgpt-auth-bearer-not-image";
    const CODEX_TOKEN: &str = "codex-auth-bearer-not-image";

    struct Outside {
        root: PathBuf,
        db: PathBuf,
        provider: Provider,
        connection: Connection,
    }

    impl Outside {
        fn open(with_vault_key: bool) -> Self {
            let root = std::env::temp_dir().join(format!(
                "ezerbot-image-key-{}-{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            std::fs::create_dir_all(&root).unwrap();
            let db = root.join("workspace.sqlite");
            let expires = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs()
                + 3600;
            std::fs::write(
                db.with_extension("chatgpt-auth.json"),
                json!({
                    "host_id": "test-host",
                    "client_id": "oaiapp_test",
                    "access_token": CHAT_TOKEN,
                    "refresh_token": "chatgpt-refresh-token",
                    "expires_at": expires,
                    "scopes": "chatgpt.tokens.use.direct"
                })
                .to_string(),
            )
            .unwrap();
            std::fs::write(
                db.with_extension("codex-auth.json"),
                json!({
                    "host_id": "test-host",
                    "client_id": "app_EMoamEEZ73f0CkXaXp7hrann",
                    "account_id": "ezerbot-only",
                    "access_token": CODEX_TOKEN,
                    "refresh_token": "codex-refresh-token",
                    "expires_at": expires
                })
                .to_string(),
            )
            .unwrap();
            if with_vault_key {
                Vault::open(&db.with_extension("secrets.json"))
                    .unwrap()
                    .set(SECRET, "Compte OpenAI pour générer des images", VAULT_KEY)
                    .unwrap();
            } else {
                Vault::open(&db.with_extension("secrets.json")).unwrap();
            }
            let provider = Provider::with_endpoints(&db, Endpoints::default(), false);
            provider.select_backend("codex").unwrap();
            Self {
                root,
                db,
                provider,
                connection: Connection::open_in_memory().unwrap(),
            }
        }

        fn ctx(&self) -> Context<'_> {
            Context {
                connection: &self.connection,
                provider: &self.provider,
                db_path: &self.db,
                now_ms: 0,
                bot_id: None,
                conversation_id: None,
                run_id: None,
            }
        }
    }

    impl Drop for Outside {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn secrets_text(db: &Path) -> String {
        std::fs::read_to_string(db.with_extension("secrets.json")).unwrap_or_default()
    }

    fn draw() -> ToolCall {
        ToolCall {
            call_id: "dehors".into(),
            name: TOOL.into(),
            arguments: json!({"description": "un phare au crépuscule"}),
        }
    }

    #[test]
    fn image_key_outside_loopback_is_read_from_the_account_vault() {
        reset_posts();
        let present = Outside::open(true);
        execute(&present.ctx(), &draw());
        assert_eq!(post_count(), 1, "la clé du trousseau n’a pas été utilisée");
        assert_eq!(post_key(), VAULT_KEY);
        let stored = secrets_text(&present.db);
        assert!(!stored.contains(CHAT_TOKEN));
        assert!(!stored.contains(CODEX_TOKEN));

        reset_posts();
        let absent = Outside::open(false);
        let outcome = execute(&absent.ctx(), &draw());
        assert_eq!(
            post_count(),
            0,
            "le jeton d’auth a été envoyé au fournisseur"
        );
        match outcome {
            ToolOutcome::Failed { reason } => assert_eq!(reason, CONNECT),
            other => panic!("résultat inattendu: {other:?}"),
        }
        let stored = secrets_text(&absent.db);
        assert!(
            !stored.contains(CHAT_TOKEN),
            "jeton chatgpt écrit hors du trousseau"
        );
        assert!(
            !stored.contains(CODEX_TOKEN),
            "jeton codex écrit hors du trousseau"
        );
    }

    #[test]
    fn refused_keychain_write_does_not_store_the_image_key_or_call_the_provider() {
        reset_posts();
        let outside = Outside::open(false);
        let path = outside.db.with_extension("secrets.json");
        let before = std::fs::read(&path).unwrap();
        let error = Vault::open_refusing_keychain(&path)
            .unwrap()
            .set(SECRET, "Compte OpenAI pour générer des images", VAULT_KEY)
            .unwrap_err();
        assert!(!error.to_string().contains(VAULT_KEY));
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let outcome = execute(&outside.ctx(), &draw());
        assert_eq!(
            post_count(),
            0,
            "fournisseur appelé après un refus du trousseau"
        );
        match outcome {
            ToolOutcome::Failed { reason } => assert_eq!(reason, CONNECT),
            other => panic!("résultat inattendu: {other:?}"),
        }
        let stored = secrets_text(&outside.db);
        assert!(!stored.contains(VAULT_KEY));
        assert!(!stored.contains(CHAT_TOKEN));
        assert!(!stored.contains(CODEX_TOKEN));
    }

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
