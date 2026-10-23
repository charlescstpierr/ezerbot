//! Authorization-code + PKCE broker. Callback listener is loopback-only,
//! single-use, state-bound, bounded in time and invalidated by disconnect/restart.
use super::*;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use std::{io::Read, time::Duration};
use tiny_http::{Header, Response, Server as HttpServer, StatusCode};

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(Duration::from_secs(12))
        .build()
}
fn fail() -> ApiError {
    invalid("L’autorisation du service a échoué. Vérifiez les réglages de l’application OAuth, puis reconnectez le service.")
}
fn read_json(response: ureq::Response) -> Result<Value, ApiError> {
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(32_769)
        .read_to_end(&mut bytes)
        .map_err(|_| fail())?;
    if bytes.len() > 32_768 {
        return Err(fail());
    }
    serde_json::from_slice(&bytes).map_err(|_| fail())
}
fn form(url: &str, fields: &[(String, String)]) -> Result<Value, ApiError> {
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().map(|(k, v)| (k, v)))
        .finish();
    read_json(
        agent()
            .post(url)
            .set("Accept", "application/json")
            .set("Content-Type", "application/x-www-form-urlencoded")
            .send_string(&body)
            .map_err(|_| fail())?,
    )
}
fn random() -> Result<String, ApiError> {
    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| invalid("Impossible de préparer une connexion sécurisée."))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
