//! Approved external effects. Never hold SQLite's writer or the registry over I/O.
use super::*;
use crate::mcp::{Connection as Handle, Error as McpError};
use std::sync::Arc;

pub(super) fn retire(ctx: &Context<'_>, id: &str) {
    let old = ctx
        .provider
        .mcp()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(id);
    if let Some(old) = old {
        old.client.lock().unwrap_or_else(|e| e.into_inner()).stop();
    }
}

// Publish only for the same process: an old EOF must not retire a replacement.
pub(super) fn publish(
    ctx: &Context<'_>,
    id: &str,
    handle: &Arc<Handle>,
    tools: &Result<Vec<Value>, ApiError>,
) -> Result<(), ApiError> {
    let tx = write_transaction(ctx.connection)?;
    let mut registry = ctx
        .provider
        .mcp()
        .0
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if !registry
        .get(id)
        .is_some_and(|c| c.identity == handle.identity)
    {
        return Err(invalid(
            "Cette connexion a été remplacée par une autre action sur le connecteur.",
        ));
    }
    match tools {
        Ok(tools) => {
            tx.execute(
                "UPDATE wrapper_mcp_servers SET tools_json=?2,error=NULL WHERE id=?1",
                params![id, json!(tools).to_string()],
            )?;
        }
        Err(error) => {
            tx.execute(
                "UPDATE wrapper_mcp_servers SET error=?2 WHERE id=?1",
                params![id, error.message],
            )?;
        }
    }
    tx.commit()?;
    if tools.is_err() {
        registry.remove(id);
    } else {
        handle.ready.store(true, Ordering::Release);
    }
    Ok(())
}

