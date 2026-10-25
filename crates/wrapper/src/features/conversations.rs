//! Existing conversation, bot and preference routes, plus the ordered timeline.
//! Submission bodies remain authoritative in the journal. Missing metadata after
//! a crash stays visible; capability entries use the same ordinal space.
use crate::*;

empty_hooks!(
    migrate,
    state,
    tick,
    tools,
    call_tool,
    context,
    entry,
    on_run_event,
    link_attachments
);

pub fn routes(
    app: &mut App,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Result<ApiResponse, ApiError>> {
    if !matches!(
        (method, path),
        ("GET", "/api/state" | "/api/ping")
            | (
                "POST",
                "/api/preferences"
                    | "/api/conversations"
                    | "/api/conversations/update"
                    | "/api/conversations/delete"
                    | "/api/bots"
                    | "/api/bots/update"
                    | "/api/bots/delete"
            )
    ) {
        return None;
    }
    Some(route(app, path, body).map(ApiResponse::Json))
}

// Detect writes from both this connection and background engine connections.
// A process identity also invalidates the client's token after a wrapper restart.
fn state_revision(
    connection: &rusqlite::Connection,
    path: &std::path::Path,
) -> Result<String, ApiError> {
    let external: u64 = connection.pragma_query_value(None, "data_version", |row| row.get(0))?;
    Ok(format!(
        "{}.{}.{}.{}.{}",
        std::process::id(),
        external,
        connection.total_changes(),
        super::screen::revision(path),
        super::updates::revision(),
    ))
}

fn route(app: &mut App, path: &str, body: &[u8]) -> Result<Value, ApiError> {
    match path {
        "/api/ping" => Ok(json!({"revision": state_revision(&app.connection, &app.db_path)?})),
        "/api/state" => {
            // Capture before reading: a concurrent write during serialization
            // must cause another refresh, never be mistaken for observed data.
            let revision = state_revision(&app.connection, &app.db_path)?;
            let (theme, accent) = app.preferences()?;
            let provider = app.provider.status();
            let resolved = app.resolved_model()?;
            let fast = app.model_preferences()?.2;
            let mut state = json!({
                "account": {"id": ACCOUNT, "name": "Espace local"},
                "mode": if provider["connected"] == true { "chatgpt" } else { "simulated" },
                "conversations": app.conversations()?, "bots": app.bots()?,
                "preferences": { "theme": theme, "accent": accent, "fast": fast,
                    "model": resolved.as_ref().map(|(model, _, _)| model),
                    "effort": resolved.as_ref().and_then(|(_, effort, _)| effort.as_ref()), },
                "provider": provider, "state_revision": revision,
            });
            // Each capability is merged under its own fixed key.
            for (key, value) in super::state(&app.feature_context())
                .as_object()
                .expect("feature state object")
            {
                state
                    .as_object_mut()
                    .expect("state object")
                    .entry(key.clone())
                    .or_insert_with(|| value.clone());
            }
            let directory = app.db_path.parent().unwrap_or(std::path::Path::new("."));
            let client = app.state_client;
            app.stream_watches.entry(client).or_default().observe(
                directory,
                client,
                &state,
                std::time::Instant::now(),
            );
            Ok(state)
        }
        "/api/preferences" => {
            let input: Preferences = parse(body)?;
            if input.model.is_some() || input.effort.is_some() || input.fast.is_some() {
                for value in [&input.model, &input.effort].into_iter().flatten() {
                    validate_id(value)?;
                }
                let (theme, accent) = app.preferences()?;
                app.connection.execute("INSERT INTO wrapper_preferences (account_id, theme, accent) VALUES (?1, ?2, ?3) ON CONFLICT(account_id) DO NOTHING", params![ACCOUNT, theme, accent])?;
                app.connection.execute("UPDATE wrapper_preferences SET model = COALESCE(?2, model), effort = COALESCE(?3, effort), fast = COALESCE(?4, fast) WHERE account_id = ?1", params![ACCOUNT, input.model, input.effort, input.fast])?;
            }
            if input.theme.is_none() && input.accent.is_none() {
                if input.model.is_some() || input.effort.is_some() || input.fast.is_some() {
                    let resolved = app.resolved_model()?;
                    return Ok(json!({"preferences": {
                        "fast": app.model_preferences()?.2,
                        "model": resolved.as_ref().map(|(model, _, _)| model),
                        "effort": resolved.as_ref().and_then(|(_, effort, _)| effort.as_ref()),
                    }}));
                }
                return Err(ApiError::new(400, "Indiquez un thème ou un accent."));
            }
            let (stored_theme, stored_accent) = app.preferences()?;
            let theme = input.theme.unwrap_or(stored_theme);
            let accent = input.accent.unwrap_or(stored_accent);
            if !matches!(theme.as_str(), "light" | "dark" | "system") {
                return Err(ApiError::new(
                    400,
                    "Le thème doit être light, dark ou system.",
                ));
            }
            if !ACCENTS.contains(&accent.as_str()) {
                return Err(ApiError::new(400, "Cet accent n’existe pas."));
            }
            app.connection.execute("INSERT INTO wrapper_preferences (account_id, theme, accent) VALUES (?1, ?2, ?3) ON CONFLICT(account_id) DO UPDATE SET theme = excluded.theme, accent = excluded.accent", params![ACCOUNT, theme, accent])?;
            Ok(json!({"preferences": {"theme": theme, "accent": accent}}))
        }
        "/api/conversations" => {
            let input: CreateConversation = parse(body)?;
            let title = valid_title(input.title.as_deref().unwrap_or(DEFAULT_TITLE))?;
            let mut bots = input
                .bot_ids
                .unwrap_or_else(|| vec![input.bot_id.unwrap_or_else(|| DEFAULT_BOT.id.into())]);
            let mut seen = HashSet::new();
            bots.retain(|id| seen.insert(id.clone()));
            if bots.is_empty() || bots.len() > 6 {
                return Err(ApiError::new(400, "Choisissez de un à six bots."));
            }
            if bots.len() < 2 && (input.description.is_some() || input.avatar.is_some()) {
                return Err(ApiError::new(
                    400,
                    "La description et l’avatar concernent un groupe.",
                ));
            }
            let description = input
                .description
                .as_deref()
                .map(valid_group_description)
                .transpose()?
                .unwrap_or_default();
            let avatar = input
                .avatar
                .as_ref()
                .map(valid_group_avatar)
                .transpose()?
                .map(|value| serde_json::to_string(&value))
                .transpose()
                .map_err(|_| {
                    ApiError::new(500, "L’avatar du groupe n’a pas pu être enregistré.")
                })?;
            for id in &bots {
                validate_id(id)?;
                app.require_bot(id)?;
            }
            let bot_id = &bots[0];
            let id: String =
                app.connection
                    .query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))?;
            let timestamp = now();
            let tx = super::write_transaction(&app.connection)?;
            tx.execute("INSERT INTO wrapper_conversations (account_id, id, title, title_customized, created_at, updated_at, bot_id, group_description, group_avatar) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6, ?7, ?8)", params![ACCOUNT, id, title, input.title.is_some(), timestamp, bot_id, description, avatar])?;
            super::multibot::initialize(&tx, &id, &bots)?;
            if let Some(topic) = input.topic_id.as_deref() {
                // Le fil naît déjà classé : un sujet introuvable annule la création.
                super::topics::assign(&tx, &id, Some(topic))?;
            }
            tx.commit()?;
            app.conversation_response(&id)
        }
        "/api/conversations/update" => {
            let input: UpdateConversation = parse(body)?;
            validate_conversation_id(&input.id)?;
            app.require_conversation(&input.id)?;
            let title = input.title.as_deref().map(valid_title).transpose()?;
            let kind: String = app.connection.query_row(
                "SELECT kind FROM wrapper_conversations WHERE account_id = ?1 AND id = ?2",
                params![ACCOUNT, input.id],
                |row| row.get(0),
            )?;
            if kind != "group" && (input.description.is_some() || input.avatar.is_some()) {
                return Err(ApiError::new(
                    400,
                    "La description et l’avatar concernent un groupe.",
                ));
            }
            let description = input
                .description
                .as_deref()
                .map(valid_group_description)
                .transpose()?;
            let avatar = input
                .avatar
                .as_ref()
                .map(valid_group_avatar)
                .transpose()?
                .map(|value| serde_json::to_string(&value))
                .transpose()
                .map_err(|_| {
                    ApiError::new(500, "L’avatar du groupe n’a pas pu être enregistré.")
                })?;
            if title.is_none()
                && input.favorite.is_none()
                && description.is_none()
                && avatar.is_none()
            {
                return Err(ApiError::new(
                    400,
                    "Choisis un titre ou un favori à modifier.",
                ));
            }
            app.connection.execute("UPDATE wrapper_conversations SET title = COALESCE(?3, title), title_customized = CASE WHEN ?3 IS NOT NULL THEN 1 ELSE title_customized END, favorite = COALESCE(?4, favorite), group_description = COALESCE(?6, group_description), group_avatar = COALESCE(?7, group_avatar), updated_at = ?5 WHERE account_id = ?1 AND id = ?2", params![ACCOUNT, input.id, title, input.favorite, now(), description, avatar])?;
            app.conversation_response(&input.id)
        }
        "/api/conversations/delete" => {
            let input: DeleteConversation = parse(body)?;
            validate_conversation_id(&input.id)?;
            let tx = super::write_transaction(&app.connection)?;
            let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_conversations WHERE account_id = ?1 AND id = ?2)", params![ACCOUNT, input.id], |r| r.get(0))?;
            if !exists {
                return Err(ApiError::new(404, "Conversation introuvable."));
            }
            let capturing: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_meetings WHERE conversation_id=?1 AND status='recording')", [&input.id], |r| r.get(0))?;
            if capturing {
                return Err(ApiError::new(409, "Arrêtez ou supprimez la réunion en cours avant de supprimer cette conversation."));
            }
            tx.execute("UPDATE wrapper_conversations SET deleted = 1, updated_at = ?3 WHERE account_id = ?1 AND id = ?2", params![ACCOUNT, input.id, now()])?;
            super::storage::discard_conversation_draft(&tx, &input.id)?;
            super::pages::anchors::discard_conversation(&tx, &input.id)?;
            super::voice_session::discard_conversation(&tx, &input.id)?;
            tx.execute("UPDATE wrapper_meetings SET audio=NULL,expires_at=NULL,status=CASE WHEN status='recording' THEN 'interrupted' ELSE status END WHERE conversation_id=?1", [&input.id])?;
            tx.commit()?;
            Ok(json!({"ok": true}))
        }
        "/api/bots" => {
            let input: CreateBot = parse(body)?;
            validate_id(&input.id)?;
            let fields = BotChanges {
                name: Some(input.name),
                color: Some(input.color),
                shape: Some(input.shape.unwrap_or_else(|| DEFAULT_SHAPE.into())),
                description: Some(input.description),
                instructions: Some(input.instructions),
            }
            .validated()?;
            app.create_bot(&input.id, &fields)
        }
        "/api/bots/update" => {
            let input: UpdateBot = parse(body)?;
            validate_id(&input.id)?;
            let changes = BotChanges {
                name: input.name,
                color: input.color,
                shape: input.shape,
                description: input.description,
                instructions: input.instructions,
            }
            .validated()?;
            app.update_bot(&input.id, &changes)
        }
        "/api/bots/delete" => {
            let input: DeleteBot = parse(body)?;
            validate_id(&input.id)?;
            if input.id == DEFAULT_BOT.id {
                return Err(ApiError::new(
                    409,
                    "Le bot par défaut ne peut pas être supprimé.",
                ));
            }
            if app.bot(&input.id)?.is_none() {
                return Err(ApiError::new(404, "Bot introuvable."));
            }
            super::multibot::delete_bot(app, &input.id)?;
            Ok(json!({"ok": true}))
        }
        _ => unreachable!("route filtered above"),
    }
}