/// What identifies the client at the token and revocation endpoints. Gmail's and Slack's is
/// the public client of the build and nothing else: no secret is read, so none can be sent.
fn token_fields(m: Manifest, env: &HashMap<String, String>) -> Vec<(String, String)> {
    let public = match m.id {
        "gmail" => Some(gmail::client_id()),
        "slack" => Some(slack::client_id()),
        _ => None,
    };
    if let Some(client) = public {
        return client
            .map(|id| vec![("client_id".into(), id)])
            .unwrap_or_default();
    }
    ["CLIENT_ID", "CLIENT_SECRET"]
        .into_iter()
        .filter_map(|k| env.get(k).map(|v| (k.to_lowercase(), v.clone())))
        .collect()
}
fn validate_scopes(m: Manifest, scopes: Option<&str>) -> Result<(), ApiError> {
    // Notion's "default" is a broker selector, not a permission that its token
    // response must echo. The user chooses pages/permissions at consent.
    if m.id == "notion" {
        return Ok(());
    }
    if let Some(scopes) = scopes {
        let granted: BTreeSet<_> = scopes
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|s| !s.is_empty())
            .collect();
        if m.scopes.split_whitespace().any(|scope| {
            !granted.contains(scope)
                && !(scope == "email"
                    && granted.contains("https://www.googleapis.com/auth/userinfo.email"))
        }) {
            return Err(invalid("Certaines autorisations nécessaires n’ont pas été accordées. Reconnectez le service et vérifiez les portées demandées."));
        }
    }
    Ok(())
}
fn accept_token(
    ctx: &Context<'_>,
    server: &super::super::Server,
    m: Manifest,
    env: &mut HashMap<String, String>,
    raw: Value,
) -> Result<(), ApiError> {
    if raw.get("error").is_some() || raw["ok"] == false {
        return Err(fail());
    }
    // Slack names the workspace beside the user token, not inside it.
    let workspace = raw["team"]["name"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty() && s.len() <= 100 && !s.chars().any(char::is_control))
        .map(str::to_owned);
    let raw = if raw["authed_user"]["access_token"].is_string() {
        raw["authed_user"].clone()
    } else {
        raw
    };
    if raw["token_type"]
        .as_str()
        .is_some_and(|v| !v.eq_ignore_ascii_case("bearer") && !(m.id == "slack" && v == "user"))
    {
        return Err(fail());
    }
    validate_scopes(m, raw["scope"].as_str())?;
    let access = raw["access_token"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 3000 && !s.chars().any(char::is_control))
        .ok_or_else(fail)?;
    env.insert("ACCESS_TOKEN".into(), access.into());
    if let Some(refresh) = raw["refresh_token"].as_str().filter(|s| !s.is_empty()) {
        env.insert("REFRESH_TOKEN".into(), refresh.into());
    }
    let expires = raw["expires_in"]
        .as_u64()
        .map(|s| {
            wall_clock()
                .saturating_add(s.saturating_mul(1000))
                .min(i64::MAX as u64) as i64
        })
        .unwrap_or(0);
    if m.id == "gmail" {
        // A renewal keeps the account it already named; a sign-in learns it from Google.
        if !env.contains_key("EMAIL") {
            let email = gmail::profile_email(m, access)?;
            // One connection per Google address: a second sign-in of it adds nothing.
            gmail::refuse_twin(ctx, server, &email)?;
            env.insert("EMAIL".into(), email);
        }
    } else if m.endpoint.is_empty() {
        let user = read_json(
            agent()
                .get(&endpoint(m, "userinfo"))
                .set("Authorization", &format!("Bearer {access}"))
                .call()
                .map_err(|_| fail())?,
        )?;
        let email = user["email"]
            .as_str()
            .filter(|s| s.len() < 254 && s.contains('@') && !s.chars().any(char::is_control))
            .ok_or_else(fail)?;
        env.insert("EMAIL".into(), email.into());
    }
    if m.id == "slack" {
        // The workspace is the account shown on the card; a renewal keeps the one it had.
        // The exchange may give the team's identifier only: the token names the workspace.
        let workspace = workspace.or_else(|| {
            env.get("EMAIL")
                .is_none()
                .then(|| slack::workspace_of(m, access))
                .flatten()
        });
        if let Some(workspace) = workspace {
            env.insert("EMAIL".into(), workspace);
        } else {
            env.entry("EMAIL".into()).or_insert_with(|| "Slack".into());
        }
        // A client typed under an earlier version is not carried into the new secret.
        env.remove("CLIENT_ID");
        env.remove("CLIENT_SECRET");
        slack::write_tokens(ctx, server, env)?;
    } else {
        save(ctx, &server.id, env).map_err(|error| {
            if m.id == "gmail" {
                gmail::refused_write(ctx)
            } else {
                error
            }
        })?;
    }
    ctx.connection.execute(
        "UPDATE wrapper_connector_catalogue SET expires_at=?2 WHERE server_id=?1",
        params![server.id, expires],
    )?;
    if m.id == "gmail" {
        ctx.connection.execute(
            "UPDATE wrapper_connector_catalogue SET account_id=lower(account_email) WHERE server_id=?1",
            [&server.id],
        )?;
    }
    Ok(())
}
pub(super) fn refresh(
    ctx: &Context<'_>,
    server: &super::super::Server,
    m: Manifest,
    env: &mut HashMap<String, String>,
) -> Result<(), ApiError> {
    if m.id == "github" {
        return super::github::refresh(ctx, server, m, env);
    }
    let mut fields = token_fields(m, env);
    fields.push(("grant_type".into(), "refresh_token".into()));
    fields.push((
        "refresh_token".into(),
        env.get("REFRESH_TOKEN").ok_or_else(fail)?.clone(),
    ));
    let raw = form(&endpoint(m, "token"), &fields)?;
    let mut renewed = env.clone();
    accept_token(ctx, server, m, &mut renewed, raw)?;
    *env = renewed;
    Ok(())
}
pub(super) fn begin(
    ctx: &Context<'_>,
    server: &super::super::Server,
    m: Manifest,
    mut env: HashMap<String, String>,
) -> Result<Value, ApiError> {
    let listener = HttpServer::http(("127.0.0.1",if m.id=="slack" && fake_base().is_none() {52187} else {0})).map_err(|_|invalid("Le port de retour OAuth est occupé. Fermez l’autre connexion en cours, puis réessayez."))?;
    let port = listener.server_addr().to_ip().ok_or_else(fail)?.port();
    let redirect = format!("http://127.0.0.1:{port}/oauth/{}/callback", m.id);
    let state = random()?;
    let verifier = random()?;
    if m.id == "notion" {
        let registered = read_json(agent().post(&endpoint(m,"register")).send_json(json!({"client_name":"Ezerbot","redirect_uris":[redirect],"grant_types":["authorization_code","refresh_token"],"response_types":["code"],"token_endpoint_auth_method":"none"})).map_err(|_|fail())?)?;
        let id = registered["client_id"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 512)
            .ok_or_else(fail)?;
        env.insert("CLIENT_ID".into(), id.into());
        env.remove("CLIENT_SECRET");
        if let Some(secret) = registered["client_secret"].as_str() {
            env.insert("CLIENT_SECRET".into(), secret.into());
        }
        save(ctx, &server.id, &env)?;
    }
    if fake_base().is_some() && !matches!(m.id, "gmail" | "slack") && !env.contains_key("CLIENT_ID")
    {
        env.insert("CLIENT_ID".into(), "ezerbot-factice".into());
        save(ctx, &server.id, &env)?;
    }
    // Gmail's and Slack's client is the build's, never read from the vault.
    // A dynamic client is registered for this exact redirect URI. Notion assigns
    // a new loopback port on every attempt, so register anew after restart too.
    let client = match m.id {
        "gmail" => gmail::client_id().ok_or_else(gmail::not_configured)?,
        "slack" => slack::client_id().ok_or_else(slack::not_configured)?,
        _ => env.get("CLIENT_ID").ok_or_else(fail)?.clone(),
    };
    let mut url = url::Url::parse(&endpoint(m, "authorize")).map_err(|_| fail())?;
    {
        let mut query = url.query_pairs_mut();
        query
            .append_pair("response_type", "code")
            .append_pair("client_id", &client)
            .append_pair("redirect_uri", &redirect)
            .append_pair("state", &state)
            .append_pair(
                "code_challenge",
                &URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
            )
            .append_pair("code_challenge_method", "S256")
            .append_pair("scope", m.scopes);
        if m.endpoint.is_empty() {
            query.append_pair("access_type", "offline").append_pair(
                "prompt",
                if m.id == "gmail" {
                    "select_account consent"
                } else {
                    "consent"
                },
            );
        }
        if m.id == "notion" {
            query.append_pair("resource", m.endpoint);
        }
    }
    ctx.connection.execute("UPDATE wrapper_connector_catalogue SET generation=generation+1,phase='authorizing',message='Terminez la connexion dans le navigateur (5 minutes).',authorization_url=?2 WHERE server_id=?1",params![server.id,url.as_str()])?;
    let generation: i64 = ctx.connection.query_row(
        "SELECT generation FROM wrapper_connector_catalogue WHERE server_id=?1",
        [&server.id],
        |r| r.get(0),
    )?;
    let path = ctx.db_path.to_path_buf();
    let provider = ctx.provider.clone();
    let server = server.clone();
    #[cfg(target_os = "macos")]
    let authorization_url = url.to_string();
    std::thread::spawn(move || {
        let result = (|| -> Result<(), ApiError> {
            let conn = Connection::open(&path)?;
            conn.busy_timeout(Duration::from_secs(4))?;
            let ctx = Context {
                connection: &conn,
                provider: &provider,
                db_path: &path,
                now_ms: wall_clock(),
                bot_id: None,
                conversation_id: None,
                run_id: None,
            };
            let current = || -> bool {
                conn.query_row("SELECT EXISTS(SELECT 1 FROM wrapper_connector_catalogue WHERE server_id=?1 AND generation=?2 AND phase='authorizing')",params![server.id,generation],|r|r.get(0)).unwrap_or(false)
            };
            let deadline = std::time::Instant::now() + Duration::from_secs(300);
            while std::time::Instant::now() < deadline && current() {
                let Some(request) = listener
                    .recv_timeout(Duration::from_millis(250))
                    .map_err(|_| fail())?
                else {
                    continue;
                };
                let parsed = url::Url::parse(&format!("http://127.0.0.1:{port}{}", request.url()));
                let values = parsed
                    .ok()
                    .filter(|u| u.path() == format!("/oauth/{}/callback", m.id))
                    .map(|u| {
                        u.query_pairs()
                            .map(|(k, v)| (k.into_owned(), v.into_owned()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let value = |key: &str| {
                    let mut found = values.iter().filter(|(k, _)| k == key);
                    let first = found.next().map(|(_, v)| v.as_str());
                    if found.next().is_some() {
                        None
                    } else {
                        first
                    }
                };
                let host = request
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Host"))
                    .map(|h| h.value.as_str());
                if request.method().as_str() != "GET"
                    || request.url().len() > 8192
                    || host != Some(format!("127.0.0.1:{port}").as_str())
                    || value("state") != Some(state.as_str())
                {
                    respond(
                        request,
                        400,
                        m.name,
                        "Retour OAuth invalide. Revenez dans Ezerbot pour reconnecter le service.",
                    );
                    continue;
                }
                let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
                if !current() {
                    respond(
                        request,
                        409,
                        m.name,
                        "Cette demande a été remplacée ou annulée.",
                    );
                    return Ok(());
                }
                let completion = (|| -> Result<(), ApiError> {
                    if value("error").is_some() {
                        return Err(invalid("Vous avez refusé l’autorisation du service. Aucun nouveau jeton n’a été enregistré."));
                    }
                    let code = value("code").filter(|s| !s.is_empty()).ok_or_else(fail)?;
                    let mut fields = token_fields(m, &env);
                    fields.extend([
                        ("grant_type".into(), "authorization_code".into()),
                        ("code".into(), code.into()),
                        ("redirect_uri".into(), redirect.clone()),
                        ("code_verifier".into(), verifier.clone()),
                    ]);
                    let raw = form(&endpoint(m, "token"), &fields)?;
                    // A new login must not retain a refresh token from another account.
                    env.remove("ACCESS_TOKEN");
                    env.remove("REFRESH_TOKEN");
                    env.remove("EMAIL");
                    accept_token(&ctx, &server, m, &mut env, raw)?;
                    // A new sign-in may select another account. Do not carry
                    // tool permissions or bot activation to that new identity.
                    let tx = write_transaction(ctx.connection)?;
                    tx.execute(
                        "DELETE FROM wrapper_mcp_bots WHERE server_id=?1",
                        [&server.id],
                    )?;
                    tx.execute(
                        "DELETE FROM forms_approval_rules WHERE tool GLOB ?1",
                        [format!("mcp_{}_*", server.id)],
                    )?;
                    tx.commit()?;
                    let attached = attach(&ctx, &server, m, &env);
                    if attached.is_err() && m.id == "gmail" {
                        // A connection that did not come up keeps no token.
                        if let Ok(vault) = crate::features::forms::vault(&ctx) {
                            let _ = vault.remove(&secret_name(&server.id));
                        }
                    }
                    attached?;
                    Ok(())
                })();
                match completion {
                    Ok(())=>respond(request,200,m.name,"Les outils de ce service sont prêts à être activés pour vos bots dans Ezerbot."),
                    Err(error)=>{phase(&ctx,&server.id,"reconnect",&error.message)?;respond(request,400,m.name,"La connexion n’a pas abouti. Revenez dans Ezerbot pour voir les indications et réessayer.");},
                }
                return Ok(());
            }
            let _guard = EFFECTS.lock().unwrap_or_else(|e| e.into_inner());
            if current() {
                phase(
                    &ctx,
                    &server.id,
                    "reconnect",
                    "La connexion a expiré. Cliquez sur Reconnecter pour réessayer.",
                )?;
            }
            Ok(())
        })();
        // Never print provider errors, codes, redirect queries or token bodies.
        let _ = result;
    });
    if fake_base().is_none() {
        #[cfg(target_os = "macos")]
        if std::process::Command::new("/usr/bin/open")
            .arg(&authorization_url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_err()
        {
            return Ok(
                json!({"message":"Ouvrez le lien de connexion dans Paramètres › Connecteurs pour continuer."}),
            );
        }
    }
    Ok(
        json!({"message":"Connexion en attente dans le navigateur. Retrouvez le lien et l’état dans Paramètres › Connecteurs."}),
    )
}
fn callback_html(status: u16, service: &str, text: &str) -> String {
    fn escaped(text: &str) -> String {
        text.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&#39;")
    }
    // Replace public, escaped text last; the template contains no remote resources.
    include_str!("callback.html")
        .replace(
            "{{status}}",
            if status == 200 { "success" } else { "error" },
        )
        .replace(
            "{{title}}",
            if status == 200 {
                "Connexion réussie"
            } else {
                "Connexion interrompue"
            },
        )
        .replace("{{service}}", &escaped(service))
        .replace("{{message}}", &escaped(text))
}
fn respond(request: tiny_http::Request, status: u16, service: &str, text: &str) {
    let html = callback_html(status, service, text);
    let _ = request.respond(
        Response::from_string(html)
            .with_status_code(StatusCode(status))
            .with_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap())
            .with_header(
                Header::from_bytes(
                    "Content-Security-Policy",
                    "default-src 'none'; style-src 'unsafe-inline'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
                )
                .unwrap(),
            )
            .with_header(Header::from_bytes("Referrer-Policy", "no-referrer").unwrap())
            .with_header(Header::from_bytes("Cache-Control", "no-store").unwrap()),
    );
}
pub(super) fn revoke(m: Manifest, env: &HashMap<String, String>) -> bool {
    let token = if m.id == "slack" {
        env.get("ACCESS_TOKEN")
    } else {
        env.get("REFRESH_TOKEN").or_else(|| env.get("ACCESS_TOKEN"))
    };
    if m.id == "github" {
        return super::github::revoke(m, env);
    }
    let Some(token) = token else { return true };
    let mut fields = token_fields(m, env);
    fields.push(("token".into(), token.clone()));
    if m.id == "notion" {
        fields.push((
            "token_type_hint".into(),
            if env.contains_key("REFRESH_TOKEN") {
                "refresh_token"
            } else {
                "access_token"
            }
            .into(),
        ));
    }
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().map(|(k, v)| (k, v)))
        .finish();
    match agent()
        .post(&endpoint(m, "revoke"))
        .set("Content-Type", "application/x-www-form-urlencoded")
        .send_string(&body)
    {
        Ok(r) if m.id == "slack" => {
            read_json(r).is_ok_and(|v| v["ok"] == true && v["revoked"] == true)
        }
        Ok(_) => true,
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notion_consent_scopes_need_not_echo_default_but_other_services_stay_strict() {
        let notion = manifest("notion").unwrap();
        for scopes in [
            None,
            Some(""),
            Some("pages:read pages:write"),
            Some("default"),
        ] {
            assert!(validate_scopes(notion, scopes).is_ok());
        }
        let gmail = manifest("gmail").unwrap();
        assert!(validate_scopes(gmail, Some("")).is_err());
        assert!(validate_scopes(gmail, Some("openid email")).is_err());
        assert!(validate_scopes(gmail, Some(gmail.scopes)).is_ok());
        let slack = manifest("slack").unwrap();
        assert!(validate_scopes(slack, Some(&slack.scopes.replace(' ', ","))).is_ok());
    }
    #[test]
    fn callback_pages_are_self_contained_themed_and_escape_public_text() {
        for (status, title, tone) in [
            (200, "Connexion réussie", "success"),
            (400, "Connexion interrompue", "error"),
        ] {
            let html = callback_html(status, "Gmail <test>", "Un texte <img src=x> & un conseil.");
            assert!(html.contains(title));
            assert!(html.contains(&format!("data-status=\"{tone}\"")));
            assert!(html.contains("prefers-color-scheme: dark"));
            assert!(html.contains("max-width:420px"));
            assert!(html.contains("Gmail &lt;test&gt;"));
            assert!(!html.contains("<img"));
            assert!(html.contains("Vous pouvez fermer cet onglet et revenir dans Ezerbot."));
            assert!(!html.contains("<script"));
            assert!(!html.contains("{{"));
        }
    }
}
