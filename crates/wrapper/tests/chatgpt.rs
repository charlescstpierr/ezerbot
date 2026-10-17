//! Le parcours « Sign in with ChatGPT » contre un faux serveur OpenAI : enregistrement, échange du
//! code, vérification de l'identité, renouvellement, catalogue, réponse en flux et déconnexion.

mod support;

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use grokbot_wrapper::{App, Endpoints, Provider};
use serde_json::{json, Value};
use support::{form, wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};

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
    let server = FakeOpenAi::new(seen.clone());
    let base = server.base.clone();
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

    // The raw Responses seam preserves caller input and refreshes only once on a 401.
    let body = json!({"model": "gpt-test", "stream": true, "store": false,
        "input": [{"role": "user", "content": "Raw request"}], "tools": []});
    let unauthorized = || ScriptedResponse::json(401, json!({"error": {"code": "expired"}}));
    let forms_before = seen.lock().unwrap().forms.len();
    let bodies_before = seen.lock().unwrap().bodies.len();
    seen.lock().unwrap().responses.extend([
        unauthorized(),
        ScriptedResponse::events(&[json!({"type": "response.completed"})]),
    ]);
    let response = provider.responses(body.clone()).unwrap();
    assert_eq!(response.header("Content-Type"), Some("text/event-stream"));
    assert!(response
        .into_string()
        .unwrap()
        .contains("response.completed"));
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.forms.len(), forms_before + 1);
        assert_eq!(&seen.bodies[bodies_before..], &[body.clone(), body.clone()]);
        assert!(seen.bearer.iter().all(|bearer| bearer == "Bearer access-2"));
    }
    let forms_before = seen.lock().unwrap().forms.len();
    let bodies_before = seen.lock().unwrap().bodies.len();
    seen.lock()
        .unwrap()
        .responses
        .extend([unauthorized(), unauthorized()]);
    assert_eq!(provider.responses(body.clone()).unwrap_err().kind, "signin");
    assert_eq!(seen.lock().unwrap().forms.len(), forms_before + 1);
    assert_eq!(seen.lock().unwrap().bodies.len(), bodies_before + 2);

    // Non-authentication errors are not retried or silently stripped of caller options.
    seen.lock()
        .unwrap()
        .responses
        .push_back(ScriptedResponse::json(
            400,
            json!({"error": {"message": "unsupported service_tier", "param": "service_tier"}}),
        ));
    assert!(provider.responses(body.clone()).is_err());
    assert_eq!(seen.lock().unwrap().forms.len(), forms_before + 1);
    assert_eq!(seen.lock().unwrap().bodies.len(), bodies_before + 3);
    seen.lock()
        .unwrap()
        .responses
        .push_back(ScriptedResponse::json(200, json!({"id": "raw"})));
    let response: Value = provider
        .responses(json!({"model": "gpt-test", "stream": false}))
        .unwrap()
        .into_json()
        .unwrap();
    assert_eq!(response, json!({"id": "raw"}));

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
