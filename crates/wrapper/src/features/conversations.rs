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
        ("GET", "/api/state")
            | (
                "POST",
                "/api/preferences"
                    | "/api/conversations"
                    | "/api/conversations/update"
                    | "/api/conversations/delete"
                    | "/api/bots"
                    | "/api/bots/update"
                    | "/api/bots/delete"
                    | "/api/proposals/accept"
                    | "/api/proposals/dismiss"
            )
    ) {
        return None;
    }
    Some(route(app, path, body).map(ApiResponse::Json))
}

fn route(app: &mut App, path: &str, body: &[u8]) -> Result<Value, ApiError> {
    match path {
        "/api/state" => {
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
                "provider": provider,
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
            tx.execute("INSERT INTO wrapper_conversations (account_id, id, title, title_customized, created_at, updated_at, bot_id) VALUES (?1, ?2, ?3, ?4, ?5, ?5, ?6)", params![ACCOUNT, id, title, input.title.is_some(), timestamp, bot_id])?;
            super::multibot::initialize(&tx, &id, &bots)?;
            tx.commit()?;
            app.conversation_response(&id)
        }
        "/api/conversations/update" => {
            let input: UpdateConversation = parse(body)?;
            validate_conversation_id(&input.id)?;
            app.require_conversation(&input.id)?;
            let title = input.title.as_deref().map(valid_title).transpose()?;
            if title.is_none() && input.favorite.is_none() {
                return Err(ApiError::new(
                    400,
                    "Choisis un titre ou un favori à modifier.",
                ));
            }
            app.connection.execute("UPDATE wrapper_conversations SET title = COALESCE(?3, title), title_customized = CASE WHEN ?3 IS NOT NULL THEN 1 ELSE title_customized END, favorite = COALESCE(?4, favorite), updated_at = ?5 WHERE account_id = ?1 AND id = ?2", params![ACCOUNT, input.id, title, input.favorite, now()])?;
            app.conversation_response(&input.id)
        }
        "/api/conversations/delete" => {
            let input: DeleteConversation = parse(body)?;
            validate_conversation_id(&input.id)?;
            let exists: bool = app.connection.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_conversations WHERE account_id = ?1 AND id = ?2)", params![ACCOUNT, input.id], |r| r.get(0))?;
            if !exists {
                return Err(ApiError::new(404, "Conversation introuvable."));
            }
            app.connection.execute("UPDATE wrapper_conversations SET deleted = 1, updated_at = ?3 WHERE account_id = ?1 AND id = ?2", params![ACCOUNT, input.id, now()])?;
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
        "/api/proposals/accept" => {
            let input: AcceptProposal = parse(body)?;
            validate_id(&input.id)?;
            app.accept_proposal(&input.id, input.overrides)
        }
        "/api/proposals/dismiss" => {
            let input: DismissProposal = parse(body)?;
            validate_id(&input.id)?;
            app.dismiss_proposal(&input.id)
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

impl App {
    pub(crate) fn conversations(&self) -> Result<Vec<Conversation>, ApiError> {
        let mut conversations = BTreeMap::new();
        let mut hidden = HashSet::new();
        let mut created = BTreeMap::new();
        let mut statement = self.connection.prepare("SELECT id, title, favorite, deleted, created_at, updated_at, bot_id, kind, parent_id FROM wrapper_conversations WHERE account_id = ?1")?;
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
            ))
        })?;
        for row in rows {
            let (id, title, favorite, deleted, created_at, updated_at, bot_id, kind, parent_id) =
                row?;
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
                    messages: Vec::new(),
                },
            );
        }
        let mut ordered: BTreeMap<String, Vec<(i64, Vec<Message>)>> = BTreeMap::new();
        let mut proposals = self.proposals_by_message()?;
        for record in self.reader.list_submissions()? {
            let id = record.conversation_id.as_str();
            if hidden.contains(id) {
                continue;
            }
            let metadata: Option<(i64, u64)> = self.connection.query_row("SELECT ordinal, created_at FROM wrapper_messages WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3 AND kind = 'submission'", params![ACCOUNT, id, record.message_id.as_str()], |r| Ok((r.get(0)?, r.get(1)?))).optional()?;
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
                proposals: Vec::new(),
                kind: "message".into(),
                payload: json!({"mentions":super::multibot::mentions(&self.connection, id, record.message_id.as_str())}),
                attachments: super::attachments::for_message(&ctx, id, record.message_id.as_str()),
            }];
            if matches!(
                record.state,
                SubmissionState::AcceptedAwaitingEcho { .. } | SubmissionState::Echoed { .. }
            ) {
                if let Some(reply) = super::engine::reply(
                    &ctx,
                    id,
                    record.message_id.as_str(),
                    proposals
                        .remove(&(id.to_owned(), record.message_id.as_str().to_owned()))
                        .unwrap_or_default(),
                )? {
                    pair.push(reply);
                }
            }
            ordered.entry(id.into()).or_default().push((ordinal, pair));
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
            if hidden.contains(&conversation_id)
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
                proposals: Vec::new(),
                kind,
                payload: Value::Null,
                attachments: Vec::new(),
            });
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
        Ok(result)
    }
}
