//! Image generation for a drawing request. The call uses the current ChatGPT
//! session on the Codex responses route and never an account-vault secret.
//! The frame is one of a closed list and a reference is an image already
//! attached to the thread; anything else is refused before any call.

use super::Context;
use crate::provider::error_parts;
use grokbot_attachments::staging::{self, MAX_IMAGE_BYTES};
use grokbot_engine::contracts::{ToolCall, ToolOutcome, ToolSpec};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use url::Url;

pub const TOOL: &str = "generate_image";
pub const INDICATOR: &str = "Génération d’une image…";
pub const CONNECT: &str = "Connectez votre compte ChatGPT";
const SAFETY: &str = "La génération de cette image a été refusée pour des raisons de sécurité.";
const RESTRICTED: &str = "Ce modèle d’image est restreint pour ce compte.";
const MISSING: &str = "Décrivez l’image à générer.";
const FAILED: &str = "La génération de l’image a échoué.";
const UNUSABLE: &str = "L’image reçue n’est pas utilisable.";
const TOO_BIG: &str = "L’image générée dépasse 20 Mo et n’a pas été conservée.";
const UNAVAILABLE: &str = "Le service d’images est indisponible.";
const BAD_FRAME: &str =
    "Ce cadre n’est pas pris en charge. Choisissez 1:1, 16:9, 9:16, 4:3 ou 3:4.";
const BAD_REFERENCE: &str = "Une image de référence doit être jointe à cette conversation.";
const TOO_MANY_REFERENCES: &str = "Utilisez au plus 10 images de référence.";
const MAX_REFERENCES: usize = 10;
const MODEL: &str = "gpt-image-1.5";
const IMAGE_HOST: &str = "https://chatgpt.com/backend-api/codex/responses";
const ORIGINATOR: &str = "ezerbot";
/// GPT Image can take about two minutes. The read and the whole call share this budget.
const GENERATION_TIMEOUT: Duration = Duration::from_secs(180);
/// The finished base64 image appears in both output_item.done and response.completed.
/// Keep the decoded-image limit separate from this bounded transport budget.
const MAX_RESPONSE_BYTES: usize = 2 * MAX_IMAGE_BYTES.div_ceil(3) * 4 + 1024 * 1024;

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
        description: "Appelle cet outil quand l’utilisateur demande un dessin ou une image à créer. description est la scène à dessiner. aspect_ratio est optionnel, vaut 1:1 s’il est omis, et se limite à 1:1, 16:9, 9:16, 4:3 ou 3:4. reference_image_paths est optionnel : identifiants des images déjà jointes à cette conversation, à imiter pour le style. Appelle list_delegation_context pour les connaître. Ne l’utilise pas pour décrire une image déjà jointe. Aucune approbation n’est requise.".into(),
        schema: json!({"type":"object","additionalProperties":false,"properties":{"description":{"type":"string"},"aspect_ratio":{"type":"string"},"reference_image_paths":{"type":"array","items":{"type":"string"}}},"required":["description"]}),
        requires_approval: false,
    }]
}

pub fn is_external(name: &str) -> bool {
    name == TOOL
}

/// Loopback tests and the production Codex host. The Platform images API is
/// refused: a ChatGPT session bearer has no `api.model.images.request` scope.
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
            if !matches!(
                url.path(),
                "/v1" | "/v1/" | "/backend-api/codex" | "/backend-api/codex/"
            ) {
                return Err(());
            }
            Ok(format!(
                "http://127.0.0.1:{port}/backend-api/codex/responses"
            ))
        }
        ("https", "chatgpt.com")
            if url.port().is_none()
                && matches!(url.path(), "/backend-api/codex" | "/backend-api/codex/") =>
        {
            Ok(IMAGE_HOST.into())
        }
        _ => Err(()),
    }
}

/// Sizes of the closed frame list; an omitted frame is square.
const FRAMES: [&str; 5] = ["1:1", "16:9", "9:16", "4:3", "3:4"];

