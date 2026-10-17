//! Le parcours « Sign in with ChatGPT » contre un faux serveur OpenAI : enregistrement, échange du
//! code, vérification de l'identité, renouvellement, catalogue, réponse en flux et déconnexion.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use grokbot_wrapper::{App, Endpoints, Provider};
use jsonwebtoken::{encode, Algorithm, EncodingKey, Header};
use serde_json::{json, Value};

// Clé jetable, créée pour ces tests. Elle ne protège rien.
const SIGNING_KEY: &[u8] = include_bytes!("fixtures/test-signing-key.pem");
const JWKS: &str = include_str!("fixtures/test-jwks.json");
const CLIENT: &str = "oaiapp_test";

#[derive(Default)]
struct Seen {
    nonce: String,
    forms: Vec<HashMap<String, String>>,
    bearer: Vec<String>,
    bodies: Vec<Value>,
    revoked: bool,
}

fn form(body: &str) -> HashMap<String, String> {
    body.split('&')
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.to_owned(), unescape(value)))
        .collect()
}

fn unescape(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() + 1 {
            out.push(u8::from_str_radix(&value[index + 1..index + 3], 16).unwrap());
            index += 3;
        } else {
            out.push(if bytes[index] == b'+' {
                b' '
            } else {
                bytes[index]
            });
            index += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

fn id_token(issuer: &str, nonce: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some("test-key".into());
    let claims = json!({
        "iss": issuer, "aud": CLIENT, "sub": "user-1", "email": "vous@exemple.com",
        "nonce": nonce, "iat": now, "exp": now + 600,
    });
    encode(
        &header,
        &claims,
        &EncodingKey::from_rsa_pem(SIGNING_KEY).unwrap(),
    )
    .unwrap()
}

/// Faux serveur d'autorisation et d'API. Le premier jeton d'accès expire aussitôt, ce qui force
/// un renouvellement avant la première demande.
fn fake_openai(seen: Arc<Mutex<Seen>>) -> String {
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let base = format!(
        "http://127.0.0.1:{}",
        server.server_addr().to_ip().unwrap().port()
    );
    let issuer = base.clone();
    std::thread::spawn(move || {
        for mut request in server.incoming_requests() {
            let mut body = String::new();
            request.as_reader().read_to_string(&mut body).unwrap();
            let bearer = request
                .headers()
                .iter()
                .find(|header| header.field.equiv("Authorization"))
                .map(|header| header.value.to_string())
                .unwrap_or_default();
            let path = request.url().to_owned();
            let mut seen = seen.lock().unwrap();
            let (status, content_type, payload) = match path.as_str() {
                "/jwks" => (200, "application/json", JWKS.to_owned()),
                "/token" => {
                    let fields = form(&body);
                    let refresh = fields["grant_type"] == "refresh_token";
                    seen.forms.push(fields);
                    let reply = json!({
                        "access_token": if refresh { "access-2" } else { "access-1" },
                        "refresh_token": if refresh { "refresh-2" } else { "refresh-1" },
                        "id_token": id_token(&issuer, &seen.nonce),
                        "token_type": "Bearer", "expires_in": if refresh { 3600 } else { 1 },
                        "scope": "chatgpt.tokens.use.direct email offline_access openid profile resource.invoke",
                    });
                    (200, "application/json", reply.to_string())
                }
                "/revoke" => {
                    seen.revoked = true;
                    (200, "application/json", String::new())
                }
                "/v1/models" => {
                    seen.bearer.push(bearer);
                    let models = json!({"models": [
                        {"slug": "gpt-test", "display_name": "GPT Test", "description": "Modèle de test", "visibility": "list",
                         "default_reasoning_level": "medium", "default_reasoning_summary": "none",
                         "supported_reasoning_levels": [{"effort": "low", "description": "Léger"}, {"effort": "medium", "description": "Équilibré"}, {"effort": "high", "description": "Profond"}],
                         "service_tiers": [{"id": "priority", "name": "Fast", "description": "2x speed"}]},
                        {"slug": "internal", "display_name": "Internal", "visibility": "hide"},
                    ]});
                    (200, "application/json", models.to_string())
                }
                "/v1/responses" => {
                    seen.bearer.push(bearer);
                    let request_body: Value = serde_json::from_str(&body).unwrap();
                    let fast = request_body.get("service_tier").is_some();
                    seen.bodies.push(request_body.clone());
                    if request_body["instructions"]
                        .as_str()
                        .unwrap()
                        .contains("titre")
                    {
                        let title = json!({"type": "response.output_text.delta", "delta": "« Salutations en français. »"});
                        (
                            200,
                            "text/event-stream",
                            format!(
                                "data: {title}\n\ndata: {}\n\n",
                                json!({"type": "response.completed"})
                            ),
                        )
                    } else if fast {
                        let refusal = json!({"error": {"code": "subscription_sharing_unsupported_capability", "message": "unsupported", "param": "service_tier"}});
                        (400, "application/json", refusal.to_string())
                    } else {
                        let events = [
                            json!({"type": "response.created"}),
                            json!({"type": "response.output_text.delta", "delta": "Bon"}),
                            json!({"type": "response.output_text.delta", "delta": "jour **vous**"}),
                            json!({"type": "response.completed"}),
                        ];
                        let stream: String = events
                            .iter()
                            .map(|event| {
                                format!(
                                    "event: {}\ndata: {event}\n\n",
                                    event["type"].as_str().unwrap()
                                )
                            })
                            .collect();
                        (200, "text/event-stream", stream)
                    }
                }
                _ => (404, "text/plain", String::new()),
            };
            drop(seen);
            let header = tiny_http::Header::from_bytes("Content-Type", content_type).unwrap();
            let _ = request.respond(
                tiny_http::Response::from_string(payload)
                    .with_status_code(status)
                    .with_header(header),
            );
        }
    });
    base
}

fn wait(what: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !ready() {
        assert!(Instant::now() < deadline, "délai dépassé : {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn post(app: &mut App, path: &str, body: Value) -> Value {
    app.api("POST", path, body.to_string().as_bytes()).unwrap()
}

fn assistant(app: &mut App) -> Value {
    let state = app.api("GET", "/api/state", b"").unwrap();
    state["conversations"][0]["messages"]
        .as_array()
        .and_then(|messages| {
            messages
                .iter()
                .rev()
                .find(|message| message["role"] == "assistant")
                .cloned()
        })
        .unwrap_or(Value::Null)
}

#[test]
fn chatgpt_sign_in_streams_a_real_reply_and_signs_out() {
    let directory: PathBuf =
        std::env::temp_dir().join(format!("grokbot-chatgpt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let db = directory.join("app.sqlite");
    let seen = Arc::new(Mutex::new(Seen::default()));
    let base = fake_openai(seen.clone());
    let endpoints = Endpoints {
        authorize: format!("{base}/authorize"),
        token: format!("{base}/token"),
        revoke: format!("{base}/revoke"),
        jwks: format!("{base}/jwks"),
        issuer: base.clone(),
        api: format!("{base}/v1"),
    };
    let provider = Provider::with_endpoints(&db, endpoints, false);
    let mut app = App::open_with(&db, provider.clone()).unwrap();
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["mode"],
        "simulated"
    );

    // Premier enregistrement : le gabarit de client, le nom de l'application, PKCE et l'hôte.
    let url = provider.start_signin().unwrap();
    let query = form(url.split_once('?').unwrap().1);
    assert_eq!(query["client_id"], "dynamic_agent_client");
    assert_eq!(query["agent_name_hint"], "Grokbot");
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["resource"], "https://api.openai.com/v1");
    assert!(query["scope"].contains("chatgpt.tokens.use.direct"));
    assert!(query["ext_agent_host_id"].starts_with("urn:uuid:"));
    assert!(
        query["redirect_uri"].starts_with("http://127.0.0.1:")
            && query["redirect_uri"].ends_with("/auth/callback")
    );
    assert_eq!(provider.status()["signin"], "pending");
    seen.lock().unwrap().nonce = query["nonce"].clone();

    // Un retour avec un mauvais état est ignoré; le bon termine la connexion.
    let callback = |state: &str| {
        ureq::get(&format!(
            "{}?code=good-code&state={state}&client_id={CLIENT}",
            query["redirect_uri"]
        ))
        .call()
        .ok()
        .map(|response| response.into_string().unwrap())
    };
    assert!(callback("wrong-state").is_none());
    assert_eq!(provider.status()["signin"], "pending");
    assert!(callback(&query["state"])
        .unwrap()
        .contains("Connexion réussie"));
    wait("connexion", || provider.status()["connected"] == true);
    assert_eq!(provider.status()["email"], "vous@exemple.com");
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.forms[0]["grant_type"], "authorization_code");
        assert_eq!(seen.forms[0]["client_id"], CLIENT);
        assert_eq!(seen.forms[0]["code"], "good-code");
        assert_eq!(seen.forms[0]["redirect_uri"], query["redirect_uri"]);
    }
    let credentials = directory.join("app.chatgpt-auth.json");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&credentials)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    // Le catalogue vient du compte : le modèle caché n'apparaît pas, les niveaux d'effort suivent.
    wait("catalogue", || provider.status()["models_ready"] == true);
    let state = app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(state["mode"], "chatgpt");
    assert_eq!(
        state["provider"]["models"],
        json!([{
            "slug": "gpt-test", "name": "GPT Test", "description": "Modèle de test", "default_effort": "medium",
            "efforts": [{"effort": "low", "description": "Léger"}, {"effort": "medium", "description": "Équilibré"}, {"effort": "high", "description": "Profond"}],
            "tiers": [{"id": "priority", "name": "Fast", "description": "2x speed"}],
        }])
    );
    assert_eq!(state["preferences"]["model"], "gpt-test");
    assert_eq!(state["preferences"]["effort"], "medium");
    let chosen = post(
        &mut app,
        "/api/preferences",
        json!({"effort": "high", "fast": true}),
    );
    assert_eq!(
        chosen["preferences"],
        json!({"model": "gpt-test", "effort": "high", "fast": true})
    );

    // La réponse arrive en flux, en arrière-plan. La vitesse Fast, refusée, est retirée et signalée.
    let conversation = post(&mut app, "/api/conversations", json!({}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let sent = post(
        &mut app,
        "/api/send",
        json!({"conversation_id": conversation, "message_id": "m1", "body": "Bonjour"}),
    );
    assert_eq!(sent["conversation"]["messages"][0]["state"], "echoed");
    wait("réponse", || assistant(&mut app)["state"] == "echoed");
    let reply = assistant(&mut app);
    assert_eq!(reply["body"], "Bonjour **vous**");
    assert!(reply["notice"].as_str().unwrap().contains("Fast"));
    {
        let seen = seen.lock().unwrap();
        // Le premier jeton expirait : chaque appel d'API porte le jeton renouvelé.
        assert!(seen
            .forms
            .iter()
            .any(|fields| fields["grant_type"] == "refresh_token"
                && fields["refresh_token"] == "refresh-1"));
        assert!(seen.bearer.iter().all(|bearer| bearer == "Bearer access-2"));
        let first = &seen.bodies[0];
        assert_eq!(
            (first["store"].clone(), first["stream"].clone()),
            (json!(false), json!(true))
        );
        assert_eq!(first["model"], "gpt-test");
        assert_eq!(first["reasoning"]["effort"], "high");
        assert_eq!(
            first["input"],
            json!([{"role": "user", "content": "Bonjour"}])
        );
        assert!(seen.bodies[1].get("service_tier").is_none());
        assert!(seen
            .bodies
            .iter()
            .all(|body| body["store"] == false && body["stream"] == true));
    }

    // Le sujet donne son titre à la conversation, nettoyé; la demande de titre reste hors du fil.
    wait("titre", || {
        app.api("GET", "/api/state", b"").unwrap()["conversations"][0]["title"]
            == "Salutations en français"
    });
    wait("fin du titre", || provider.status()["titling"] == false);
    let chat_bodies = |seen: &Seen| -> Vec<Value> {
        seen.bodies
            .iter()
            .filter(|body| !body["instructions"].as_str().unwrap().contains("titre"))
            .cloned()
            .collect()
    };
    assert!(seen
        .lock()
        .unwrap()
        .bodies
        .iter()
        .any(|body| body["input"][0]["content"]
            .as_str()
            .unwrap()
            .contains("Utilisateur : Bonjour")));

    // Un titre choisi par l'utilisateur n'est plus jamais remplacé.
    post(
        &mut app,
        "/api/conversations/update",
        json!({"id": conversation, "title": "Mon titre"}),
    );

    // Le deuxième message emporte l'historique.
    post(&mut app, "/api/preferences", json!({"fast": false}));
    post(
        &mut app,
        "/api/send",
        json!({"conversation_id": conversation, "message_id": "m2", "body": "Encore"}),
    );
    wait("deuxième réponse", || {
        assistant(&mut app)["id"] == "reply-m2" && assistant(&mut app)["state"] == "echoed"
    });
    assert_eq!(
        chat_bodies(&seen.lock().unwrap()).last().unwrap()["input"],
        json!([{"role": "user", "content": "Bonjour"}, {"role": "assistant", "content": "Bonjour **vous**"}, {"role": "user", "content": "Encore"}])
    );

    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["conversations"][0]["title"],
        "Mon titre"
    );
    let default_instructions = chat_bodies(&seen.lock().unwrap())[0]["instructions"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(default_instructions.contains("Tu t'appelles Grokbot."));
    assert!(!default_instructions.contains("Tu es Grokbot"));

    // Un bot personnalisé : ses instructions sont lues au moment de répondre, même après sa
    // modification puis sa suppression.
    post(
        &mut app,
        "/api/bots",
        json!({"id": "bot-chef", "name": "Chef", "color": "orange", "instructions": "Parle comme un cuisinier."}),
    );
    let cooking = post(
        &mut app,
        "/api/conversations",
        json!({"bot_id": "bot-chef"}),
    )["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    post(
        &mut app,
        "/api/bots/update",
        json!({"id": "bot-chef", "instructions": "Réponds en recettes."}),
    );
    post(&mut app, "/api/bots/delete", json!({"id": "bot-chef"}));
    post(
        &mut app,
        "/api/send",
        json!({"conversation_id": cooking, "message_id": "chef-1", "body": "Une idée ?"}),
    );
    wait("réponse du bot", || {
        assistant(&mut app)["id"] == "reply-chef-1" && assistant(&mut app)["state"] == "echoed"
    });
    let chef = chat_bodies(&seen.lock().unwrap())
        .into_iter()
        .find(|body| body["input"] == json!([{"role": "user", "content": "Une idée ?"}]))
        .unwrap();
    let instructions = chef["instructions"].as_str().unwrap();
    assert!(
        instructions.contains("Tu t'appelles Chef."),
        "{instructions}"
    );
    assert!(
        instructions.ends_with("Réponds en recettes."),
        "{instructions}"
    );
    wait("fin du titre du bot", || {
        provider.status()["titling"] == false
    });

    // Les jetons renouvelés survivent à un redémarrage.
    drop(app);
    let mut app = App::open_with(&db, provider.clone()).unwrap();
    assert_eq!(
        app.api("GET", "/api/state", b"").unwrap()["mode"],
        "chatgpt"
    );

    // La déconnexion révoque la session et rend le mode simulé.
    assert_eq!(
        post(&mut app, "/api/provider/signout", json!({}))["revoked"],
        true
    );
    assert!(seen.lock().unwrap().revoked);
    assert!(!std::fs::read_to_string(&credentials)
        .unwrap()
        .contains("refresh-2"));
    let simulated = post(
        &mut app,
        "/api/send",
        json!({"conversation_id": conversation, "message_id": "m3", "body": "Sans compte"}),
    );
    assert!(simulated["conversation"]["messages"][5]["body"]
        .as_str()
        .unwrap()
        .starts_with("Bien reçu"));
    let _ = std::fs::remove_dir_all(&directory);
}
