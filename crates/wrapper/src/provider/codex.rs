//! Reproduction du chemin SSE de pi-ai 1.3.0 : auth/oauth/openai-codex.js,
//! api/openai-codex-responses.js et providers/data/openai-codex.json (lecture seule).
//! Le serveur OAuth HTTPS fournit les jetons; le JWT décodé ici sert uniquement au
//! routage du backend et à l’affichage, pas de preuve d’identité locale vérifiée.
use super::*;

const CLIENT: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const SCOPE: &str = "openid profile email offline_access";
const AUTH_CLAIM: &str = "https://api.openai.com/auth";
pub(super) const DEFAULT_MODEL: &str = "gpt-6.1-sol";

pub(super) fn authorization_query(attempt: &Attempt) -> Vec<(&'static str, String)> {
    vec![
        ("response_type", "code".into()),
        ("client_id", CLIENT.into()),
        ("redirect_uri", attempt.redirect_uri.clone()),
        ("scope", SCOPE.into()),
        ("code_challenge", challenge(&attempt.verifier)),
        ("code_challenge_method", "S256".into()),
        ("state", attempt.state.clone()),
        ("id_token_add_organizations", "true".into()),
        ("codex_cli_simplified_flow", "true".into()),
        ("originator", "pi".into()),
    ]
}

fn claims(token: &str) -> Option<Value> {
    let payload = token.split('.').nth(1)?;
    serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).ok()?).ok()
}

pub(super) fn update_profile(
    credentials: &mut Credentials,
    tokens: &TokenResponse,
) -> Result<(), Failure> {
    let access = claims(&tokens.access_token)
        .ok_or_else(|| Failure::other("Le jeton Codex reçu est illisible."))?;
    let account_id = access[AUTH_CLAIM]["chatgpt_account_id"]
        .as_str()
        .filter(|id| {
            !id.is_empty()
                && id.len() <= 256
                && id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
        })
        .ok_or_else(|| Failure::other("OpenAI n’a pas fourni l’identifiant du compte Codex."))?;
    if credentials
        .account_id
        .as_deref()
        .is_some_and(|old| old != account_id)
    {
        return Err(Failure::other(
            "Le compte Codex a changé. Déconnectez-vous puis reconnectez-vous.",
        ));
    }
    let identity = tokens
        .id_token
        .as_deref()
        .and_then(claims)
        .unwrap_or(Value::Null);
    credentials.account_id = Some(account_id.into());
    credentials.email = identity["email"]
        .as_str()
        .or_else(|| access["email"].as_str())
        .or_else(|| access["https://api.openai.com/profile"]["email"].as_str())
        .map(str::to_owned)
        .or_else(|| credentials.email.clone());
    credentials.plan = access[AUTH_CLAIM]["chatgpt_plan_type"]
        .as_str()
        .or_else(|| identity[AUTH_CLAIM]["chatgpt_plan_type"].as_str())
        .map(str::to_owned)
        .or_else(|| credentials.plan.clone());
    Ok(())
}

impl Session {
    pub(super) fn complete_codex(&self, attempt: &Attempt, code: &str) -> Result<(), Failure> {
        let response = agent()
            .post(&self.inner.endpoints.token)
            .send_form(&[
                ("grant_type", "authorization_code"),
                ("client_id", CLIENT),
                ("code", code),
                ("code_verifier", &attempt.verifier),
                ("redirect_uri", &attempt.redirect_uri),
            ])
            .map_err(|error| self.request_failure(error).4)?;
        let tokens: TokenResponse = response
            .into_json()
            .map_err(|_| Failure::other("La réponse de connexion Codex est illisible."))?;
        if tokens.refresh_token.as_deref().is_none_or(str::is_empty)
            || tokens.access_token.is_empty()
        {
            return Err(Failure::other(
                "OpenAI n’a pas fourni de session Codex renouvelable.",
            ));
        }
        // Annulation et écriture sont ordonnées : une ancienne tentative ne peut
        // pas ressusciter une connexion après Annuler / Se déconnecter.
        let _signin = lock(&self.inner.signin);
        if attempt.cancel.load(Ordering::SeqCst) {
            return Err(Failure::other("Connexion annulée."));
        }
        let _guard = lock(&self.inner.credentials);
        let mut credentials = self.load_or_create().map_err(|_| {
            Failure::other("Impossible d’enregistrer la connexion Codex sur ce Mac.")
        })?;
        update_profile(&mut credentials, &tokens)?;
        credentials.client_id = Some(CLIENT.into());
        credentials.access_token = Some(tokens.access_token);
        credentials.refresh_token = tokens.refresh_token;
        credentials.id_token = tokens.id_token;
        credentials.scopes = SCOPE.into();
        credentials.expires_at = seconds() + tokens.expires_in;
        credentials.saved_at = seconds();
        self.write(&credentials).map_err(|_| {
            Failure::other("Impossible d’enregistrer la connexion Codex sur ce Mac.")
        })?;
        self.inner.local_revoked.store(false, Ordering::SeqCst);
        *lock(&self.inner.catalog) = Catalog::default();
        Ok(())
    }