pub(crate) fn image_size(aspect: &str) -> &'static str {
    match aspect {
        "16:9" | "4:3" => "1536x1024",
        "9:16" | "3:4" => "1024x1536",
        _ => "1024x1024",
    }
}

pub fn execute(ctx: &Context<'_>, call: &ToolCall, cancel: &AtomicBool) -> ToolOutcome {
    let description = call.arguments["description"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_owned();
    if description.is_empty() {
        return failed(MISSING);
    }
    let aspect = match call.arguments["aspect_ratio"].as_str() {
        None | Some("") => "1:1",
        Some(frame) if FRAMES.contains(&frame) => frame,
        Some(_) => return failed(BAD_FRAME),
    };
    let size = image_size(aspect);
    let references = match references(ctx, &call.arguments["reference_image_paths"]) {
        Ok(references) => references,
        Err(reason) => return failed(reason),
    };
    let Ok(endpoint) = image_endpoint(&ctx.provider.image_api_base()) else {
        return failed(UNAVAILABLE);
    };
    let (token, headers) = match ctx.provider.image_auth() {
        Ok(auth) => auth,
        Err(failure) if failure.kind == "signin" || failure.kind == "signin_expired" => {
            return failed(CONNECT);
        }
        Err(_) => return failed(UNAVAILABLE),
    };
    let (status, result) = post(
        &endpoint,
        &token,
        &headers,
        ctx.provider.image_responses_model(),
        &description,
        &references,
        size,
    );
    let run_id = ctx.run_id.unwrap_or(TOOL);
    crate::upstream::turn(
        ctx.db_path.parent().unwrap_or(Path::new(".")),
        run_id,
        status,
        "image",
    );
    match result {
        Ok(bytes) => store(ctx, call, &bytes, aspect, cancel),
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

fn post(
    endpoint: &str,
    token: &str,
    headers: &[(&str, String)],
    model: &str,
    description: &str,
    references: &[String],
    size: &str,
) -> (u16, Result<Vec<u8>, &'static str>) {
    #[cfg(test)]
    {
        tests::note_post(token);
        if tests::block_post() {
            return (0, Err(FAILED));
        }
    }
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(15))
        .timeout_read(GENERATION_TIMEOUT)
        .timeout_write(Duration::from_secs(15))
        .timeout(GENERATION_TIMEOUT)
        .redirects(0)
        .build();
    let request = headers
        .iter()
        .fold(agent.post(endpoint), |request, (name, value)| {
            request.set(name, value)
        });
    let response = request
        .set("Authorization", &format!("Bearer {token}"))
        .set("originator", ORIGINATOR)
        .set("User-Agent", "ezerbot/0.1.0")
        .set("Accept", "application/json, text/event-stream")
        .send_json(json!({
            "model": model,
            "input": [{"role": "user", "content": content(description, references)}],
            "store": false,
            "stream": true,
            "instructions": "Tu es un assistant utile.",
            "tools": [{
                "type": "image_generation",
                "model": MODEL,
                "size": size
            }],
            "tool_choice": {"type": "image_generation"}
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

/// Plain text without references; text then one `input_image` per reference.
fn content(description: &str, references: &[String]) -> Value {
    if references.is_empty() {
        return json!(description);
    }
    let mut parts = vec![json!({"type": "input_text", "text": description})];
    parts.extend(
        references
            .iter()
            .map(|url| json!({"type": "input_image", "image_url": url})),
    );
    Value::Array(parts)
}

/// Data URLs of the images named by `reference_image_paths`. An entry is the id
/// of an image attached to this thread, or the path of its staged content.
fn references(ctx: &Context<'_>, requested: &Value) -> Result<Vec<String>, &'static str> {
    let entries = match requested {
        Value::Null => return Ok(Vec::new()),
        Value::Array(entries) => entries,
        _ => return Err(BAD_REFERENCE),
    };
    if entries.len() > MAX_REFERENCES {
        return Err(TOO_MANY_REFERENCES);
    }
    let Some(conversation) = ctx.conversation_id else {
        return if entries.is_empty() {
            Ok(Vec::new())
        } else {
            Err(BAD_REFERENCE)
        };
    };
    let mut urls = Vec::new();
    for entry in entries {
        let entry = entry.as_str().ok_or(BAD_REFERENCE)?;
        let attachment = staging::get(ctx.connection, entry)
            .ok()
            .flatten()
            .or_else(|| staged_at_path(ctx, entry))
            .ok_or(BAD_REFERENCE)?;
        if !super::attachments::thread_owns_image(ctx.connection, conversation, &attachment.id)
            .unwrap_or(false)
        {
            return Err(BAD_REFERENCE);
        }
        let bytes = staging::read(ctx.connection, ctx.db_path, &attachment.id)
            .map_err(|_| BAD_REFERENCE)?;
        urls.push(format!(
            "data:{};base64,{}",
            attachment.image_type.mime_type(),
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
        ));
    }
    Ok(urls)
}

fn staged_at_path(ctx: &Context<'_>, path: &str) -> Option<staging::Attachment> {
    let sha = Path::new(path).file_name()?.to_str()?;
    let id: String = ctx
        .connection
        .query_row(
            "SELECT id FROM attachments_staged WHERE sha256=?1",
            params![sha],
            |row| row.get(0),
        )
        .optional()
        .ok()??;
    let attachment = staging::get(ctx.connection, &id).ok()??;
    (staging::content_path(ctx.db_path, &attachment).ok()? == Path::new(path)).then_some(attachment)
}

fn interpret(status: u16, body: &[u8]) -> Result<Vec<u8>, &'static str> {
    let value: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    if let Some(code) = sse_error_code(body) {
        return Err(phrase(status, &code));
    }
    if !(200..300).contains(&status) || value.get("error").is_some_and(|error| !error.is_null()) {
        let code = error_parts(&value).0;
        return Err(phrase(status, &code));
    }
    let Some(encoded) = encoded_image(&value).or_else(|| sse_image(body)) else {
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

fn encoded_image(value: &Value) -> Option<String> {
    image_result(value)
        .or_else(|| image_result(&value["item"]))
        .or_else(|| image_result(&value["response"]))
        .or_else(|| output_result(value))
        .or_else(|| output_result(&value["response"]))
        .map(str::to_owned)
}

fn output_result(value: &Value) -> Option<&str> {
    value["output"]
        .as_array()?
        .iter()
        .rev()
        .find_map(image_result)
}

fn image_result(value: &Value) -> Option<&str> {
    (value["type"] == "image_generation_call")
        .then(|| value["result"].as_str())
        .flatten()
        .filter(|result| !result.trim().is_empty())
}

fn sse_image(body: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(body).ok()?;
    let mut found = None;
    for line in text.lines() {
        let Some(data) = line.trim().strip_prefix("data:") else {
            continue;
        };
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            continue;
        }
        let Ok(value) = serde_json::from_str::<Value>(data) else {
            continue;
        };
        if let Some(encoded) = encoded_image(&value) {
            found = Some(encoded);
        }
    }
    found
}

fn sse_error_code(body: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(body).ok()?;
    for line in text.lines() {
        let Some(data) = line.trim().strip_prefix("data:") else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(data.trim()) else {
            continue;
        };
        let code = error_parts(&value).0;
        if !code.is_empty() {
            return Some(code);
        }
        if value["type"] == "response.failed" || value["type"] == "error" {
            let nested = error_parts(&value["response"]);
            return Some(nested.0);
        }
    }
    None
}

fn phrase(status: u16, code: &str) -> &'static str {
    if code == "content_safety_blocked" {
        return SAFETY;
    }
    if (400..500).contains(&status) {
        return RESTRICTED;
    }
    FAILED
}

fn store(
    ctx: &Context<'_>,
    call: &ToolCall,
    bytes: &[u8],
    aspect: &str,
    cancel: &AtomicBool,
) -> ToolOutcome {
    let Some(conversation) = ctx.conversation_id else {
        return failed(FAILED);
    };
    let Ok(message_id) = tool_message_id(ctx, &call.call_id) else {
        return failed(FAILED);
    };
    let Ok(tx) = super::write_transaction(ctx.connection) else {
        return failed(FAILED);
    };
    // Serialize with run cancellation before creating any attachment or file.
    let running = ctx
        .run_id
        .and_then(|id| grokbot_engine::store::required(&tx, id).ok())
        .is_some_and(|run| run.status == grokbot_engine::model::RunStatus::Running);
    if cancel.load(Ordering::SeqCst) || !running {
        return failed(FAILED);
    }
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

    const VAULT_KEY: &str = "vault-bot-secret-9f3c1a7e";
    const CHAT_TOKEN: &str = "chatgpt-auth-bearer-not-image";

    fn codex_token() -> String {
        use base64::Engine;
        format!(
            "e30.{}.e30",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                json!({"https://api.openai.com/auth":{"chatgpt_account_id":"ezerbot-only"}})
                    .to_string()
            )
        )
    }

    struct Outside {
        root: PathBuf,
        db: PathBuf,
        provider: Provider,
        connection: Connection,
    }

    impl Outside {
        fn open(with_session: bool) -> Self {
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
            if with_session {
                std::fs::write(
                    db.with_extension("codex-auth.json"),
                    json!({
                        "host_id": "test-host",
                        "client_id": "app_EMoamEEZ73f0CkXaXp7hrann",
                        "account_id": "ezerbot-only",
                        "access_token": codex_token(),
                        "refresh_token": "codex-refresh-token",
                        "expires_at": expires
                    })
                    .to_string(),
                )
                .unwrap();
            }
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
            grokbot_forms::vault::Vault::open(&db.with_extension("secrets.json"))
                .unwrap()
                .set("BOT_PHARE", "Secret d’un bot", VAULT_KEY)
                .unwrap();
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
    fn session_bearer_is_sent_and_a_vault_secret_stays_put() {
        reset_posts();
        let present = Outside::open(true);
        let before = std::fs::read(present.db.with_extension("secrets.json")).unwrap();
        execute(&present.ctx(), &draw(), &AtomicBool::new(false));
        assert_eq!(post_count(), 1, "le jeton de session n’a pas été utilisé");
        assert_eq!(post_key(), codex_token());
        assert_ne!(post_key(), VAULT_KEY);
        assert_ne!(post_key(), CHAT_TOKEN);
        assert_eq!(
            std::fs::read(present.db.with_extension("secrets.json")).unwrap(),
            before
        );
        let stored = secrets_text(&present.db);
        assert!(!stored.contains(CHAT_TOKEN));
        assert!(!stored.contains(&codex_token()));

        reset_posts();
        let absent = Outside::open(false);
        let before = std::fs::read(absent.db.with_extension("secrets.json")).unwrap();
        let outcome = execute(&absent.ctx(), &draw(), &AtomicBool::new(false));
        assert_eq!(post_count(), 0, "un appel part sans session");
        match outcome {
            ToolOutcome::Failed { reason } => assert_eq!(reason, CONNECT),
            other => panic!("résultat inattendu: {other:?}"),
        }
        assert_eq!(
            std::fs::read(absent.db.with_extension("secrets.json")).unwrap(),
            before
        );
        let stored = secrets_text(&absent.db);
        assert!(stored.contains(VAULT_KEY));
        assert!(!stored.contains(CHAT_TOKEN));
        assert!(!stored.contains(&codex_token()));
    }

    #[test]
    fn endpoint_accepts_only_loopback_and_the_production_image_host() {
        assert_eq!(
            image_endpoint("http://127.0.0.1:1455/v1").unwrap(),
            "http://127.0.0.1:1455/backend-api/codex/responses"
        );
        assert_eq!(
            image_endpoint("http://127.0.0.1:1455/backend-api/codex").unwrap(),
            "http://127.0.0.1:1455/backend-api/codex/responses"
        );
        assert_eq!(
            image_endpoint("https://chatgpt.com/backend-api/codex").unwrap(),
            IMAGE_HOST
        );
        for rejected in [
            "http://user:secret@127.0.0.1:1455/v1",
            "https://127.0.0.1:1455/v1",
            "http://chatgpt.com/backend-api/codex",
            "https://api.openai.com/v1",
            "https://api.openai.com/v1/images/generations",
            "https://chatgpt.com/backend-api/codex/responses",
            "https://evil.example/v1",
            "http://127.0.0.1:1455/v1?x=1",
            "http://127.0.0.1/v1",
        ] {
            assert!(image_endpoint(rejected).is_err(), "{rejected}");
        }
    }

    #[test]
    fn known_ratios_become_accepted_sizes() {
        assert_eq!(image_size("1:1"), "1024x1024");
        assert_eq!(image_size(""), "1024x1024");
        assert_eq!(image_size("16:9"), "1536x1024");
        assert_eq!(image_size("4:3"), "1536x1024");
        assert_eq!(image_size("9:16"), "1024x1536");
        assert_eq!(image_size("3:4"), "1024x1536");
    }

    #[test]
    fn the_closed_frame_list_has_a_size_for_each_frame() {
        for frame in FRAMES {
            assert!(
                ["1024x1024", "1536x1024", "1024x1536"].contains(&image_size(frame)),
                "{frame}"
            );
        }
        assert!(!FRAMES.contains(&"2:1"));
    }

    #[test]
    fn references_are_text_then_images_and_plain_text_without_them() {
        assert_eq!(content("un phare", &[]), json!("un phare"));
        assert_eq!(
            content("un phare", &["data:image/png;base64,AA==".into()]),
            json!([
                {"type": "input_text", "text": "un phare"},
                {"type": "input_image", "image_url": "data:image/png;base64,AA=="}
            ])
        );
    }

    #[test]
    fn provider_errors_become_french_and_a_url_is_not_fetched() {
        let safety = br#"{"error":{"code":"content_safety_blocked","message":"blocked"}}"#;
        assert_eq!(interpret(400, safety).unwrap_err(), SAFETY);
        let restricted = br#"{"error":{"code":"model_restricted","message":"restricted"}}"#;
        assert_eq!(interpret(400, restricted).unwrap_err(), RESTRICTED);
        let refused = br#"{"error":{"code":"insufficient_quota","message":"nope"}}"#;
        assert_eq!(interpret(403, refused).unwrap_err(), RESTRICTED);
        let down = br#"{"error":{"code":"server_error","message":"down"}}"#;
        assert_eq!(interpret(503, down).unwrap_err(), FAILED);
        assert_eq!(interpret(0, b"").unwrap_err(), FAILED);
        let remote = br#"{"output":[{"type":"image_generation_call","result":""}]}"#;
        assert_eq!(interpret(200, remote).unwrap_err(), UNUSABLE);
        let url_only = br#"{"data":[{"url":"https://evil.example/secret.png"}]}"#;
        assert_eq!(interpret(200, url_only).unwrap_err(), UNUSABLE);
        let garbage =
            br#"{"output":[{"type":"image_generation_call","result":"bm90IGFuIGltYWdl"}]}"#;
        assert_eq!(interpret(200, garbage).unwrap_err(), UNUSABLE);
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a6XcAAAAASUVORK5CYII=";
        let json = format!(
            r#"{{"output":[{{"type":"image_generation_call","status":"completed","result":"{png}"}}]}}"#
        );
        assert!(interpret(200, json.as_bytes()).is_ok());
        let sse = format!(
            "event: response.output_item.done\ndata: {{\"type\":\"response.output_item.done\",\"item\":{{\"type\":\"image_generation_call\",\"result\":\"{png}\"}}}}\n\n"
        );
        assert!(interpret(200, sse.as_bytes()).is_ok());
    }
}
