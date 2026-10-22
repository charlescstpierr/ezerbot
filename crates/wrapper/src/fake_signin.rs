//! Connexion factice, compilée seulement avec `debug_assertions`.
//! `EZERBOT_FAKE_SIGNIN=1` ouvre un faux OpenAI sur 127.0.0.1 et écrit une session
//! locale, sans OAuth ni compte réel. Un binaire de production n’inclut pas ce module.
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine;
use grokbot_wrapper::{Endpoints, Provider};
use serde_json::json;

pub(crate) fn enabled(value: Option<&str>) -> bool {
    value == Some("1")
}

pub(crate) fn connect(db: &Path) -> Option<Provider> {
    if !enabled(std::env::var("EZERBOT_FAKE_SIGNIN").ok().as_deref()) {
        return None;
    }
    let base = server();
    write_session(db);
    let endpoints = Endpoints {
        authorize: format!("{base}/authorize"),
        token: format!("{base}/token"),
        revoke: format!("{base}/revoke"),
        jwks: format!("{base}/jwks"),
        issuer: base.clone(),
        api: format!("{base}/v1"),
    };
    let codex = Endpoints {
        api: format!("{base}/backend-api/codex"),
        ..endpoints.clone()
    };
    Some(Provider::with_backends(db, endpoints, codex, false))
}

fn server() -> String {
    static BASE: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BASE.get_or_init(|| {
        let listener = tiny_http::Server::http("127.0.0.1:0")
            .expect("le faux OpenAI de debug doit écouter 127.0.0.1");
        let port = listener
            .server_addr()
            .to_ip()
            .expect("port de bouclage")
            .port();
        std::thread::spawn(move || serve(listener));
        format!("http://127.0.0.1:{port}")
    })
    .clone()
}

fn serve(listener: tiny_http::Server) {
    let stream = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"OK\"}\n\ndata: {\"type\":\"response.completed\"}\n\n";
    let models = json!({"models":[{
        "slug":"gpt-test","display_name":"GPT Test","description":"Modèle factice",
        "visibility":"list","default_reasoning_level":"medium",
        "supported_reasoning_levels":[
            {"effort":"low","description":"Léger"},
            {"effort":"medium","description":"Équilibré"}
        ],
        "service_tiers":[]
    }]});
    loop {
        let Ok(Some(mut request)) = listener.recv_timeout(std::time::Duration::from_secs(30))
        else {
            continue;
        };
        let mut body = Vec::new();
        let _ = std::io::Read::read_to_end(request.as_reader(), &mut body);
        let path = request.url().split('?').next().unwrap_or("");
        let (status, kind, payload) = if path.ends_with("/responses") {
            (200, "text/event-stream", stream.to_owned())
        } else if path.ends_with("/models") {
            (200, "application/json", models.to_string())
        } else if path.ends_with("/token") {
            (
                200,
                "application/json",
                json!({
                    "access_token": access_token(),
                    "refresh_token": "fake-refresh",
                    "id_token": access_token(),
                    "token_type": "Bearer",
                    "expires_in": 86_400,
                    "scope": "openid profile email offline_access",
                })
                .to_string(),
            )
        } else if path.ends_with("/revoke") || path.ends_with("/usage") {
            (200, "application/json", "{}".into())
        } else {
            (404, "application/json", "{}".into())
        };
        let response = tiny_http::Response::from_string(payload)
            .with_status_code(tiny_http::StatusCode(status))
            .with_header(
                tiny_http::Header::from_bytes("Content-Type", kind).expect("en-tête de type"),
            )
            .with_header(
                tiny_http::Header::from_bytes("Cache-Control", "no-store").expect("en-tête cache"),
            );
        let _ = request.respond(response);
    }
}

fn write_session(db: &Path) {
    let directory = db.parent().filter(|parent| !parent.as_os_str().is_empty());
    let directory = directory.unwrap_or(Path::new("."));
    let stem = db
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("grokbot");
    let path = directory.join(format!("{stem}.codex-auth.json"));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("dossier de la session factice");
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    let session = json!({
        "host_id": "urn:uuid:00000000-0000-4000-8000-0000000000e2",
        "account_id": "account-fake",
        "plan": "plus",
        "client_id": "app_EMoamEEZ73f0CkXaXp7hrann",
        "email": "vous@exemple.com",
        "access_token": access_token(),
        "refresh_token": "fake-refresh",
        "expires_at": now.saturating_add(86_400),
        "scopes": "openid profile email offline_access",
        "welcomed": true,
    });
    let temporary = path.with_extension("json.tmp");
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .expect("session factice temporaire");
    file.write_all(session.to_string().as_bytes())
        .expect("écriture de la session factice");
    file.sync_all().ok();
    fs::rename(temporary, &path).expect("installation de la session factice");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
}

fn access_token() -> String {
    fn part(bytes: &[u8]) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
    }
    let header = part(br#"{"alg":"none","typ":"JWT"}"#);
    let payload = part(
        br#"{"email":"vous@exemple.com","https://api.openai.com/auth":{"chatgpt_account_id":"account-fake","chatgpt_plan_type":"plus","chatgpt_data_residency":"eu"}}"#,
    );
    format!("{header}.{payload}.fake")
}

#[cfg(test)]
mod tests {
    use super::enabled;

    #[test]
    fn seul_le_chiffre_un_active_la_connexion_factice() {
        assert!(enabled(Some("1")));
        for value in [None, Some(""), Some("0"), Some("true"), Some("yes")] {
            assert!(!enabled(value), "{value:?}");
        }
    }
}