    /// Point commun à TOUTES les requêtes (chat, outils, images, titres, mémoire,
    /// revue des approbations). Seule la variante Codex adapte le corps Responses.
    pub(super) fn request(&self, token: &str, body: &mut Value) -> Result<ureq::Request, Failure> {
        if self.inner.local_revoked.load(Ordering::SeqCst) {
            return Err(Failure::signin());
        }
        let mut request = agent()
            .post(&format!("{}/responses", self.inner.endpoints.api))
            .set("Authorization", &format!("Bearer {token}"));
        if self.inner.codex {
            prepare_body(body)?;
            let access = claims(token).ok_or_else(Failure::signin)?;
            let account = access[AUTH_CLAIM]["chatgpt_account_id"]
                .as_str()
                .ok_or_else(Failure::signin)?;
            request = request
                .set("chatgpt-account-id", account)
                .set("originator", "pi")
                // Même famille d’en-têtes que pi; identité de l’application explicite.
                .set("User-Agent", "ezerbot/0.1.0")
                .set("OpenAI-Beta", "responses=experimental");
            if let Some(session) = body["prompt_cache_key"].as_str() {
                request = request
                    .set("session-id", session)
                    .set("x-client-request-id", session);
            }
        }
        Ok(request.set("Content-Type", "application/json").set(
            "Accept",
            if body["stream"] == true {
                "text/event-stream"
            } else {
                "application/json"
            },
        ))
    }
}

fn prepare_body(body: &mut Value) -> Result<(), Failure> {
    let object = body
        .as_object_mut()
        .ok_or_else(|| Failure::other("Demande Codex invalide."))?;
    object.insert("store".into(), json!(false));
    object.insert("stream".into(), json!(true));
    if object
        .get("instructions")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        object.insert("instructions".into(), json!("Tu es un assistant utile."));
    }
    object.insert("include".into(), json!(["reasoning.encrypted_content"]));
    object.entry("text").or_insert(json!({}))["verbosity"] = json!("low");
    object.insert("tool_choice".into(), json!("auto"));
    object.insert("parallel_tool_calls".into(), json!(true));
    // Le backend Codex refuse ces champs, même dans les petits appels internes.
    object.remove("max_output_tokens");
    object.remove("temperature");
    if !object.contains_key("reasoning") {
        let effort = if supports_no_reasoning(object["model"].as_str().unwrap_or_default()) {
            "none"
        } else {
            "medium"
        };
        object.insert("reasoning".into(), json!({"effort":effort}));
    }
    if object["reasoning"].get("summary").is_none() {
        object.get_mut("reasoning").expect("inséré")["summary"] = json!("auto");
    }
    if let Some(tools) = object.get_mut("tools").and_then(Value::as_array_mut) {
        for tool in tools {
            tool["strict"] = Value::Null;
        }
    }
    Ok(())
}

// pi's thinkingLevelMap.off is null for these two models; the others map
// off to "none". Advertise wire efforts, not duplicate aliases (minimal → low).
fn supports_no_reasoning(model: &str) -> bool {
    !matches!(model, "gpt-6-astra" | "gpt-6.1-sol")
}

/// Catalogue statique (et non /v1/models, réservé au partage).
/// Ces modèles ont été acceptés par un vrai compte ChatGPT le 2026-10-20.
/// Le défaut vision généraliste est aussi le repli pour les descriptions d’image.
pub(super) fn models() -> Vec<Model> {
    [
        (DEFAULT_MODEL, "GPT-6.1 Sol"),
        ("gpt-5.6-sol", "GPT-5.6 Sol"),
        ("gpt-5.6-luna", "GPT-5.6 Luna"),
        ("gpt-5.6-terra", "GPT-5.6 Terra"),
        ("gpt-6-astra", "GPT-6 Astra"),
        ("gpt-6-luna", "GPT-6 Luna"),
        ("gpt-6-sol", "GPT-6 Sol"),
    ]
    .into_iter()
    .map(|(id, name)| Model {
        slug: id.into(),
        name: name.into(),
        description: "Quota Codex · texte et images".into(),
        efforts: supports_no_reasoning(id)
            .then_some(("none", "Aucun"))
            .into_iter()
            .chain([
                ("low", "Léger"),
                ("medium", "Équilibré"),
                ("high", "Approfondi"),
                ("xhigh", "Très approfondi"),
                ("max", "Maximum"),
            ])
            .map(|(effort, description)| Effort {
                effort: effort.into(),
                description: description.into(),
            })
            .collect(),
        default_effort: Some("medium".into()),
        tiers: vec![],
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn body_keeps_tools_images_cache_and_reasoning_but_removes_unsupported_limits() {
        let mut body = json!({"model":DEFAULT_MODEL,"instructions":"Consignes", "store":true,
            "input":[{"role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,AA=="}]}],
            "tools":[{"type":"function","name":"remember","parameters":{"type":"object"}}],
            "prompt_cache_key":"conversation", "reasoning":{"effort":"high"}, "max_output_tokens":900});
        prepare_body(&mut body).unwrap();
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
        assert_eq!(body["reasoning"], json!({"effort":"high","summary":"auto"}));
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        assert_eq!(body["input"][0]["content"][0]["type"], "input_image");
        assert_eq!(body["tools"][0]["name"], "remember");
        assert_eq!(body["prompt_cache_key"], "conversation");
        assert!(body.get("max_output_tokens").is_none());
    }
}
