//! Local reconstruction: REA CM13/CM15/CM19 leaves generation, files and history
//! server-side/unknown. Reuse the host approval gate, but require a human click
//! for EVERY revision (no standing rules or model reviewer). Companion paths are
//! SQLite labels only; no filesystem writes or executable attachments.
use super::*;
use std::collections::HashSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    name: String,
    description: String,
    body: String,
    skill_id: Option<String>,
    expected_version: Option<u64>,
    #[serde(default)]
    files: Vec<SkillFile>,
}

fn validated(
    ctx: &Context<'_>,
    call: &ToolCall,
) -> Result<(Proposal, SkillContent, Option<library::Skill>), ApiError> {
    let bot = ctx
        .bot_id
        .ok_or_else(|| ApiError::new(400, "Bot de la skill indisponible."))?;
    let input: Proposal = serde_json::from_value(call.arguments.clone())
        .map_err(|_| ApiError::new(400, "Proposition de skill invalide."))?;
    // Production model events have already been redacted before this gate.
    // Refuse holes as well as raw known values; neither belongs in a skill.
    if call.arguments.to_string().contains("[...]")
        || grokbot_forms::redact::redact_value(&super::super::forms::vault(ctx)?, &call.arguments)?
            != call.arguments
    {
        return Err(ApiError::new(400, "Une proposition de skill ne doit contenir aucun secret enregistré ni texte masqué ([...])."));
    }
    let before = match (&input.skill_id, input.expected_version) {
        (None, None) => {
            let key = library::normalize_name(&input.name);
            let duplicate = {
                let mut statement = ctx.connection.prepare(
                    "SELECT name, display_name FROM skills_library WHERE bot_id=?1 AND deleted=0",
                )?;
                let rows = statement.query_map([bot], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                let mut duplicate = false;
                for row in rows {
                    let (name, title) = row?;
                    if name == key || library::normalize_name(&title) == key {
                        duplicate = true;
                        break;
                    }
                }
                duplicate
            };
            if duplicate {
                return Err(ApiError::new(409, "Une skill porte déjà ce nom. Consultez-la et proposez une mise à jour avec skill_id et expected_version, ou choisissez un autre nom."));
            }
            let count: usize = ctx.connection.query_row(
                "SELECT count(*) FROM skills_library WHERE bot_id=?1 AND deleted=0",
                [bot],
                |row| row.get(0),
            )?;
            if count >= import::MAX_SKILLS_PER_BOT {
                return Err(ApiError::new(409, "Limite de 250 skills atteinte."));
            }
            None
        }
        (Some(id), Some(version)) if version > 0 => {
            owned_skill(ctx.connection, bot, id)?;
            let current: u64 = ctx
                .connection
                .query_row(
                    "SELECT version FROM skills_library WHERE skill_id=?1 AND deleted=0",
                    [id],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or_else(|| ApiError::new(409, "Cette skill a été supprimée."))?;
            if current != version {
                return Err(ApiError::new(409, "La skill a changé. Demandez une nouvelle proposition à partir de sa version courante."));
            }
            library::get_version(ctx.connection, id, version).map_err(skill_error)?
        }
        _ => {
            return Err(ApiError::new(
                400,
                "Une mise à jour exige skill_id et expected_version ensemble.",
            ))
        }
    };
    let body = if before
        .as_ref()
        .is_some_and(|skill| matches!(skill.content.origin, Origin::Upload { .. }))
    {
        import::editable_body(&input.body)
    } else {
        input.body.clone()
    };
    let mut content =
        manual_content(input.name.clone(), input.description.clone(), body).map_err(|error| {
            ApiError::new(
                error.status,
                match error.message.as_str() {
                    "name-too-large" => "Le nom ne doit pas dépasser 128 caractères.",
                    "description-too-large" => {
                        "La description ne doit pas dépasser 1 024 caractères."
                    }
                    "too-large" => "Le contenu dépasse 256 000 caractères.",
                    _ => "Le nom doit contenir une lettre latine ou un chiffre.",
                },
            )
        })?;
    if content.body.trim().is_empty() {
        return Err(ApiError::new(400, "La skill doit avoir un contenu."));
    }
    let mut paths = HashSet::new();
    if input.files.len() > 20 {
        return Err(ApiError::new(
            400,
            "Une skill accepte au plus 20 fichiers texte annexes.",
        ));
    }
    for file in &input.files {
        if !import::valid_companion_path(&file.path) || !paths.insert(file.path.to_lowercase()) {
            return Err(ApiError::new(400, "Nom de fichier annexe invalide ou répété. Utilisez un chemin relatif de fichier texte."));
        }
        if file
            .content
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        {
            return Err(ApiError::new(
                400,
                "Les fichiers annexes doivent contenir du texte UTF-8 sans données binaires.",
            ));
        }
    }
    content.origin = Origin::Model;
    content.files = input.files.clone();
    validate_companion_budget(&content)?;
    Ok((input, content, before))
}

pub(super) fn display(ctx: &Context<'_>, call: &ToolCall) -> Result<Value, ApiError> {
    let (input, content, before) = validated(ctx, call)?;
    let before = before.map(|mut skill| {
        skill.content.body = editable_body(&skill.content);
        skill
    });
    Ok(json!({"proposal":content, "before":before, "expected_version":input.expected_version}))
}

pub(super) fn save(ctx: &Context<'_>, call: &ToolCall) -> Result<Value, ApiError> {
    // Defense against direct invoke_tool/feature calls: only the engine's exact
    // consumed, argument-bound one-shot grant permits this effect.
    let id = super::super::forms::consumed_tool_approval(ctx, call)?;
    let saved: Option<String> = ctx.connection.query_row(
        "SELECT json_extract(display_json,'$.saved') FROM wrapper_forms_calls WHERE request_id=?1",
        [&id],
        |row| row.get(0),
    )?;
    if let Some(saved) = saved {
        return serde_json::from_str(&saved)
            .map_err(|_| ApiError::new(500, "Résultat d’enregistrement illisible."));
    }
    ctx.connection
        .execute_batch("SAVEPOINT skills_proposal_save")?;
    let result: Result<Value, ApiError> = (|| {
        let (input, content, _) = validated(ctx, call)?;
        let bot = ctx.bot_id.expect("validated bot");
        let skill = match (&input.skill_id, input.expected_version) {
            (Some(id), Some(version)) => library::update(ctx.connection, id, version, &content),
            _ => library::add(ctx.connection, bot, &content),
        }
        .map_err(skill_error)?;
        super::super::demonstrations::saved(ctx, &id, &skill.skill_id)?;
        let output = json!({"saved":true,"skill_id":skill.skill_id,"version":skill.version,"name":skill.content.display_name});
        ctx.connection.execute("UPDATE wrapper_forms_calls SET display_json=json_set(display_json,'$.saved',json(?2)) WHERE request_id=?1",
            params![id, output.to_string()])?;
        Ok(output)
    })();
    if result.is_err() {
        ctx.connection
            .execute_batch("ROLLBACK TO skills_proposal_save")?;
    }
    ctx.connection
        .execute_batch("RELEASE skills_proposal_save")?;
    let result = result.map_err(|error| {
        if error.status >= 500 {
            ApiError::new(500, "L’enregistrement a échoué. Réessayez avec une nouvelle proposition après avoir vérifié l’espace local.")
        } else { error }
    });
    if let Err(error) = &result {
        ctx.connection.execute("UPDATE wrapper_forms_calls SET display_json=json_set(display_json,'$.save_error',?2) WHERE request_id=?1",
            params![id, error.message])?;
    }
    result
}