/// Append a timeline identity on the caller's connection/transaction. Submissions
/// and capability entries share this namespace: only the same kind is idempotent.
/// Capability payloads remain with their owner; `entry` projects them on reads.
pub fn append_entry(
    conn: &Connection,
    conversation: &str,
    kind: &str,
    id: &str,
    created_at: u64,
) -> Result<(), ApiError> {
    conn.execute("INSERT INTO wrapper_messages (account_id, conversation_id, message_id, kind, created_at) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(account_id, conversation_id, message_id) DO NOTHING", params![ACCOUNT, conversation, id, kind, created_at])?;
    let existing: String = conn.query_row("SELECT kind FROM wrapper_messages WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3", params![ACCOUNT, conversation, id], |row| row.get(0))?;
    if existing != kind {
        return Err(ApiError::new(
            409,
            "Cet identifiant appartient déjà à un autre type d'entrée.",
        ));
    }
    Ok(())
}

pub fn has_submission(
    conn: &Connection,
    conversation: &str,
    message: &str,
) -> Result<bool, ApiError> {
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM wrapper_messages WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3 AND kind = 'submission')",
        params![ACCOUNT, conversation, message],
        |row| row.get(0),
    )?)
}

/// Projections attempted before a state read gives up on a vault that keeps changing.
const VAULT_SETTLE_ATTEMPTS: usize = 4;