pub(super) fn execute(ctx: &Context<'_>, call: &ToolCall) -> Result<Value, ApiError> {
    if !ctx.connection.is_autocommit() {
        return Err(invalid(
            "Un effet MCP exige une admission durable avant son exécution.",
        ));
    }
    if is_lifecycle(&call.name) {
        let host: bool = ctx.connection.query_row(
            "SELECT model='mcp-local' FROM wrapper_engine_runs WHERE run_id=?1",
            [ctx.run_id.unwrap_or("")],
            |r| r.get(0),
        )?;
        if !host || !ctx.run_id.is_some_and(|id| id.starts_with("mcp-host-")) {
            return Err(invalid(
                "Cette action de gestion doit être lancée depuis les paramètres.",
            ));
        }
    }
    for server in servers(ctx.connection)? {
        if call.name == format!("mcp_start_{}", server.id) {
            if call.arguments != lifecycle_arguments(ctx, &server, "start", false)
                && call.arguments != lifecycle_arguments(ctx, &server, "start", true)
            {
                return Err(invalid(
                    "La commande MCP ne correspond pas à la configuration à approuver.",
                ));
            }
            retire(ctx, &server.id);
            if let Some(result) = catalogue::start(ctx, &server, &call.arguments)? {
                return Ok(result);
            }
            let env = environment(ctx, &server)?;
            let client = match Client::spawn(&server.command, &server.arguments, &env) {
                Ok(client) => client,
                Err(error) => {
                    // No process was created. Do not overwrite a simultaneous start.
                    let tx = write_transaction(ctx.connection)?;
                    let registry = ctx
                        .provider
                        .mcp()
                        .0
                        .lock()
                        .unwrap_or_else(|e| e.into_inner());
                    if !registry.contains_key(&server.id) {
                        tx.execute(
                            "UPDATE wrapper_mcp_servers SET error=?2 WHERE id=?1",
                            params![server.id, error.to_string()],
                        )?;
                    }
                    tx.commit()?;
                    return Err(ApiError::new(503, error.to_string()));
                }
            };
            let handle = Handle::new(client);
            let previous = ctx
                .provider
                .mcp()
                .0
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(server.id.clone(), handle.clone());
            drop(previous);
            let discovered = handle
                .client
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .initialize()
                .map_err(|e| ApiError::new(503, e.to_string()));
            let tools = discovered.and_then(|tools| {
                let mut p = active_private(ctx);
                p.extend(handle.environment.clone());
                safe_catalogue(ctx, tools, &p)
            });
            publish(ctx, &server.id, &handle, &tools)?;
            return tools.map(|tools| json!({"message":"Connecteur MCP connecté.","outils":tools}));
        }
        if call.name == format!("mcp_remove_{}", server.id) {
            if call.arguments != lifecycle_arguments(ctx, &server, "remove", false) {
                return Err(invalid(
                    "La suppression MCP ne correspond pas à la configuration à approuver.",
                ));
            }
            if let Some(result) = catalogue::remove(ctx, &server, &call.arguments)? {
                return Ok(result);
            }
            retire(ctx, &server.id);
            let tx = write_transaction(ctx.connection)?;
            tx.execute(
                "DELETE FROM wrapper_mcp_bots WHERE server_id=?1",
                [&server.id],
            )?;
            tx.execute("DELETE FROM wrapper_mcp_servers WHERE id=?1", [&server.id])?;
            tx.execute(
                "DELETE FROM forms_approval_rules WHERE tool GLOB ?1 OR tool IN (?2,?3)",
                params![
                    format!("mcp_{}_*", server.id),
                    format!("mcp_start_{}", server.id),
                    format!("mcp_remove_{}", server.id)
                ],
            )?;
            // A denied Keychain removal must roll back the connector and its
            // permissions, rather than claim failure after deleting its SQL rows.
            crate::features::forms::vault(ctx)?.remove(&secret_name(&server.id))?;
            tx.commit()?;
            return Ok(json!({"message":"Connecteur MCP supprimé et déconnecté."}));
        }
        if let Some(tool) = server
            .tools
            .iter()
            .find(|t| namespace(&server, t) == call.name)
        {
            if !ctx
                .bot_id
                .is_some_and(|bot| enabled(ctx.connection, &server.id, bot))
            {
                return Err(invalid("Ce connecteur MCP est désactivé pour ce bot."));
            }
            catalogue::ensure(ctx, &server)?;
            let handle=ctx.provider.mcp().0.lock().unwrap_or_else(|e|e.into_inner()).get(&server.id).cloned()
                .ok_or_else(||invalid("Le serveur MCP est déconnecté. Démarrez le connecteur dans les paramètres."))?;
            if !handle.ready.load(Ordering::Acquire) {
                return Err(invalid("Le connecteur MCP est en cours de démarrage."));
            }
            let mut private = private_values(ctx)?;
            private.extend(active_private(ctx));
            private.extend(handle.environment.clone());
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut client = loop {
                match handle.client.try_lock() {
                    Ok(client) => break client,
                    Err(std::sync::TryLockError::Poisoned(error)) => break error.into_inner(),
                    Err(std::sync::TryLockError::WouldBlock) => {
                        if std::time::Instant::now() >= deadline {
                            return Err(invalid(
                                "Le connecteur MCP est occupé. Réessayez après l’appel en cours.",
                            ));
                        }
                        let run = grokbot_engine::store::required(
                            ctx.connection,
                            ctx.run_id.unwrap_or(""),
                        )?;
                        if run.status != grokbot_engine::model::RunStatus::Running {
                            return Err(invalid("L’appel MCP a été interrompu."));
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                }
            };
            // A user may stop a queued call while another call owns this client.
            crate::features::forms::verify_tool_admitted(ctx, call)?;
            let output = client.request(
                "tools/call",
                json!({"name":tool["name"],"arguments":call.arguments}),
            );
            drop(client);
            match output {
                Ok(output) => {
                    let valid = output["content"].as_array().is_some_and(|parts| {
                        parts.iter().all(|part| {
                            part.is_object()
                                && part["type"].is_string()
                                && (part["type"] != "text" || part["text"].is_string())
                        })
                    }) && output.get("isError").is_none_or(Value::is_boolean);
                    if !valid {
                        let error=invalid("Le résultat de l’outil MCP est invalide. Le connecteur a été déconnecté.");
                        publish(ctx, &server.id, &handle, &Err(invalid(&error.message)))?;
                        return Err(error);
                    }
                    return sanitize(&output, &private);
                }
                Err(error @ McpError::Rpc { .. }) => {
                    return Err(ApiError::new(400, error.to_string()))
                }
                Err(error @ McpError::Transport(_)) => {
                    let message = error.to_string();
                    catalogue::auth_error(ctx, &server.id, &message);
                    publish(
                        ctx,
                        &server.id,
                        &handle,
                        &Err(ApiError::new(503, message.clone())),
                    )?;
                    return Err(ApiError::new(503, message));
                }
            }
        }
    }
    Err(invalid(
        "Cet outil MCP est inconnu ou son connecteur a été supprimé.",
    ))
}