impl App {
    pub(crate) fn conversations(&self) -> Result<Vec<Conversation>, ApiError> {
        self.conversations_settled(&mut || {})
    }

    /// Every output of the projection is masked with one content of the vault (#377): a
    /// `redact::hold` makes the first vault read the one used by every masking that follows on
    /// this thread, whatever the entry (a direct reply, a routine or group reply, a meeting card,
    /// a delegation or form card). A secret stored meanwhile (a tool renewing an OAuth token)
    /// would escape that read: once the projection is done the vault is read again, and the
    /// projection starts over if it changed. What is returned was masked with the vault as it
    /// stands after the last output, never with an older one.
    /// `after_entry` runs after each projected reply or entry: a no-op, except in the tests
    /// that write to the vault at that point.
    pub(crate) fn conversations_settled(
        &self,
        after_entry: &mut dyn FnMut(),
    ) -> Result<Vec<Conversation>, ApiError> {
        for _ in 0..VAULT_SETTLE_ATTEMPTS {
            let held = grokbot_forms::redact::hold();
            let result = self.project_conversations(after_entry)?;
            if held.settled()? {
                return Ok(result);
            }
        }
        Err(ApiError::new(
            503,
            "Le coffre des secrets change sans arrêt : la lecture des conversations est refusée, réessayez.",
        ))
    }

    fn project_conversations(
        &self,
        after_entry: &mut dyn FnMut(),
    ) -> Result<Vec<Conversation>, ApiError> {
        let mut conversations = BTreeMap::new();
        let mut hidden = HashSet::new();
        let mut created = BTreeMap::new();
        let mut statement = self.connection.prepare("SELECT id, title, favorite, deleted, created_at, updated_at, bot_id, kind, parent_id, group_description, group_avatar FROM wrapper_conversations WHERE account_id = ?1")?;
        let rows = statement.query_map([ACCOUNT], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, bool>(2)?,
                row.get::<_, bool>(3)?,
                row.get::<_, u64>(4)?,
                row.get::<_, u64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, Option<String>>(10)?,
            ))
        })?;
        for row in rows {
            let (
                id,
                title,
                favorite,
                deleted,
                created_at,
                updated_at,
                bot_id,
                kind,
                parent_id,
                description,
                avatar,
            ) = row?;
            if deleted {
                hidden.insert(id);
                continue;
            }
            created.insert(id.clone(), created_at);
            conversations.insert(
                id.clone(),
                Conversation {
                    member_ids: super::multibot::members(&self.connection, &id)?,
                    id,
                    bot_id,
                    title,
                    favorite,
                    updated_at,
                    kind,
                    parent_id,
                    description,
                    avatar: stored_group_avatar(avatar),
                    messages: Vec::new(),
                },
            );
        }
        let mut ordered: BTreeMap<String, Vec<(i64, Vec<Message>)>> = BTreeMap::new();
        for record in self.reader.list_submissions()? {
            let id = record.conversation_id.as_str();
            if hidden.contains(id) {
                continue;
            }
            let metadata: Option<(i64, u64)> = self.connection.prepare_cached("SELECT ordinal, created_at FROM wrapper_messages WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3 AND kind = 'submission'")?.query_row(params![ACCOUNT, id, record.message_id.as_str()], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
            let (ordinal, created_at) =
                metadata.unwrap_or((i64::MAX, *created.get(id).unwrap_or(&0)));
            let conversation = conversations
                .entry(id.to_owned())
                .or_insert_with(|| Conversation {
                    id: id.to_owned(),
                    bot_id: DEFAULT_BOT.id.into(),
                    member_ids: vec![DEFAULT_BOT.id.into()],
                    title: "Conversation récupérée".into(),
                    favorite: false,
                    updated_at: created_at,
                    kind: "chat".into(),
                    parent_id: None,
                    description: String::new(),
                    avatar: None,
                    messages: Vec::new(),
                });
            conversation.updated_at = conversation.updated_at.max(created_at);
            let mut ctx = self.feature_context();
            ctx.conversation_id = Some(id);
            ctx.bot_id = Some(&conversation.bot_id);
            let mut pair = vec![Message {
                id: record.message_id.as_str().into(),
                role: "user",
                body: record.body,
                state: state_name(&record.state),
                created_at,
                error: None,
                error_kind: None,
                notice: None,
                kind: "message".into(),
                payload: json!({"mentions":super::multibot::mentions(&self.connection, id, record.message_id.as_str()), "demonstration_id":super::demonstrations::message_source(&ctx, record.message_id.as_str())}),
                attachments: super::attachments::for_message(&ctx, id, record.message_id.as_str()),
            }];
            if matches!(
                record.state,
                SubmissionState::AcceptedAwaitingEcho { .. } | SubmissionState::Echoed { .. }
            ) {
                if let Some(reply) = super::engine::reply(&ctx, id, record.message_id.as_str())? {
                    pair.push(reply);
                    after_entry();
                }
            }
            // Meeting summaries are projected inside their durable meeting card,
            // not as a second visible copy of the internal summary request.
            if !super::meetings::is_summary(&self.connection, id, record.message_id.as_str())? {
                ordered.entry(id.into()).or_default().push((ordinal, pair));
            }
        }
        let mut statement = self.connection.prepare("SELECT ordinal, conversation_id, kind, message_id, created_at FROM wrapper_messages WHERE account_id = ?1 AND kind <> 'submission' ORDER BY ordinal")?;
        let rows = statement.query_map([ACCOUNT], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, u64>(4)?,
            ))
        })?;
        for row in rows {
            let (ordinal, conversation_id, kind, id, created_at) = row?;
            let meeting_reply: bool = kind == "group_reply" && self.connection.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_group_turns t JOIN wrapper_meetings m ON m.conversation_id=t.conversation_id AND m.message_id=t.root_id WHERE t.id=?1 AND t.parent_id IS NULL)",[&id],|r|r.get(0))?;
            if hidden.contains(&conversation_id)
                || meeting_reply
                || (kind == "group_notice"
                    && !super::multibot::notice_visible(&self.connection, &id)?)
            {
                continue;
            }
            let Some(conversation) = conversations.get_mut(&conversation_id) else {
                continue;
            };
            let mut ctx = self.feature_context();
            ctx.conversation_id = Some(&conversation_id);
            ctx.bot_id = Some(&conversation.bot_id);
            // Unknown entries must survive projection for the UI fallback card.
            let message = super::entry(&ctx, &kind, &id).unwrap_or_else(|| Message {
                id,
                role: "assistant",
                body: String::new(),
                state: "echoed",
                created_at,
                error: None,
                error_kind: None,
                notice: None,
                kind,
                payload: Value::Null,
                attachments: Vec::new(),
            });
            after_entry();
            conversation.updated_at = conversation.updated_at.max(created_at);
            ordered
                .entry(conversation_id)
                .or_default()
                .push((ordinal, vec![message]));
        }
        for (id, mut messages) in ordered {
            messages.sort_by_key(|(ordinal, _)| *ordinal);
            if let Some(conversation) = conversations.get_mut(&id) {
                conversation.messages = messages.into_iter().flat_map(|(_, pair)| pair).collect();
            }
        }
        let mut result: Vec<_> = conversations.into_values().collect();
        result.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        let reactions = self.reactions()?;
        for conversation in &mut result {
            for message in &mut conversation.messages {
                if let Some(emoji) = reactions.get(&(conversation.id.clone(), message.id.clone())) {
                    stamp_reaction(message, emoji);
                }
            }
        }
        Ok(result)
    }

    fn reactions(&self) -> Result<BTreeMap<(String, String), String>, ApiError> {
        let mut statement = self.connection.prepare(
            "SELECT conversation_id, message_id, emoji FROM wrapper_reactions WHERE account_id = ?1",
        )?;
        let rows =
            statement.query_map([ACCOUNT], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?;
        let mut reactions = BTreeMap::new();
        for row in rows {
            let (conversation, message, emoji) = row?;
            reactions.insert((conversation, message), emoji);
        }
        Ok(reactions)
    }
}

fn stamp_reaction(message: &mut Message, emoji: &str) {
    if message.payload.is_null() {
        message.payload = json!({});
    }
    if let Some(object) = message.payload.as_object_mut() {
        object.insert("reaction".into(), json!(emoji));
    }
}
