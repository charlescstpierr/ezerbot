mod support;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use grokbot_engine::{
    contracts::{RunEventKind, ToolOutcome},
    model::RunStatus,
    store,
};
use grokbot_forms::requests;
use grokbot_wrapper::{App, Provider};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use support::{wait, FakeOpenAi, ScriptedResponse, Seen, CLIENT};
use tiny_http::{Header, Response, Server};

/// The public client of Ezerbot's Slack app, as a build or the judge configures it.
const SLACK_CLIENT: &str = "1234567890.1234567890";
const SLACK_ACCESS: &str = "xoxp-slack-access-secret-never-visible";
fn fields_of(url: &str) -> std::collections::HashMap<String, String> {
    url::Url::parse(&format!("http://fixture{url}"))
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect()
}

#[derive(Default)]
struct SeenOAuth {
    requests: Vec<(String, String)>,
    challenge: String,
    token_error: bool,
    tools_error: bool,
    revoke_error: bool,
    /// The fake server's only tool is named like a deletion.
    deleting_tool: bool,
    sse: bool,
    token_scope: Option<String>,
    github: GithubScript,
    gmail: GmailScript,
    slack: SlackScript,
}
/// Scripted Slack provider: the PKCE exchange, the Web API reads and the revocation.
#[derive(Default)]
struct SlackScript {
    /// What the authorization URL asked for, echoed back as granted.
    scope: String,
    /// The access token the provider still accepts.
    live: Option<String>,
    /// `ok`, or the error Slack answers to a revocation.
    revoke_error: bool,
    /// The documented token response names the team by id only.
    team_id_only: bool,
}
/// Scripted GitHub App provider: device flow, user, refresh and one REST route.
struct GithubScript {
    serial: u32,
    device_expires_in: u64,
    device_interval: u64,
    /// Replies to successive device polls before authorization; then pending.
    polls: std::collections::VecDeque<Value>,
    authorized: Option<String>,
    live_access: String,
    live_refresh: String,
    /// Every access token the provider still accepts: one per connected account.
    live_tokens: Vec<String>,
    /// The next sign-in is again the first user, as when Charles picks the same login.
    same_user_next: bool,
    renewals: u32,
    refresh: &'static str,
    issues: &'static str,
    /// `ok`, or `missing` for a repository the account cannot see.
    repo: &'static str,
    more: bool,
    long_readme: bool,
    /// `ok` confirms a revocation; `refused` is what GitHub says without the app secret.
    revoke: &'static str,
    token_times: Vec<std::time::Instant>,
    authorization: Vec<(String, String)>,
    /// Bodies of the comments the provider accepted, in order.
    comments: Vec<Value>,
    /// Bodies of the issues the provider accepted, in order.
    created: Vec<Value>,
    /// Status the provider answers to an issue creation; 201 creates it.
    create_status: u16,
}
impl Default for GithubScript {
    fn default() -> Self {
        Self {
            serial: 0,
            device_expires_in: 900,
            device_interval: 0,
            polls: Default::default(),
            authorized: None,
            live_access: String::new(),
            live_refresh: String::new(),
            live_tokens: Vec::new(),
            same_user_next: false,
            renewals: 0,
            refresh: "ok",
            issues: "ok",
            repo: "ok",
            more: false,
            long_readme: false,
            revoke: "ok",
            token_times: Vec::new(),
            authorization: Vec::new(),
            comments: Vec::new(),
            created: Vec::new(),
            create_status: 201,
        }
    }
}
/// Scripted Google provider for Gmail: the token endpoint, the profile and two REST routes.
#[derive(Default)]
struct GmailScript {
    /// Messages of the inbox, newest first: id, from, subject, date and text.
    inbox: Vec<Value>,
    serial: u32,
    renewals: u32,
    /// Every access token the provider still accepts.
    live_tokens: Vec<String>,
    /// Form fields of every token request, in order.
    token_posts: Vec<std::collections::HashMap<String, String>>,
    authorization: Vec<(String, String)>,
    /// Runs while the provider answers a listing: what happens to the app meanwhile.
    during_listing: Option<Box<dyn Fn() + Send>>,
}
impl GmailScript {
    /// One message as Gmail's `format=full` returns it: a text part and an HTML part.
    fn message(entry: &Value) -> Value {
        let encoded = |text: &str| URL_SAFE_NO_PAD.encode(text);
        // Messages are received in the order of their number: m7 after m6.
        let number: i64 = entry["id"].as_str().unwrap()[1..].parse().unwrap();
        let received = (1_790_000_000_000 + number * 1000).to_string();
        json!({"id":entry["id"],"threadId":entry["id"],"snippet":"Extrait court","internalDate":received,"payload":{"mimeType":"multipart/alternative","headers":[{"name":"From","value":entry["from"]},{"name":"Subject","value":entry["subject"]},{"name":"Date","value":entry["date"]}],"parts":[{"mimeType":"text/plain","body":{"data":encoded(entry["text"].as_str().unwrap())}},{"mimeType":"text/html","body":{"data":encoded("<p>Version HTML</p>")}}]}})
    }
}
struct OAuth {
    base: String,
    seen: Arc<Mutex<SeenOAuth>>,
    stop: Arc<AtomicBool>,
}
impl OAuth {
    fn new() -> Self {
        let server = Server::http("127.0.0.1:0").unwrap();
        let base = format!("http://{}", server.server_addr());
        let seen = Arc::new(Mutex::new(SeenOAuth::default()));
        let captured = seen.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                let Some(mut req) = server.recv_timeout(Duration::from_millis(50)).unwrap() else {
                    continue;
                };
                let url = req.url().to_owned();
                let post = *req.method() == tiny_http::Method::Post;
                let mut body = String::new();
                req.as_reader().read_to_string(&mut body).unwrap();
                let mut seen = captured.lock().unwrap();
                seen.requests.push((url.clone(), body.clone()));
                let fields: std::collections::HashMap<_, _> =
                    url::form_urlencoded::parse(body.as_bytes())
                        .into_owned()
                        .collect();
                let auth = req
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("Authorization"))
                    .map(|h| h.value.to_string());
                let mut extra: Vec<Header> = Vec::new();
                let (status, mut data) = if url.starts_with("/github/") {
                    let g = &mut seen.github;
                    let who = auth.clone();
                    if let Some(auth) = auth {
                        g.authorization.push((url.clone(), auth));
                    }
                    let is_live = |g: &GithubScript| {
                        g.authorization.last().is_some_and(|(_, a)| {
                            g.live_tokens.iter().any(|t| *a == format!("Bearer {t}"))
                        })
                    };
                    if url == "/github/device" {
                        g.serial += 1;
                        (
                            200,
                            json!({"device_code":format!("device-secret-{}-never-visible",g.serial),"user_code":format!("WDJB-MJH{}",g.serial),"verification_uri":"https://github.com/login/device","expires_in":g.device_expires_in,"interval":g.device_interval}),
                        )
                    } else if url == "/github/token" {
                        g.token_times.push(std::time::Instant::now());
                        match fields.get("grant_type").map(String::as_str) {
                            Some("urn:ietf:params:oauth:grant-type:device_code") => {
                                let code = fields.get("device_code").cloned().unwrap_or_default();
                                if let Some(reply) = g.polls.pop_front() {
                                    (200, reply)
                                } else if g.authorized.as_deref() == Some(code.as_str()) {
                                    let user = if g.same_user_next { 1 } else { g.serial };
                                    g.same_user_next = false;
                                    g.live_access = format!(
                                        "ghu_access-secret-never-visible-{user}-{}",
                                        g.serial
                                    );
                                    g.live_refresh = format!(
                                        "ghr_refresh-secret-never-visible-{user}-{}",
                                        g.serial
                                    );
                                    g.live_tokens.push(g.live_access.clone());
                                    (
                                        200,
                                        json!({"access_token":g.live_access,"refresh_token":g.live_refresh,"expires_in":28800,"refresh_token_expires_in":15897600,"token_type":"bearer","scope":""}),
                                    )
                                } else if code
                                    == format!("device-secret-{}-never-visible", g.serial)
                                {
                                    (200, json!({"error":"authorization_pending"}))
                                } else {
                                    (200, json!({"error":"expired_token"}))
                                }
                            }
                            Some("refresh_token") => match g.refresh {
                                "dead" => (
                                    200,
                                    json!({"error":"bad_refresh_token","error_description":"secret-must-not-leak"}),
                                ),
                                "down" => (503, json!({})),
                                "limited" => (429, json!({"message":"rate limit"})),
                                "forbidden" => (403, json!({"message":"API rate limit exceeded"})),
                                _ if fields.get("refresh_token") != Some(&g.live_refresh) => {
                                    (200, json!({"error":"bad_refresh_token"}))
                                }
                                _ => {
                                    g.renewals += 1;
                                    let old = g.live_access.clone();
                                    g.live_tokens.retain(|t| *t != old);
                                    g.live_access =
                                        format!("ghu_renewed-access-secret-{}", g.renewals);
                                    g.live_tokens.push(g.live_access.clone());
                                    g.live_refresh =
                                        format!("ghr_renewed-refresh-secret-{}", g.renewals);
                                    (
                                        200,
                                        json!({"access_token":g.live_access,"refresh_token":g.live_refresh,"expires_in":28800,"token_type":"bearer","scope":""}),
                                    )
                                }
                            },
                            _ => (400, json!({"error":"unsupported_grant_type"})),
                        }
                    } else if url == "/github/user" {
                        // The user a token belongs to is the number after its prefix.
                        let user = who.as_deref().and_then(|a| {
                            a.rsplit_once('-')
                                .and_then(|(rest, _)| rest.rsplit_once('-'))
                                .map(|(_, user)| user.to_owned())
                        });
                        match user.as_deref() {
                            Some("2") => (200, json!({"login":"autre-compte","id":4343})),
                            _ => (200, json!({"login":"charles-test","id":4242})),
                        }
                    } else if url == "/github/revoke" {
                        if g.revoke == "ok" {
                            (200, json!({"revoked":true}))
                        } else {
                            (404, json!({"message":"Not Found"}))
                        }
                    } else if post && url == "/github/api/repos/charles/essai/issues/12/comments" {
                        if is_live(g) {
                            let sent: Value = serde_json::from_str(&body).unwrap();
                            g.comments.push(sent.clone());
                            (
                                201,
                                json!({"id":777,"html_url":"https://github.com/charles/essai/issues/12#issuecomment-777","body":sent["body"],"user":{"login":"charles-test"}}),
                            )
                        } else {
                            (401, json!({"message":"Bad credentials"}))
                        }
                    } else if post && url == "/github/api/repos/charles/essai/issues" {
                        if !is_live(g) {
                            (401, json!({"message":"Bad credentials"}))
                        } else if g.create_status != 201 {
                            (g.create_status, json!({"message":"Validation Failed"}))
                        } else {
                            let sent: Value = serde_json::from_str(&body).unwrap();
                            g.created.push(sent.clone());
                            (
                                201,
                                json!({"number":31,"html_url":"https://github.com/charles/essai/issues/31","title":sent["title"],"body":sent["body"],"user":{"login":"charles-test"}}),
                            )
                        }
                    } else if url.starts_with("/github/api/repos/charles/essai/issues") {
                        let known = is_live(g);
                        if !known {
                            (401, json!({"message":"Bad credentials"}))
                        } else if g
                            .authorization
                            .last()
                            .is_some_and(|(_, a)| a.contains("-2-"))
                        {
                            // The second account sees other issues than the first.
                            (
                                200,
                                json!([{"number":21,"title":"Tâche du compte de travail","state":"open","user":{"login":"carole"}}]),
                            )
                        } else {
                            match g.issues {
                                "missing" => (404, json!({"message":"Not Found"})),
                                "forbidden" => (
                                    403,
                                    json!({"message":"Resource not accessible by integration"}),
                                ),
                                "limit" => {
                                    extra.push(
                                        Header::from_bytes("X-RateLimit-Remaining", "0").unwrap(),
                                    );
                                    extra.push(
                                        Header::from_bytes("X-RateLimit-Reset", "4102444800")
                                            .unwrap(),
                                    );
                                    (403, json!({"message":"API rate limit exceeded"}))
                                }
                                _ => {
                                    if g.more {
                                        extra.push(Header::from_bytes("Link", "<https://api.github.com/repositories/1/issues?page=2>; rel=\"next\"").unwrap());
                                    }
                                    (
                                        200,
                                        json!([
                                            {"number":12,"title":"Corriger le <b>bouton</b>","state":"open","user":{"login":"alice"}},
                                            {"number":13,"title":"Une demande de fusion","state":"open","user":{"login":"bob"},"pull_request":{"url":"x"}},
                                            {"number":9,"title":"Ancien problème","state":"closed","user":{"login":"charles-test"}}
                                        ]),
                                    )
                                }
                            }
                        }
                    } else if url.starts_with("/github/api/repos/charles/essai")
                        && !url.starts_with("/github/api/repos/charles/essai/issues")
                    {
                        let known = is_live(g);
                        let path = url
                            .trim_start_matches("/github/api/repos/charles/essai")
                            .split('?')
                            .next()
                            .unwrap()
                            .to_owned();
                        let file = |text: &str, name: &str| {
                            let encoded = base64::engine::general_purpose::STANDARD.encode(text);
                            let wrapped: Vec<&str> = encoded
                                .as_bytes()
                                .chunks(60)
                                .map(|c| std::str::from_utf8(c).unwrap())
                                .collect();
                            json!({"type":"file","name":name,"path":name,"encoding":"base64","size":text.len(),"content":wrapped.join("\n")+"\n"})
                        };
                        if !known {
                            (401, json!({"message":"Bad credentials"}))
                        } else if g.repo == "missing" {
                            (404, json!({"message":"Not Found"}))
                        } else {
                            match path.as_str() {
                                "" => (
                                    200,
                                    json!({"full_name":"charles/essai","description":"Dépôt d’essai d’Ezerbot","default_branch":"trunk"}),
                                ),
                                "/pulls/7" => (
                                    200,
                                    json!({"number":7,"title":"Ajouter le <i>guide</i>","state":"open","merged":false,"user":{"login":"alice"},"base":{"ref":"trunk"},"head":{"ref":"guide"}}),
                                ),
                                "/pulls/8" => (
                                    200,
                                    json!({"number":8,"title":"Corriger la faute","state":"closed","merged":true,"user":{"login":"bob"},"base":{"ref":"trunk"}}),
                                ),
                                "/pulls" => (
                                    200,
                                    json!([{"number":9,"title":"La plus récente","state":"open","user":{"login":"alice"},"base":{"ref":"trunk"}}]),
                                ),
                                "/readme" if g.long_readme => (
                                    200,
                                    file(
                                        &format!("# Essai\n{}", "ligne du fichier\n".repeat(2000)),
                                        "README.md",
                                    ),
                                ),
                                "/readme" => (
                                    200,
                                    file("# Essai\nBonjour depuis le README.\n", "README.md"),
                                ),
                                "/contents/docs/guide.md" => {
                                    (200, file("# Guide\nPremière ligne du guide.\n", "guide.md"))
                                }
                                "/contents/docs" => {
                                    (200, json!([{"type":"file","name":"guide.md"}]))
                                }
                                "/contents/logo.png" => (
                                    200,
                                    json!({"type":"file","name":"logo.png","encoding":"base64","size":4,"content":"/9j/4A=="}),
                                ),
                                _ => (404, json!({"message":"Not Found"})),
                            }
                        }
                    } else {
                        (404, json!({}))
                    }
                } else if url.starts_with("/gmail/") {
                    let challenge = seen.challenge.clone();
                    let g = &mut seen.gmail;
                    if let Some(auth) = auth {
                        g.authorization.push((url.clone(), auth));
                    }
                    let live = |g: &GmailScript| {
                        g.authorization.last().is_some_and(|(_, a)| {
                            g.live_tokens.iter().any(|t| *a == format!("Bearer {t}"))
                        })
                    };
                    if url == "/gmail/token" {
                        g.token_posts.push(fields.clone());
                        match fields.get("grant_type").map(String::as_str) {
                            Some("authorization_code") => {
                                let verifier = fields.get("code_verifier").expect("PKCE verifier");
                                assert_eq!(
                                    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
                                    challenge
                                );
                                g.serial += 1;
                                let access =
                                    format!("gmail-access-secret-never-visible-{}", g.serial);
                                g.live_tokens.push(access.clone());
                                (
                                    200,
                                    json!({"access_token":access,"refresh_token":"gmail-refresh-secret-never-visible","expires_in":3600,"token_type":"Bearer","scope":"https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.compose https://www.googleapis.com/auth/gmail.modify"}),
                                )
                            }
                            Some("refresh_token")
                                if fields.get("refresh_token").map(String::as_str)
                                    == Some("gmail-refresh-secret-never-visible") =>
                            {
                                g.renewals += 1;
                                let access = format!("gmail-renewed-secret-{}", g.renewals);
                                g.live_tokens.clear();
                                g.live_tokens.push(access.clone());
                                (
                                    200,
                                    json!({"access_token":access,"expires_in":3600,"token_type":"Bearer","scope":"https://www.googleapis.com/auth/gmail.readonly https://www.googleapis.com/auth/gmail.compose https://www.googleapis.com/auth/gmail.modify"}),
                                )
                            }
                            _ => (400, json!({"error":"invalid_grant"})),
                        }
                    } else if url == "/gmail/revoke" {
                        (200, json!({}))
                    } else if !live(g) {
                        (
                            401,
                            json!({"error":{"code":401,"message":"Invalid Credentials"}}),
                        )
                    } else if url == "/gmail/api/gmail/v1/users/me/profile" {
                        (
                            200,
                            json!({"emailAddress":"charles@example.test","messagesTotal":6}),
                        )
                    } else if let Some(listing) =
                        url.strip_prefix("/gmail/api/gmail/v1/users/me/messages?")
                    {
                        let query: std::collections::HashMap<_, _> =
                            url::form_urlencoded::parse(listing.as_bytes())
                                .into_owned()
                                .collect();
                        let wanted: usize = query["maxResults"].parse().unwrap();
                        assert_eq!(query["labelIds"], "INBOX");
                        if let Some(during) = &g.during_listing {
                            during();
                        }
                        let ids: Vec<Value> = g
                            .inbox
                            .iter()
                            .take(wanted)
                            .map(|m| json!({"id":m["id"],"threadId":m["id"]}))
                            .collect();
                        (200, json!({"messages":ids,"resultSizeEstimate":ids.len()}))
                    } else if let Some(one) = url
                        .strip_prefix("/gmail/api/gmail/v1/users/me/messages/")
                        .and_then(|rest| rest.strip_suffix("?format=full"))
                    {
                        match g.inbox.iter().find(|m| m["id"] == one) {
                            Some(entry) => (200, GmailScript::message(entry)),
                            None => (404, json!({"error":{"code":404}})),
                        }
                    } else {
                        (404, json!({}))
                    }
                } else if url.starts_with("/slack/") {
                    let challenge = seen.challenge.clone();
                    let slack = &mut seen.slack;
                    let path = url.split('?').next().unwrap().to_owned();
                    let live = slack
                        .live
                        .as_ref()
                        .is_some_and(|t| auth.as_deref() == Some(format!("Bearer {t}").as_str()));
                    if path == "/slack/token" {
                        if fields.get("grant_type").map(String::as_str)
                            == Some("authorization_code")
                        {
                            let verifier = fields.get("code_verifier").expect("PKCE verifier");
                            assert_eq!(
                                URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
                                challenge
                            );
                        }
                        slack.live = Some(SLACK_ACCESS.to_owned());
                        let team = if slack.team_id_only {
                            json!({"id":"T0FIXTURE"})
                        } else {
                            json!({"id":"T0FIXTURE","name":"Atelier Ezerbot"})
                        };
                        (
                            200,
                            json!({"ok":true,"app_id":"A0FIXTURE","team":team,"authed_user":{"id":"U0CHARLES","scope":slack.scope,"access_token":SLACK_ACCESS,"token_type":"user"}}),
                        )
                    } else if path == "/slack/revoke" {
                        if slack.revoke_error {
                            (200, json!({"ok":false,"error":"token_revoked"}))
                        } else {
                            slack.live = None;
                            (200, json!({"ok":true,"revoked":true}))
                        }
                    } else if !live {
                        (200, json!({"ok":false,"error":"invalid_auth"}))
                    } else if path == "/slack/api/auth.test" {
                        (
                            200,
                            json!({"ok":true,"team":"Atelier Ezerbot","team_id":"T0FIXTURE","user":"charles","user_id":"U0CHARLES"}),
                        )
                    } else if path == "/slack/api/conversations.list" {
                        (
                            200,
                            json!({"ok":true,"channels":[{"id":"C0GENERAL","name":"general","is_member":true},{"id":"C0PROJET","name":"projet-prive","is_private":true,"is_member":true},{"id":"C0DEHORS","name":"dehors","is_member":false}],"response_metadata":{"next_cursor":""}}),
                        )
                    } else if path == "/slack/api/conversations.history" {
                        match fields_of(&url).get("channel").map(String::as_str) {
                            Some("C0GENERAL") => (
                                200,
                                json!({"ok":true,"messages":[
                                    {"type":"message","user":"U0ALICE","ts":"1760000200.000100","text":"Le déploiement est terminé, merci <@U0CHARLES>."},
                                    {"type":"message","user":"U0CHARLES","ts":"1760000100.000200","text":"Bonjour à toute l’équipe"},
                                    {"type":"message","subtype":"channel_join","user":"U0BOB","ts":"1760000000.000300","text":"<@U0BOB> a rejoint le canal"}
                                ],"has_more":true}),
                            ),
                            Some("C0PROJET") => (
                                200,
                                json!({"ok":true,"messages":[{"type":"message","user":"U0ALICE","ts":"1760000300.000100","text":"Le budget est validé"}],"has_more":false}),
                            ),
                            Some("C0DEHORS") => (200, json!({"ok":false,"error":"not_in_channel"})),
                            Some("C0LIMITES1") => {
                                (200, json!({"ok":false,"error":"missing_scope"}))
                            }
                            _ => (200, json!({"ok":false,"error":"channel_not_found"})),
                        }
                    } else if path == "/slack/api/users.info" {
                        match fields_of(&url).get("user").map(String::as_str) {
                            Some("U0ALICE") => (
                                200,
                                json!({"ok":true,"user":{"id":"U0ALICE","name":"alice","real_name":"Alice Martin","profile":{"display_name":"alice"}}}),
                            ),
                            Some("U0CHARLES") => (
                                200,
                                json!({"ok":true,"user":{"id":"U0CHARLES","name":"charles","real_name":"Charles Test","profile":{"display_name":"charles"}}}),
                            ),
                            _ => (200, json!({"ok":false,"error":"user_not_found"})),
                        }
                    } else {
                        (404, json!({}))
                    }
                } else if url.ends_with("/register") {
                    (200, json!({"client_id":"fake-notion-client"}))
                } else if url.ends_with("/token") {
                    if seen.token_error {
                        (
                            400,
                            json!({"error":"invalid_grant","error_description":"secret-must-not-leak"}),
                        )
                    } else {
                        if fields
                            .get("grant_type")
                            .is_some_and(|s| s == "authorization_code")
                        {
                            let verifier = fields.get("code_verifier").expect("PKCE verifier");
                            assert_eq!(
                                URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes())),
                                seen.challenge
                            );
                        }
                        (
                            200,
                            json!({"access_token":"access-secret-never-visible-123456789","refresh_token":"refresh-secret-never-visible-987654321","token_type":"Bearer","expires_in":3600}),
                        )
                    }
                } else if url.ends_with("/userinfo") {
                    (200, json!({"email":"personne@example.test"}))
                } else if url.ends_with("/revoke") {
                    if seen.revoke_error {
                        (500, json!({"error":"revocation-failed-secret"}))
                    } else {
                        (200, json!({"ok":true,"revoked":true}))
                    }
                } else if url.ends_with("/mcp") {
                    let frame: Value = serde_json::from_str(&body).unwrap();
                    if seen.tools_error {
                        (401, json!({"error":"private-server-error"}))
                    } else if frame.get("id").is_none() {
                        (202, Value::Null)
                    } else {
                        let result = match frame["method"].as_str().unwrap() {
                            "initialize" => {
                                json!({"protocolVersion":"2025-06-18","capabilities":{"tools":{}},"serverInfo":{"name":"factice","version":"1"}})
                            }
                            "tools/list" => {
                                json!({"tools":[{"name":if seen.deleting_tool {"delete_note"} else {"echo"},"description":"Outil factice ; access-secret-never-visible-123456789","inputSchema":{"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}}]})
                            }
                            "tools/call" => {
                                json!({"content":[{"type":"text","text":format!("{} · access-secret-never-visible-123456789",frame["params"]["arguments"]["text"])}]})
                            }
                            _ => panic!("unexpected method"),
                        };
                        (
                            200,
                            json!({"jsonrpc":"2.0","id":frame["id"],"result":result}),
                        )
                    }
                } else {
                    (404, json!({}))
                };
                if url.ends_with("/token") && status == 200 {
                    if let Some(scope) = &seen.token_scope {
                        data["scope"] = json!(scope);
                    }
                }
                let sse = seen.sse && url.ends_with("/mcp") && status == 200;
                drop(seen);
                let text = if sse {
                    format!("event: message\ndata: {}\n\n", data)
                } else {
                    data.to_string()
                };
                let mut response = Response::from_string(text)
                    .with_status_code(status)
                    .with_header(
                        Header::from_bytes(
                            "Content-Type",
                            if sse {
                                "text/event-stream"
                            } else {
                                "application/json"
                            },
                        )
                        .unwrap(),
                    )
                    .with_header(Header::from_bytes("Mcp-Session-Id", "fake-session").unwrap());
                for header in extra {
                    response.add_header(header);
                }
                let _ = req.respond(response);
            }
        });
        Self { base, seen, stop }
    }
    fn count(&self, suffix: &str) -> usize {
        self.seen
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(p, _)| p.ends_with(suffix))
            .count()
    }
}
impl Drop for OAuth {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
static OAUTH_TEST_LOCK: Mutex<()> = Mutex::new(());
struct Fixture {
    app: App,
    path: PathBuf,
    provider: Provider,
    ai: FakeOpenAi,
    oauth: OAuth,
    _environment_guard: MutexGuard<'static, ()>,
}
impl Fixture {
    fn new() -> Self {
        let environment_guard = OAUTH_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir()
            .join(format!(
                "ezerbot-connectors-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ))
            .join("workspace.sqlite");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::write(path.with_extension("chatgpt-auth.json"),json!({"host_id":"test","client_id":CLIENT,"access_token":"fake-access","refresh_token":"fake-refresh","expires_at":now+3600,"scopes":"chatgpt.tokens.use.direct"}).to_string()).unwrap();
        let ai = FakeOpenAi::new(Arc::new(Mutex::new(Seen::default())));
        let provider = Provider::with_endpoints(&path, ai.endpoints(), false);
        let oauth = OAuth::new();
        std::env::set_var("GROKBOT_FAKE_CONNECTORS", &oauth.base);
        let app = App::open_with(&path, provider.clone()).unwrap();
        wait("models", || provider.status()["models_ready"] == true);
        Self {
            app,
            path,
            provider,
            ai,
            oauth,
            _environment_guard: environment_guard,
        }
    }
    fn db(&self) -> rusqlite::Connection {
        let c = rusqlite::Connection::open(&self.path).unwrap();
        c.busy_timeout(Duration::from_secs(4)).unwrap();
        c
    }
    fn post(&mut self, path: &str, body: Value) -> Value {
        self.app
            .api("POST", path, body.to_string().as_bytes())
            .unwrap()
    }
    fn state(&mut self) -> Value {
        self.app.api("GET", "/api/state", b"").unwrap()
    }
    fn item(&mut self, id: &str) -> Value {
        self.state()["mcp"]["catalogue"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == id)
            .unwrap()
            .clone()
    }
    fn connect(&mut self, id: &str) -> String {
        let body = if !matches!(id, "notion" | "github" | "gmail" | "slack")
            && self.item(id)["has_client"] != true
        {
            json!({"id":id,"client_id":"fixture-oauth-client","client_secret":"fixture-oauth-secret"})
        } else {
            json!({"id":id})
        };
        self.post("/api/connectors/connect", body)["run_id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn finish(&self, run: &str) {
        wait("terminal host action", || {
            store::required(&self.db(), run)
                .unwrap()
                .status
                .is_terminal()
        });
    }
    fn wait_approval(&self, run: &str) {
        wait("approval", || {
            !requests::list_for_run(&self.db(), run).unwrap().is_empty()
        });
    }
    fn decide(&mut self, run: &str, decision: Value) {
        wait("approval", || {
            if !requests::list_for_run(&self.db(), run).unwrap().is_empty() {
                return true;
            }
            let status = store::required(&self.db(), run).unwrap().status;
            assert!(
                !status.is_terminal(),
                "run {run} ended before approval: {:?}",
                store::events(&self.db(), run, 0).unwrap()
            );
            false
        });
        let r = requests::list_for_run(&self.db(), run).unwrap().remove(0);
        self.post("/api/approvals/resolve",json!({"entry_id":r.entry_id,"conversation_id":r.route.target_conversation_id,"submission_id":format!("decide-{run}"),"decision":decision}));
        self.app.tick(0);
        self.finish(run);
    }
    fn callback(&mut self, id: &str, deny: bool, wrong_state: bool) -> u16 {
        let auth = self.item(id)["authorization_url"]
            .as_str()
            .unwrap()
            .to_owned();
        let auth = url::Url::parse(&auth).unwrap();
        let fields: std::collections::HashMap<_, _> = auth.query_pairs().into_owned().collect();
        assert_eq!(fields["code_challenge_method"], "S256");
        {
            let mut seen = self.oauth.seen.lock().unwrap();
            seen.challenge = fields["code_challenge"].clone();
            if id == "slack" {
                // What Slack is asked for is what the fake grants.
                seen.slack.scope = fields["scope"].replace(' ', ",");
            }
        }
        let mut callback = url::Url::parse(&fields["redirect_uri"]).unwrap();
        callback
            .query_pairs_mut()
            .append_pair(
                "state",
                if wrong_state {
                    "wrong-state"
                } else {
                    &fields["state"]
                },
            )
            .append_pair(
                if deny { "error" } else { "code" },
                if deny { "access_denied" } else { "fake-code" },
            );
        match ureq::get(callback.as_str()).call() {
            Ok(r) => r.status(),
            Err(ureq::Error::Status(code, _)) => code,
            Err(e) => panic!("callback transport {e}"),
        }
    }
    fn outcome(&self, run: &str) -> ToolOutcome {
        let event = store::events(&self.db(), run, 0)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == RunEventKind::ToolResult)
            .unwrap();
        serde_json::from_value(event.payload["outcome"].clone()).unwrap()
    }
    fn script(&self, events: Vec<Value>) {
        self.ai
            .seen
            .lock()
            .unwrap()
            .responses
            .push_back(ScriptedResponse::events(&events));
    }
    fn tool_run(&mut self, server: &Value, id: &str) -> String {
        let conversation = self.post("/api/conversations", json!({"title":"Connecteur factice"}))
            ["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let name = server["tools"][0]["namespace"].as_str().unwrap();
        self.script(vec![json!({"type":"response.output_item.done","item":{"type":"function_call","id":format!("fc-{id}"),"call_id":id,"name":name,"arguments":json!({"text":"bonjour"}).to_string()}}),json!({"type":"response.completed"})]);
        self.post(
            "/api/send",
            json!({"conversation_id":conversation,"message_id":id,"body":"Utilise le service"}),
        );
        self.db().query_row("SELECT r.run_id FROM engine_runs r JOIN wrapper_engine_runs w USING(run_id) WHERE json_extract(w.input_json,'$.entry_id')=?1",[id],|r|r.get::<_,String>(0)).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::env::remove_var("GROKBOT_FAKE_CONNECTORS");
        std::env::remove_var("EZERBOT_GITHUB_CLIENT_ID");
        std::env::remove_var("EZERBOT_GOOGLE_CLIENT_ID");
        std::env::remove_var("EZERBOT_SLACK_CLIENT_ID");
        let _ = std::fs::remove_dir_all(self.path.parent().unwrap());
    }
}

#[test]
fn google_server_adapter_keeps_only_an_access_snapshot_in_memory() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = std::process::Command::new("python3")
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/local/bin")
        .env("ACCESS_TOKEN", "fake-google-access-token")
        .env("GOOGLE_EMAIL", "factice@example.test")
        .env("GOOGLE_SCOPES", "openid email gmail.modify")
        .arg(root.join("tests/fixtures/google_bridge_check.py"))
        .arg(root.join("src/features/mcp/google_bridge.py"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        "memory-only credential adapter: ok"
    );
}

#[test]
fn changing_oauth_application_is_refused_without_losing_tokens_even_when_reconnect_is_denied() {
    let mut f = Fixture::new();
    let start = f.connect("calendar");
    f.decide(&start, json!("allow_once"));
    assert_eq!(f.callback("calendar", false, false), 200);
    let server = f.item("calendar")["server"].clone();
    f.post(
        "/api/mcp/select",
        json!({"id":server["id"],"bot_id":"grokbot","enabled":true}),
    );
    assert_eq!(f.item("calendar")["has_tokens"], true);
    let pending = f.connect("calendar");
    let key = format!("MCP_{}_ENV", server["id"].as_str().unwrap()).to_uppercase();
    let vault = grokbot_wrapper::features::forms::vault(&f.app.feature_context()).unwrap();
    let original = vault.get(&key).unwrap().unwrap();
    let revokes = f.oauth.count("/revoke");
    let token_calls = f.oauth.count("/token");
    for body in [
        json!({"id":"calendar","client_id":"other-oauth-application","client_secret":"another-application-secret"}),
        json!({"id":"calendar","client_id":"fixture-oauth-client","client_secret":"changed-secret"}),
        json!({"id":"calendar","client_secret":"changed-secret"}),
    ] {
        let error = f
            .app
            .api(
                "POST",
                "/api/connectors/connect",
                body.to_string().as_bytes(),
            )
            .unwrap_err();
        assert_eq!(error.status, 409);
        assert_eq!(
            error.message,
            "Déconnectez d’abord le service pour changer d’application OAuth."
        );
        assert_eq!(vault.get(&key).unwrap().unwrap(), original);
        assert_eq!(f.item("calendar")["status"], "connected");
    }
    f.decide(&pending, json!("deny"));
    let after = vault.get(&key).unwrap().unwrap();
    assert_eq!(after, original);
    let stored: Value = serde_json::from_str(&after).unwrap();
    assert!(stored["variables"]["ACCESS_TOKEN"].is_string());
    assert!(stored["variables"]["REFRESH_TOKEN"].is_string());
    assert_eq!(f.oauth.count("/revoke"), revokes);
    assert_eq!(f.oauth.count("/token"), token_calls);
    assert_eq!(
        f.item("calendar")["server"]["enabled_bots"],
        json!(["grokbot"])
    );
    assert_eq!(f.item("calendar")["status"], "connected");
    assert!(!f.state().to_string().contains("other-oauth-application"));
    for removed in ["ACCESS_TOKEN", "REFRESH_TOKEN"] {
        let mut single: Value = serde_json::from_str(&original).unwrap();
        single["variables"].as_object_mut().unwrap().remove(removed);
        vault
            .set(
                &key,
                grokbot_forms::vault::MCP_ENV_PURPOSE,
                &single.to_string(),
            )
            .unwrap();
        let before = vault.get(&key).unwrap().unwrap();
        let error = f
            .app
            .api(
                "POST",
                "/api/connectors/connect",
                br#"{"id":"calendar","client_id":"other-oauth-application"}"#,
            )
            .unwrap_err();
        assert_eq!(error.status, 409);
        assert_eq!(f.item("calendar")["has_tokens"], true);
        assert_eq!(vault.get(&key).unwrap().unwrap(), before);
    }
    vault
        .set(&key, grokbot_forms::vault::MCP_ENV_PURPOSE, &original)
        .unwrap();
    let tool = f.tool_run(&server, "still-connected-after-refusal");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&tool, json!("allow_once"));
    assert!(matches!(f.outcome(&tool), ToolOutcome::Done { .. }));
}

#[test]
fn catalogue_oauth_approvals_refresh_revocation_and_restart_are_end_to_end() {
    let mut f = Fixture::new();
    assert_eq!(f.state()["mcp"]["catalogue"].as_array().unwrap().len(), 7);
    assert_eq!(f.item("reunions")["name"], "Réunions");
    assert!(f
        .app
        .api("POST", "/api/connectors/connect", br#"{"id":"unknown"}"#)
        .is_err());
    let denied = f.connect("calendar");
    assert_eq!(f.oauth.count("/token"), 0);
    assert_eq!(f.item("calendar")["authorization_url"], Value::Null);
    f.decide(&denied, json!("deny"));
    assert_eq!(f.item("calendar")["has_client"], true);
    assert_eq!(f.oauth.count("/token"), 0);
    assert_eq!(f.item("calendar")["status"], "disconnected");
    let run = f.connect("calendar");
    f.decide(&run, json!("allow_once"));
    let state = f.state();
    let request = state["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["run_id"] == run)
        .unwrap();
    assert_eq!(request["connector"]["tool"], "Connecter Google Calendar");
    assert_eq!(request["connector"]["catalogue_id"], "calendar");
    assert!(request["configuration_result"]["output"]["message"]
        .as_str()
        .unwrap()
        .contains("navigateur"));
    assert_eq!(f.callback("calendar", false, true), 400);
    assert_eq!(f.oauth.count("/token"), 0);
    assert_eq!(f.callback("calendar", true, false), 400);
    assert_eq!(f.item("calendar")["status"], "reconnect");
    for id in ["calendar", "drive", "notion"] {
        f.oauth.seen.lock().unwrap().token_scope = if id == "notion" {
            Some(String::new())
        } else {
            None
        };
        let run = f.connect(id);
        f.decide(&run, json!("allow_once"));
        assert_eq!(f.item(id)["status"], "authorizing");
        assert_eq!(f.callback(id, false, false), 200);
        let item = f.item(id);
        assert_eq!(item["status"], "connected");
        assert!(item["server"]["enabled_bots"]
            .as_array()
            .unwrap()
            .is_empty());
        assert_eq!(item["server"]["tools"].as_array().unwrap().len(), 1);
        assert!(!item.to_string().contains("access-secret"));
    }
    f.oauth.seen.lock().unwrap().token_scope = None;
    let server = f.item("calendar")["server"].clone();
    f.post(
        "/api/mcp/select",
        json!({"id":server["id"],"bot_id":"grokbot","enabled":true}),
    );
    let calls = f.oauth.count("/mcp");
    let run = f.tool_run(&server, "tool-refused");
    f.decide(&run, json!("deny"));
    assert_eq!(calls, f.oauth.count("/mcp"));
    let run = f.tool_run(&server, "tool-allowed");
    f.script(vec![json!({"type":"response.completed"})]);
    f.decide(&run, json!({"always_allow":{"scope":"global"}}));
    assert!(matches!(f.outcome(&run), ToolOutcome::Done { .. }));
    assert!(!format!("{:?}", f.outcome(&run)).contains("access-secret"));
    // A standing permission still uses the normal gate, and renewal happens only
    // within that admitted call, never during state polling or app startup.
    f.db()
        .execute(
            "UPDATE wrapper_connector_catalogue SET expires_at=1 WHERE id='calendar'",
            [],
        )
        .unwrap();
    let before = f.oauth.count("/token");
    assert_eq!(f.item("calendar")["status"], "connected");
    assert_eq!(f.item("calendar")["can_refresh"], true);
    assert_eq!(f.item("calendar")["account"], "personne@example.test");
    assert!(f.item("calendar")["message"]
        .as_str()
        .unwrap()
        .contains("prochain appel autorisé"));
    assert_eq!(
        f.item("calendar")["server"]["enabled_bots"],
        json!(["grokbot"])
    );
    // Expiry without a refresh token, unlike normal expiry, requires a sign-in.
    let key = format!("MCP_{}_ENV", server["id"].as_str().unwrap()).to_uppercase();
    let vault = grokbot_wrapper::features::forms::vault(&f.app.feature_context()).unwrap();
    let original = vault.get(&key).unwrap().unwrap();
    let mut without_refresh: Value = serde_json::from_str(&original).unwrap();
    without_refresh["variables"]
        .as_object_mut()
        .unwrap()
        .remove("REFRESH_TOKEN");
    vault
        .set(
            &key,
            grokbot_forms::vault::MCP_ENV_PURPOSE,
            &without_refresh.to_string(),
        )
        .unwrap();
    assert_eq!(f.item("calendar")["status"], "reconnect");
    vault
        .set(&key, grokbot_forms::vault::MCP_ENV_PURPOSE, &original)
        .unwrap();
    assert_eq!(f.oauth.count("/token"), before);
    let run = f.tool_run(&server, "tool-standing-refresh");
    f.script(vec![json!({"type":"response.completed"})]);
    f.finish(&run);
    assert!(matches!(f.outcome(&run), ToolOutcome::Done { .. }));
    assert_eq!(f.oauth.count("/token"), before + 1);
    f.oauth.seen.lock().unwrap().tools_error = true;
    let run = f.tool_run(&server, "tool-expired-remote");
    f.script(vec![json!({"type":"response.completed"})]);
    f.finish(&run);
    assert!(matches!(f.outcome(&run), ToolOutcome::Failed { .. }));
    assert_eq!(f.item("calendar")["status"], "reconnect");
    f.oauth.seen.lock().unwrap().tools_error = false;
    let before = f.oauth.count("/token");
    let recovery = f.connect("calendar");
    f.decide(&recovery, json!("allow_once"));
    assert_eq!(f.item("calendar")["status"], "connected");
    assert_eq!(
        f.oauth.count("/token"),
        before + 1,
        "a rejected unexpired token must renew, not loop on the same token"
    );
    // State/configuration/catalogues persist, but no provider is contacted on restart.
    let pending = f.connect("notion");
    f.decide(&pending, json!("allow_once")); // valid token: starts without browser
    let provider = Provider::with_endpoints(&f.path, f.ai.endpoints(), false);
    let before = f.oauth.count("/mcp");
    f.app = App::open_with(&f.path, provider.clone()).unwrap();
    f.provider = provider;
    assert_eq!(f.item("drive")["status"], "saved");
    assert_eq!(f.oauth.count("/mcp"), before);
    f.oauth.seen.lock().unwrap().sse = true;
    let run = f.connect("drive");
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.item("drive")["status"], "connected");
    // Revocation requires its own explicit decision; denial keeps credentials.
    let before = f.oauth.count("/revoke");
    let run = f.post("/api/connectors/disconnect", json!({"id":"drive"}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&run, json!("deny"));
    assert_eq!(before, f.oauth.count("/revoke"));
    assert_eq!(f.item("drive")["status"], "connected");
    let run = f.post("/api/connectors/disconnect", json!({"id":"drive"}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.item("drive")["status"], "disconnected");
    assert!(f.item("drive")["server"].is_null());
    let removed_id: String = f
        .db()
        .query_row(
            "SELECT server_id FROM wrapper_connector_catalogue WHERE id='drive'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let remaining: Value = serde_json::from_str(
        &grokbot_wrapper::features::forms::vault(&f.app.feature_context())
            .unwrap()
            .get(&format!("MCP_{removed_id}_ENV").to_uppercase())
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(remaining["variables"].get("ACCESS_TOKEN").is_none());
    assert!(remaining["variables"].get("REFRESH_TOKEN").is_none());
    assert_eq!(f.item("drive")["has_client"], true);
    assert_eq!(f.item("drive")["account"], Value::Null);
    assert_eq!(before + 1, f.oauth.count("/revoke"));
    f.oauth.seen.lock().unwrap().revoke_error = true;
    let run = f.post("/api/connectors/disconnect", json!({"id":"notion"}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.item("notion")["warning"], true);
    assert!(f.item("notion")["message"]
        .as_str()
        .unwrap()
        .contains("non confirmée"));
    // Bad token response is not logged, reflected or persisted.
    f.db()
        .execute(
            "UPDATE wrapper_connector_catalogue SET expires_at=1 WHERE id='calendar'",
            [],
        )
        .unwrap();
    f.oauth.seen.lock().unwrap().token_error = true;
    let run = f.connect("calendar");
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.callback("calendar", false, false), 400);
    assert_eq!(f.item("calendar")["status"], "reconnect");
    // A restart invalidates an unfinished, one-time callback without contacting
    // the service or reviving an old browser session.
    let before = f.oauth.count("/token");
    let pending = f.connect("calendar");
    f.decide(&pending, json!("allow_once"));
    assert_eq!(f.item("calendar")["status"], "authorizing");
    let provider = Provider::with_endpoints(&f.path, f.ai.endpoints(), false);
    f.app = App::open_with(&f.path, provider.clone()).unwrap();
    f.provider = provider;
    assert_eq!(f.item("calendar")["status"], "reconnect");
    assert!(f.item("calendar")["authorization_url"].is_null());
    assert_eq!(f.oauth.count("/token"), before + 1); // only the failed refresh above
                                                     // Disconnect also cancels pending authorization and removes its vault value.
    let pending = f.connect("calendar");
    f.decide(&pending, json!("allow_once"));
    let removed = f.post("/api/connectors/disconnect", json!({"id":"calendar"}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&removed, json!("allow_once"));
    assert_eq!(f.item("calendar")["status"], "disconnected");
    assert!(f.item("calendar")["authorization_url"].is_null());
    let public = f.state().to_string();
    for secret in [
        "access-secret",
        "refresh-secret",
        "secret-must-not-leak",
        "private-server-error",
    ] {
        assert!(!public.contains(secret));
        assert!(!std::fs::read(&f.path)
            .unwrap()
            .windows(secret.len())
            .any(|s| s == secret.as_bytes()));
    }
    let inputs = f.ai.seen.lock().unwrap().bodies.clone();
    assert!(!format!("{inputs:?}").contains("access-secret"));
    assert_eq!(
        store::required(&f.db(), &run).unwrap().status,
        RunStatus::Finished
    );
}

const GITHUB_TOOL: &str = "github_list_issues";
const GITHUB_PULL: &str = "github_get_pull_request";
const GITHUB_REPO: &str = "github_read_repository";
const GITHUB_SECRETS: [&str; 5] = [
    "ghu_access-secret",
    "ghr_refresh-secret",
    "ghu_renewed",
    "ghr_renewed",
    "device-secret-",
];

impl Fixture {
    /// The public client of the GitHub App is build configuration, never a form field.
    fn with_github_client(&self) {
        std::env::set_var("EZERBOT_GITHUB_CLIENT_ID", "Iv1.fixturepublicid");
    }
    fn github_start(&mut self) {
        self.with_github_client();
        let run = self.connect("github");
        self.decide(&run, json!("allow_once"));
    }
    fn github_device(&mut self) -> Value {
        self.item("github")["device"].clone()
    }
    fn github_authorize(&self) {
        let mut seen = self.oauth.seen.lock().unwrap();
        let code = format!("device-secret-{}-never-visible", seen.github.serial);
        seen.github.authorized = Some(code);
    }
    fn github_connect(&mut self) {
        self.github_start();
        self.github_authorize();
        wait("github connected", || {
            self.item("github")["status"] == "connected"
        });
        let server = self.item("github")["server"].clone();
        self.post(
            "/api/mcp/select",
            json!({"id":server["id"],"bot_id":"grokbot","enabled":true}),
        );
    }
    fn github_secret(&self) -> Option<Value> {
        let id: String = self
            .db()
            .query_row(
                "SELECT server_id FROM wrapper_connector_catalogue WHERE id='github'",
                [],
                |r| r.get(0),
            )
            .ok()?;
        grokbot_wrapper::features::forms::vault(&self.app.feature_context())
            .unwrap()
            .get(&format!("MCP_{id}_ENV").to_uppercase())
            .unwrap()
            .map(|raw| serde_json::from_str::<Value>(&raw).unwrap()["variables"].clone())
    }
    fn assert_no_github_token_kept(&self) {
        let kept = self.github_secret().unwrap_or_default();
        assert!(kept.get("ACCESS_TOKEN").is_none() && kept.get("REFRESH_TOKEN").is_none());
    }
    fn github_requests(&self, part: &str) -> usize {
        self.oauth
            .seen
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(url, _)| url.contains(part))
            .count()
    }
    /// One chat turn that asks the bot to list issues.
    fn github_list(&mut self, id: &str, repository: &str) -> String {
        self.github_call(id, GITHUB_TOOL, json!({"repository":repository}))
    }
    /// One chat turn that makes the model call a GitHub tool. Both model replies are
    /// queued first: nothing here waits for a human decision.
    fn github_call(&mut self, id: &str, tool: &str, arguments: Value) -> String {
        let conversation = self.github_conversation();
        self.github_call_in(&conversation, id, tool, arguments)
    }
    fn github_conversation(&mut self) -> String {
        self.post("/api/conversations", json!({"title":"GitHub"}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn github_call_in(
        &mut self,
        conversation: &str,
        id: &str,
        tool: &str,
        arguments: Value,
    ) -> String {
        self.script(vec![json!({"type":"response.output_item.done","item":{"type":"function_call","id":format!("fc-{id}"),"call_id":id,"name":tool,"arguments":arguments.to_string()}}),json!({"type":"response.completed"})]);
        self.script(vec![json!({"type":"response.completed"})]);
        self.post(
            "/api/send",
            json!({"conversation_id":conversation,"message_id":id,"body":format!("Lis {arguments}.")}),
        );
        let run: String = self.db().query_row("SELECT r.run_id FROM engine_runs r JOIN wrapper_engine_runs w USING(run_id) WHERE json_extract(w.input_json,'$.entry_id')=?1",[id],|r|r.get(0)).unwrap();
        wait("github run", || {
            self.app.tick(0);
            store::required(&self.db(), &run)
                .unwrap()
                .status
                .is_terminal()
        });
        assert!(
            requests::list_for_run(&self.db(), &run).unwrap().is_empty(),
            "a read must not open an approval card"
        );
        run
    }
    fn expire_github(&self) {
        self.db()
            .execute(
                "UPDATE wrapper_connector_catalogue SET expires_at=1 WHERE id='github'",
                [],
            )
            .unwrap();
    }
}

#[test]
fn github_card_has_no_client_fields_and_explains_a_missing_application() {
    let mut f = Fixture::new();
    let github = f.item("github");
    assert_eq!(github["needs_client"], false);
    assert_eq!(github["configured"], false);
    assert_eq!(github["redirect_uri"], Value::Null);
    for other in ["calendar", "drive"] {
        assert_eq!(f.item(other)["needs_client"], true);
        assert_eq!(f.item(other)["configured"], true);
    }
    assert_eq!(f.item("notion")["needs_client"], false);
    let before = f.state()["forms"]["requests"].as_array().unwrap().len();
    for body in [
        br#"{"id":"github"}"#.as_slice(),
        br#"{"id":"github","client_id":"mine","client_secret":"fixture-oauth-secret"}"#,
    ] {
        let error = f
            .app
            .api("POST", "/api/connectors/connect", body)
            .unwrap_err();
        assert_eq!(error.status, 400);
        assert!(
            error.message.contains("n’est pas configurée"),
            "{}",
            error.message
        );
        assert!(!error.message.contains("fixture-oauth-secret"));
    }
    assert_eq!(
        f.state()["forms"]["requests"].as_array().unwrap().len(),
        before
    );
    assert_eq!(f.github_requests("/device"), 0);
    assert_eq!(f.item("github")["status"], "disconnected");
    // Even with the application configured, a typed secret is refused, not stored.
    f.with_github_client();
    let error = f
        .app
        .api(
            "POST",
            "/api/connectors/connect",
            br#"{"id":"github","client_id":"mine","client_secret":"fixture-oauth-secret"}"#,
        )
        .unwrap_err();
    assert_eq!(error.status, 400);
    assert!(f.github_secret().is_none());
    assert_eq!(f.item("github")["configured"], true);
}

#[test]
fn github_device_flow_shows_a_copyable_code_paces_polling_and_never_sends_a_secret() {
    let mut f = Fixture::new();
    f.oauth.seen.lock().unwrap().github.polls.extend([
        json!({"error":"authorization_pending"}),
        json!({"error":"slow_down","interval":1}),
    ]);
    f.with_github_client();
    let run = f.connect("github");
    // The approval names the permissions before any browser or code appears.
    f.wait_approval(&run);
    let request = f.state()["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["run_id"] == run)
        .unwrap()
        .clone();
    assert_eq!(request["connector"]["tool"], "Connecter GitHub");
    assert_eq!(request["connector"]["catalogue_id"], "github");
    let catalogue = &request["arguments"]["catalogue"];
    assert_eq!(catalogue["service"], "GitHub");
    assert_eq!(catalogue["action"], "Connecter GitHub");
    let permissions = catalogue["permissions"].as_str().unwrap();
    for word in [
        "Issues",
        "pull requests",
        "contenu des dépôts en lecture",
        "dépôts que vous choisirez chez GitHub",
    ] {
        assert!(permissions.contains(word), "{word}: {permissions}");
    }
    assert!(catalogue["connexion"]
        .as_str()
        .unwrap()
        .contains("github.com/login/device"));
    assert_eq!(
        f.github_requests("/device"),
        0,
        "no code before the approval"
    );
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.github_requests("/device"), 1);
    let item = f.item("github");
    assert_eq!(item["status"], "authorizing");
    assert_eq!(item["device"]["user_code"], "WDJB-MJH1");
    assert_eq!(
        item["device"]["verification_uri"],
        "https://github.com/login/device"
    );
    assert!(item["device"]["expires_at"].as_u64().unwrap() > 1_000_000_000_000);
    assert_eq!(item["authorization_url"], "https://github.com/login/device");
    assert!(f.github_secret().is_none());
    wait("three polls", || f.github_requests("/github/token") >= 2);
    f.github_authorize();
    wait("connected", || f.item("github")["status"] == "connected");
    let item = f.item("github");
    assert_eq!(item["account"], "charles-test");
    assert_eq!(item["device"], Value::Null);
    assert_eq!(item["has_tokens"], true);
    {
        let seen = f.oauth.seen.lock().unwrap();
        let times = &seen.github.token_times;
        assert!(times.len() >= 3);
        assert!(
            times[2].duration_since(times[1]) >= Duration::from_millis(950),
            "slow_down must stretch the polling interval"
        );
        assert_eq!(seen.requests[0].1, "client_id=Iv1.fixturepublicid");
        for (_, body) in &seen.requests {
            assert!(!body.contains("client_secret"), "{body}");
        }
        let poll = seen
            .requests
            .iter()
            .find(|(url, _)| url == "/github/token")
            .unwrap();
        assert!(poll
            .1
            .contains("grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code"));
        assert!(poll.1.contains("client_id=Iv1.fixturepublicid"));
    }
    let kept = f.github_secret().unwrap();
    assert!(kept["ACCESS_TOKEN"].as_str().unwrap().starts_with("ghu_"));
    assert!(kept["REFRESH_TOKEN"].as_str().unwrap().starts_with("ghr_"));
    assert_eq!(f.github_secret().unwrap().get("CLIENT_SECRET"), None);
    let public = f.state().to_string();
    let database = std::fs::read(&f.path).unwrap();
    for secret in GITHUB_SECRETS {
        assert!(!public.contains(secret));
        assert!(!database
            .windows(secret.len())
            .any(|w| w == secret.as_bytes()));
    }
    // The other services keep their client: Google Calendar still asks for one.
    assert_eq!(f.item("calendar")["needs_client"], true);
    assert_eq!(f.item("calendar")["has_client"], false);
}

#[test]
fn github_wait_ends_without_a_token_on_cancel_refusal_expiry_and_restart() {
    let mut f = Fixture::new();
    // Cancel while the code is shown.
    f.github_start();
    let cancelled = f.github_device().clone();
    assert_eq!(f.item("github")["status"], "authorizing");
    let result = f.post("/api/connectors/cancel", json!({"id":"github"}));
    assert!(result["message"].as_str().unwrap().contains("annulée"));
    assert_eq!(f.item("github")["status"], "disconnected");
    assert_eq!(f.item("github")["device"], Value::Null);
    f.github_authorize();
    std::thread::sleep(Duration::from_millis(400));
    let polls = f.github_requests("/github/token");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(
        polls,
        f.github_requests("/github/token"),
        "the dead code is no longer polled"
    );
    f.assert_no_github_token_kept();
    assert_eq!(f.item("github")["status"], "disconnected");
    // Cancelling something that is not waiting changes nothing.
    assert!(f
        .app
        .api("POST", "/api/connectors/cancel", br#"{"id":"gmail"}"#)
        .is_err());
    // Refusal at GitHub.
    f.oauth.seen.lock().unwrap().github.authorized = None;
    f.oauth
        .seen
        .lock()
        .unwrap()
        .github
        .polls
        .push_back(json!({"error":"access_denied"}));
    f.github_start();
    wait("refusal", || f.item("github")["status"] == "reconnect");
    assert!(f.item("github")["message"]
        .as_str()
        .unwrap()
        .contains("refusé"));
    f.assert_no_github_token_kept();
    // Expiry, then a fresh code succeeds while the old one stays dead.
    f.oauth.seen.lock().unwrap().github.device_expires_in = 1;
    f.github_start();
    let expiring = f.github_device();
    wait("expiry", || f.item("github")["status"] == "reconnect");
    assert!(f.item("github")["message"]
        .as_str()
        .unwrap()
        .contains("expiré"));
    f.assert_no_github_token_kept();
    let polls = f.github_requests("/github/token");
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(polls, f.github_requests("/github/token"));
    f.oauth.seen.lock().unwrap().github.device_expires_in = 900;
    f.github_start();
    let fresh = f.github_device();
    assert_ne!(fresh["user_code"], expiring["user_code"]);
    assert_ne!(fresh["user_code"], cancelled["user_code"]);
    // Restart while waiting: nothing resumes, and authorizing the old code does nothing.
    let provider = Provider::with_endpoints(&f.path, f.ai.endpoints(), false);
    f.app = App::open_with(&f.path, provider.clone()).unwrap();
    f.provider = provider;
    assert_eq!(f.item("github")["status"], "reconnect");
    assert_eq!(f.item("github")["device"], Value::Null);
    assert!(f.item("github")["authorization_url"].is_null());
    f.github_authorize();
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(f.item("github")["status"], "reconnect");
    f.assert_no_github_token_kept();
    let requests = f.oauth.seen.lock().unwrap();
    assert!(requests
        .requests
        .iter()
        .all(|(_, b)| !b.contains("client_secret")));
}

#[test]
fn github_lists_one_page_of_issues_without_an_approval_card_and_names_the_account() {
    let mut f = Fixture::new();
    f.github_connect();
    f.oauth.seen.lock().unwrap().github.more = true;
    let before = f.github_requests("/issues");
    let run = f.github_list("list-issues", "charles/essai");
    let ToolOutcome::Done { output } = f.outcome(&run) else {
        panic!("{:?}", f.outcome(&run));
    };
    assert_eq!(output["compte"], "charles-test");
    assert_eq!(output["depot"], "charles/essai");
    assert_eq!(output["autres_issues"], true);
    let issues = output["issues"].as_array().unwrap();
    assert_eq!(issues.len(), 2, "pull requests are not issues");
    assert_eq!(issues[0]["numero"], 12);
    assert_eq!(issues[0]["titre"], "Corriger le <b>bouton</b>");
    assert_eq!(issues[0]["etat"], "open");
    assert_eq!(issues[0]["auteur"], "alice");
    let text = output["texte"].as_str().unwrap();
    assert!(text.contains("compte GitHub : charles-test"));
    assert!(text.contains("#12 · Corriger le <b>bouton</b> · ouverte · par alice"));
    assert!(text.contains("#9 · Ancien problème · fermée · par charles-test"));
    assert!(text.contains("première page"));
    assert_eq!(f.github_requests("/issues"), before + 1);
    {
        let seen = f.oauth.seen.lock().unwrap();
        let (url, auth) = seen
            .github
            .authorization
            .iter()
            .rev()
            .find(|(u, _)| u.contains("/issues"))
            .unwrap();
        assert!(url.contains("per_page=30"));
        assert_eq!(*auth, format!("Bearer {}", seen.github.live_access));
    }
    // The model saw the listing, never a token or a device code.
    let inputs = format!("{:?}", f.ai.seen.lock().unwrap().bodies);
    assert!(inputs.contains("charles-test") && inputs.contains("Ancien problème"));
    for secret in GITHUB_SECRETS {
        assert!(!inputs.contains(secret));
        assert!(!f.state().to_string().contains(secret));
    }
}

fn done(f: &Fixture, run: &str) -> Value {
    match f.outcome(run) {
        ToolOutcome::Done { output } => output,
        other => panic!("{other:?}"),
    }
}
fn failed(f: &Fixture, run: &str) -> String {
    match f.outcome(run) {
        ToolOutcome::Failed { reason } => reason,
        other => panic!("{other:?}"),
    }
}

#[test]
fn github_reads_a_pull_request_without_a_card_and_names_the_account() {
    let mut f = Fixture::new();
    f.github_connect();
    let before = f.github_requests("/pulls/7");
    let run = f.github_call(
        "pull-7",
        GITHUB_PULL,
        json!({"repository":"charles/essai","number":7}),
    );
    let output = done(&f, &run);
    assert_eq!(output["compte"], "charles-test");
    assert_eq!(output["depot"], "charles/essai");
    assert_eq!(output["numero"], 7);
    assert_eq!(output["titre"], "Ajouter le <i>guide</i>");
    assert_eq!(output["etat"], "open");
    assert_eq!(output["auteur"], "alice");
    assert_eq!(output["branche_base"], "trunk");
    let text = output["texte"].as_str().unwrap();
    for part in [
        "compte GitHub : charles-test",
        "#7",
        "Ajouter le <i>guide</i>",
        "État : ouverte",
        "Auteur : alice",
        "Branche de base : trunk",
    ] {
        assert!(text.contains(part), "{part}: {text}");
    }
    assert_eq!(f.github_requests("/pulls/7"), before + 1);
    {
        let seen = f.oauth.seen.lock().unwrap();
        let (_, auth) = seen
            .github
            .authorization
            .iter()
            .rev()
            .find(|(u, _)| u.contains("/pulls/7"))
            .unwrap();
        assert_eq!(*auth, format!("Bearer {}", seen.github.live_access));
    }
    // A merged pull request says so; without a number, the latest one is read.
    let run = f.github_call(
        "pull-8",
        GITHUB_PULL,
        json!({"repository":"charles/essai","number":8}),
    );
    let merged = done(&f, &run);
    assert_eq!(merged["etat"], "merged");
    assert!(merged["texte"]
        .as_str()
        .unwrap()
        .contains("État : fusionnée"));
    let run = f.github_call(
        "pull-latest",
        GITHUB_PULL,
        json!({"repository":"charles/essai"}),
    );
    let latest = done(&f, &run);
    assert_eq!(latest["numero"], 9);
    assert_eq!(latest["titre"], "La plus récente");
    let inputs = format!("{:?}", f.ai.seen.lock().unwrap().bodies);
    assert!(inputs.contains("charles-test") && inputs.contains("Branche de base : trunk"));
    for secret in GITHUB_SECRETS {
        assert!(!inputs.contains(secret));
        assert!(!f.state().to_string().contains(secret));
    }
}

#[test]
fn github_reads_a_repository_readme_or_a_named_file_and_truncates_a_long_one() {
    let mut f = Fixture::new();
    f.github_connect();
    let run = f.github_call("readme", GITHUB_REPO, json!({"repository":"charles/essai"}));
    let output = done(&f, &run);
    assert_eq!(output["compte"], "charles-test");
    assert_eq!(output["depot"], "charles/essai");
    assert_eq!(output["description"], "Dépôt d’essai d’Ezerbot");
    assert_eq!(output["branche_par_defaut"], "trunk");
    assert_eq!(output["fichier"], "README.md");
    assert_eq!(output["tronque"], false);
    assert_eq!(output["extrait"], "# Essai\nBonjour depuis le README.\n");
    let text = output["texte"].as_str().unwrap();
    for part in [
        "compte GitHub : charles-test",
        "Dépôt d’essai d’Ezerbot",
        "Branche par défaut : trunk",
        "README.md",
        "Bonjour depuis le README.",
    ] {
        assert!(text.contains(part), "{part}: {text}");
    }
    assert!(!text.contains("tronqué"));
    // A named file, in a folder.
    let run = f.github_call(
        "guide",
        GITHUB_REPO,
        json!({"repository":"charles/essai","path":"docs/guide.md"}),
    );
    let output = done(&f, &run);
    assert_eq!(output["fichier"], "docs/guide.md");
    assert!(output["extrait"]
        .as_str()
        .unwrap()
        .contains("Première ligne du guide."));
    assert_eq!(f.github_requests("/contents/docs/guide.md"), 1);
    // A long file is cut, and the bubble says so.
    f.oauth.seen.lock().unwrap().github.long_readme = true;
    let run = f.github_call("long", GITHUB_REPO, json!({"repository":"charles/essai"}));
    let output = done(&f, &run);
    assert_eq!(output["tronque"], true);
    let excerpt = output["extrait"].as_str().unwrap();
    assert!(excerpt.starts_with("# Essai\nligne du fichier"));
    assert!(
        excerpt.chars().count() <= 4000,
        "{}",
        excerpt.chars().count()
    );
    let text = output["texte"].as_str().unwrap();
    assert!(
        text.contains("Le fichier est trop long : seul le début est affiché"),
        "{text}"
    );
    assert!(text.chars().count() < 4600);
    let inputs = format!("{:?}", f.ai.seen.lock().unwrap().bodies);
    for secret in GITHUB_SECRETS {
        assert!(!inputs.contains(secret));
        assert!(!f.state().to_string().contains(secret));
    }
}

#[test]
fn github_pull_request_and_repository_tools_are_reads_without_approval() {
    let mut f = Fixture::new();
    f.github_connect();
    let tools = grokbot_wrapper::features::mcp::scoped_tools(&grokbot_wrapper::features::Context {
        connection: &f.db(),
        provider: &f.provider,
        db_path: &f.path,
        now_ms: 0,
        bot_id: Some("grokbot"),
        conversation_id: None,
        run_id: None,
    });
    for name in [GITHUB_TOOL, GITHUB_PULL, GITHUB_REPO] {
        let tool = tools.iter().find(|t| t.name == name).unwrap();
        assert!(!tool.requires_approval, "{name}");
    }
    assert_eq!(
        tools
            .iter()
            .filter(|t| t.name.starts_with("github_"))
            .count(),
        5,
        "the list of GitHub tools is closed: three reads, one comment and one issue on a draft card"
    );
    let writes: Vec<_> = tools
        .iter()
        .filter(|t| t.name.starts_with("github_") && t.requires_approval)
        .map(|t| t.name.as_str())
        .collect();
    assert_eq!(writes, [GITHUB_COMMENT_TOOL, GITHUB_ISSUE_TOOL]);
    // The closed list is also what the connection card reports.
    let listed = f.item("github")["server"]["tools"].to_string();
    for name in [
        "list_issues",
        "get_pull_request",
        "read_repository",
        "add_issue_comment",
        "create_issue",
    ] {
        assert!(listed.contains(name), "{name}: {listed}");
    }
}

#[test]
fn github_pull_request_and_file_failures_are_french_sentences_without_a_retry_loop() {
    let mut f = Fixture::new();
    f.github_connect();
    // A missing pull request: one call, one sentence.
    let before = f.github_requests("/pulls/404");
    let run = f.github_call(
        "pull-missing",
        GITHUB_PULL,
        json!({"repository":"charles/essai","number":404}),
    );
    let reason = failed(&f, &run);
    assert!(
        reason.contains("introuvable") && reason.contains("404"),
        "{reason}"
    );
    assert_eq!(f.github_requests("/pulls/404"), before + 1);
    // A missing file: the repository is read once, the file once.
    let repo_calls = f.github_requests("/repos/charles/essai");
    let before = f.github_requests("/contents/absent.md");
    let run = f.github_call(
        "file-missing",
        GITHUB_REPO,
        json!({"repository":"charles/essai","path":"absent.md"}),
    );
    let reason = failed(&f, &run);
    assert!(
        reason.contains("absent.md") && reason.contains("introuvable"),
        "{reason}"
    );
    assert_eq!(f.github_requests("/contents/absent.md"), before + 1);
    assert_eq!(f.github_requests("/repos/charles/essai"), repo_calls + 2);
    // A folder and a binary file are not excerpts.
    for (id, path, words) in [
        ("folder", "docs", "dossier"),
        ("binary", "logo.png", "texte"),
    ] {
        let run = f.github_call(
            id,
            GITHUB_REPO,
            json!({"repository":"charles/essai","path":path}),
        );
        let reason = failed(&f, &run);
        assert!(reason.contains(words), "{path}: {reason}");
    }
    // A missing repository is named once, and the file is never asked for.
    f.oauth.seen.lock().unwrap().github.repo = "missing";
    let repo_calls = f.github_requests("/repos/charles/essai");
    let files = f.github_requests("/contents");
    let run = f.github_call(
        "repo-missing",
        GITHUB_REPO,
        json!({"repository":"charles/essai"}),
    );
    let reason = failed(&f, &run);
    assert!(
        reason.contains("introuvable") && reason.contains("charles/essai"),
        "{reason}"
    );
    assert!(reason.contains("charles-test"), "{reason}");
    assert_eq!(f.github_requests("/repos/charles/essai"), repo_calls + 1);
    assert_eq!(f.github_requests("/contents"), files);
    let run = f.github_call(
        "pull-no-repo",
        GITHUB_PULL,
        json!({"repository":"charles/essai","number":7}),
    );
    assert!(failed(&f, &run).contains("introuvable"));
    f.oauth.seen.lock().unwrap().github.repo = "ok";
    // Bad arguments never reach GitHub.
    let calls = f.oauth.seen.lock().unwrap().requests.len();
    for (id, tool, arguments) in [
        (
            "bad-pull-name",
            GITHUB_PULL,
            json!({"repository":"../etc/passwd","number":1}),
        ),
        (
            "bad-pull-number",
            GITHUB_PULL,
            json!({"repository":"charles/essai","number":0}),
        ),
        (
            "bad-repo-name",
            GITHUB_REPO,
            json!({"repository":"charles"}),
        ),
        (
            "bad-path",
            GITHUB_REPO,
            json!({"repository":"charles/essai","path":"../secret"}),
        ),
        (
            "abs-path",
            GITHUB_REPO,
            json!({"repository":"charles/essai","path":"/etc/passwd"}),
        ),
        (
            "extra",
            GITHUB_REPO,
            json!({"repository":"charles/essai","ref":"main"}),
        ),
    ] {
        let run = f.github_call(id, tool, arguments);
        let reason = failed(&f, &run);
        assert!(!reason.contains("ghu_") && !reason.is_empty(), "{id}");
    }
    assert_eq!(f.oauth.seen.lock().unwrap().requests.len(), calls);
    // The issue listing of the first slice still works beside them.
    let run = f.github_list("still-lists", "charles/essai");
    assert_eq!(done(&f, &run)["issues"].as_array().unwrap().len(), 2);
}

#[test]
fn github_read_failures_are_french_sentences_and_the_provider_is_not_called_again() {
    let mut f = Fixture::new();
    // Zero account: the chat says to connect, and nothing is called.
    let tools = grokbot_wrapper::features::mcp::scoped_tools(&grokbot_wrapper::features::Context {
        connection: &f.db(),
        provider: &f.provider,
        db_path: &f.path,
        now_ms: 0,
        bot_id: Some("grokbot"),
        conversation_id: None,
        run_id: None,
    });
    assert!(tools.iter().all(|t| t.name != GITHUB_TOOL));
    f.github_connect();
    let tools = grokbot_wrapper::features::mcp::scoped_tools(&grokbot_wrapper::features::Context {
        connection: &f.db(),
        provider: &f.provider,
        db_path: &f.path,
        now_ms: 0,
        bot_id: Some("grokbot"),
        conversation_id: None,
        run_id: None,
    });
    let tool = tools.iter().find(|t| t.name == GITHUB_TOOL).unwrap();
    assert!(!tool.requires_approval);
    for (index, (script, words)) in [
        ("missing", "introuvable"),
        ("forbidden", "refus"),
        ("limit", "limite"),
    ]
    .into_iter()
    .enumerate()
    {
        f.oauth.seen.lock().unwrap().github.issues = script;
        let before = f.github_requests("/issues");
        let run = f.github_list(&format!("failure-{index}"), "charles/essai");
        let ToolOutcome::Failed { reason } = f.outcome(&run) else {
            panic!("{script}");
        };
        assert!(reason.to_lowercase().contains(words), "{script}: {reason}");
        assert_eq!(
            f.github_requests("/issues"),
            before + 1,
            "{script}: no retry loop"
        );
        assert!(!reason.contains("ghu_"));
    }
    f.oauth.seen.lock().unwrap().github.issues = "ok";
    let before = f.github_requests("/issues");
    let run = f.github_list("bad-name", "../etc/passwd");
    assert!(matches!(f.outcome(&run), ToolOutcome::Failed { .. }));
    assert_eq!(f.github_requests("/issues"), before);
    // Disconnected GitHub: a short sentence, no provider call.
    f.oauth.seen.lock().unwrap().github.revoke = "refused";
    let removal = f.post("/api/connectors/disconnect", json!({"id":"github"}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&removal, json!("allow_once"));
    assert_eq!(f.item("github")["status"], "disconnected");
    assert!(f.item("github")["message"]
        .as_str()
        .unwrap()
        .contains("non confirmée"));
    f.assert_no_github_token_kept();
}

#[test]
fn github_renews_an_expired_access_once_and_a_dead_refresh_asks_to_reconnect() {
    let mut f = Fixture::new();
    f.github_connect();
    let old_refresh = f.github_secret().unwrap()["REFRESH_TOKEN"].clone();
    f.expire_github();
    let tokens = f.github_requests("/github/token");
    let run = f.github_list("renewed", "charles/essai");
    assert!(matches!(f.outcome(&run), ToolOutcome::Done { .. }));
    assert_eq!(f.github_requests("/github/token"), tokens + 1);
    {
        let seen = f.oauth.seen.lock().unwrap();
        let refresh = seen
            .requests
            .iter()
            .rev()
            .find(|(u, b)| u == "/github/token" && b.contains("grant_type=refresh_token"))
            .unwrap();
        assert!(refresh.1.contains("client_id=Iv1.fixturepublicid"));
        assert!(!refresh.1.contains("client_secret"));
    }
    let kept = f.github_secret().unwrap();
    assert_ne!(
        kept["REFRESH_TOKEN"], old_refresh,
        "the old refresh token is replaced in the same write"
    );
    assert!(kept["ACCESS_TOKEN"]
        .as_str()
        .unwrap()
        .starts_with("ghu_renewed"));
    assert_eq!(f.item("github")["status"], "connected");
    // A later read does not renew again.
    let run = f.github_list("not-renewed-again", "charles/essai");
    assert!(matches!(f.outcome(&run), ToolOutcome::Done { .. }));
    assert_eq!(f.github_requests("/github/token"), tokens + 1);
    // Dead refresh: reconnect, tokens dropped, the old access is never replayed.
    f.expire_github();
    f.oauth.seen.lock().unwrap().github.refresh = "dead";
    let issues = f.github_requests("/issues");
    let run = f.github_list("dead-refresh", "charles/essai");
    let ToolOutcome::Failed { reason } = f.outcome(&run) else {
        panic!("a dead refresh must fail the read");
    };
    assert!(reason.contains("Reconnectez"), "{reason}");
    assert_eq!(f.github_requests("/issues"), issues);
    assert_eq!(f.item("github")["status"], "reconnect");
    assert_eq!(f.item("github")["has_tokens"], false);
    f.assert_no_github_token_kept();
    let tokens = f.github_requests("/github/token");
    let run = f.github_list("after-dead-refresh", "charles/essai");
    assert!(matches!(f.outcome(&run), ToolOutcome::Failed { .. }));
    assert_eq!(
        f.github_requests("/github/token"),
        tokens,
        "no further renewal attempt"
    );
    assert_eq!(f.github_requests("/issues"), issues);
    assert!(!f.state().to_string().contains("secret-must-not-leak"));
}

#[test]
fn github_keeps_its_tokens_when_the_renewal_is_only_unreachable() {
    let mut f = Fixture::new();
    f.github_connect();
    f.expire_github();
    for (index, script) in ["down", "limited", "forbidden"].into_iter().enumerate() {
        f.oauth.seen.lock().unwrap().github.refresh = script;
        let run = f.github_list(&format!("unreachable-{index}"), "charles/essai");
        assert!(
            matches!(f.outcome(&run), ToolOutcome::Failed { .. }),
            "{script}"
        );
        assert_eq!(f.item("github")["status"], "connected", "{script}");
        assert_eq!(f.item("github")["has_tokens"], true, "{script}");
        assert!(
            f.github_secret().unwrap()["REFRESH_TOKEN"].is_string(),
            "{script}"
        );
    }
}

impl Fixture {
    fn pending_request(&mut self, run: &str) -> Value {
        self.wait_approval(run);
        self.state()["forms"]["requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["run_id"] == run)
            .unwrap()
            .clone()
    }
    /// The status a decision is refused with, or none when it was accepted.
    fn refused_status(&mut self, request: &Value, decision: Value) -> Option<u16> {
        let body = json!({"entry_id":request["entry_id"],"conversation_id":request["route"]["target_conversation_id"],"submission_id":format!("try-{}",request["entry_id"]),"decision":decision});
        self.app
            .api(
                "POST",
                "/api/approvals/resolve",
                body.to_string().as_bytes(),
            )
            .err()
            .map(|e| e.status)
    }
    fn disconnect_github(&mut self) -> String {
        self.post("/api/connectors/disconnect", json!({"id":"github"}))["run_id"]
            .as_str()
            .unwrap()
            .to_owned()
    }
    fn approval_rules(&self) -> i64 {
        self.db()
            .query_row("SELECT count(*) FROM forms_approval_rules", [], |r| {
                r.get(0)
            })
            .unwrap()
    }
}

#[test]
fn github_disconnect_names_the_login_offers_no_always_and_a_refusal_keeps_the_account() {
    let mut f = Fixture::new();
    f.github_connect();
    let tokens = f.github_secret().unwrap();
    let revocations = f.github_requests("/github/revoke");
    let run = f.disconnect_github();
    let request = f.pending_request(&run);
    assert_eq!(request["connector"]["tool"], "Déconnecter GitHub");
    let catalogue = &request["arguments"]["catalogue"];
    assert_eq!(catalogue["compte"], "charles-test");
    assert!(catalogue["déconnexion"]
        .as_str()
        .unwrap()
        .contains("charles-test"));
    assert_eq!(
        request["policy"]["remember"], false,
        "the card must not offer « Toujours »"
    );
    for decision in [
        json!({"always_allow":{"scope":"global"}}),
        json!({"never":{"scope":"global"}}),
    ] {
        assert_eq!(f.refused_status(&request, decision), Some(400));
    }
    assert_eq!(f.approval_rules(), 0);
    // Nothing leaves before the click.
    assert_eq!(f.item("github")["status"], "connected");
    assert_eq!(f.github_requests("/github/revoke"), revocations);
    assert_eq!(f.github_secret().unwrap(), tokens);
    f.decide(&run, json!("deny"));
    assert_eq!(f.item("github")["status"], "connected");
    assert_eq!(f.item("github")["account"], "charles-test");
    assert_eq!(f.github_secret().unwrap(), tokens);
    assert_eq!(f.github_requests("/github/revoke"), revocations);
    let listed = f.github_list("after-refusal", "charles/essai");
    assert!(matches!(f.outcome(&listed), ToolOutcome::Done { .. }));
}

#[test]
fn github_disconnect_removes_the_local_secret_says_the_revocation_was_confirmed_and_asks_to_reconnect(
) {
    let mut f = Fixture::new();
    f.github_connect();
    let revocations = f.github_requests("/github/revoke");
    let run = f.disconnect_github();
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.github_requests("/github/revoke"), revocations + 1);
    assert_eq!(f.item("github")["status"], "disconnected");
    assert_eq!(f.item("github")["account"], Value::Null);
    assert_eq!(f.item("github")["warning"], false);
    let message = f.item("github")["message"].as_str().unwrap().to_owned();
    assert!(
        message.contains("GitHub a confirmé la révocation"),
        "{message}"
    );
    f.assert_no_github_token_kept();
    // The next read says to reconnect and calls nothing.
    let issues = f.github_requests("/issues");
    let run = f.github_list("after-disconnect", "charles/essai");
    let reason = failed(&f, &run);
    assert!(reason.contains("Reconnectez"), "{reason}");
    assert_eq!(f.github_requests("/issues"), issues);
    for secret in GITHUB_SECRETS {
        assert!(!f.state().to_string().contains(secret));
    }
}

#[test]
fn github_disconnect_removes_the_local_secret_even_when_github_does_not_confirm_the_revocation() {
    let mut f = Fixture::new();
    f.github_connect();
    f.oauth.seen.lock().unwrap().github.revoke = "refused";
    let revocations = f.github_requests("/github/revoke");
    let run = f.disconnect_github();
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.github_requests("/github/revoke"), revocations + 1);
    assert_eq!(f.item("github")["status"], "disconnected");
    assert_eq!(f.item("github")["warning"], true);
    let message = f.item("github")["message"].as_str().unwrap().to_owned();
    assert!(
        message.contains("révocation distante non confirmée"),
        "{message}"
    );
    assert!(!message.contains("a confirmé"), "{message}");
    f.assert_no_github_token_kept();
    assert_eq!(f.item("github")["has_tokens"], false);
    let run = f.github_list("after-unconfirmed", "charles/essai");
    assert!(failed(&f, &run).contains("Reconnectez"));
    // The revocation request carries no secret of the app.
    let seen = f.oauth.seen.lock().unwrap();
    let (_, body) = seen
        .requests
        .iter()
        .rev()
        .find(|(url, _)| url == "/github/revoke")
        .unwrap();
    assert!(!body.contains("client_secret"));
    assert!(seen
        .github
        .authorization
        .iter()
        .all(|(url, auth)| url != "/github/revoke" || !auth.starts_with("Basic")));
}

#[test]
fn a_tool_classed_as_a_deletion_never_runs_alone_and_no_always_rule_covers_it() {
    let mut f = Fixture::new();
    f.oauth.seen.lock().unwrap().deleting_tool = true;
    let run = f.connect("calendar");
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.callback("calendar", false, false), 200);
    let server = f.item("calendar")["server"].clone();
    assert_eq!(server["tools"][0]["name"], "delete_note");
    f.post(
        "/api/mcp/select",
        json!({"id":server["id"],"bot_id":"grokbot","enabled":true}),
    );
    let calls = f.oauth.count("/mcp");
    let run = f.tool_run(&server, "delete-refused");
    let request = f.pending_request(&run);
    assert_eq!(request["policy"]["remember"], false);
    assert_eq!(request["policy"]["protection"], "destructive");
    // The provider heard nothing while the card was open.
    assert_eq!(f.oauth.count("/mcp"), calls);
    for decision in [
        json!({"always_allow":{"scope":"global"}}),
        json!({"always_allow":{"scope":"subject"}}),
    ] {
        assert_eq!(f.refused_status(&request, decision), Some(400));
    }
    assert_eq!(f.approval_rules(), 0);
    f.decide(&run, json!("deny"));
    assert_eq!(f.oauth.count("/mcp"), calls);
    assert_eq!(f.approval_rules(), 0);
    // A second call still waits for its own click.
    let run = f.tool_run(&server, "delete-again");
    let again = f.pending_request(&run);
    assert_eq!(again["policy"]["remember"], false);
    assert_eq!(f.oauth.count("/mcp"), calls);
}

#[cfg(all(target_os = "macos", debug_assertions))]
#[test]
fn github_connection_fails_closed_when_the_keychain_refuses_and_writes_no_file_copy() {
    let mut f = Fixture::new();
    f.with_github_client();
    let missing = std::env::temp_dir().join("ezerbot-absent-keychain-for-github.keychain-db");
    std::env::set_var("GROKBOT_TEST_KEYCHAIN", &missing);
    grokbot_forms::vault::enable_os_vault(true);
    let run = f.connect("github");
    f.decide(&run, json!("allow_once"));
    f.github_authorize();
    wait("keychain refusal", || {
        f.item("github")["status"] == "reconnect"
    });
    let message = f.item("github")["message"].as_str().unwrap().to_owned();
    assert!(message.contains("trousseau"), "{message}");
    assert!(f.item("github")["has_tokens"] == false);
    let vault_file = f.path.with_extension("secrets.json");
    let file = std::fs::read(&vault_file).unwrap_or_default();
    let database = std::fs::read(&f.path).unwrap();
    for secret in GITHUB_SECRETS {
        assert!(!file.windows(secret.len()).any(|w| w == secret.as_bytes()));
        assert!(!database
            .windows(secret.len())
            .any(|w| w == secret.as_bytes()));
        assert!(!f.state().to_string().contains(secret));
    }
    std::env::remove_var("GROKBOT_TEST_KEYCHAIN");
    grokbot_forms::vault::enable_os_vault(false);
}

const GITHUB_COMMENT_TOOL: &str = "github_add_issue_comment";
const GITHUB_ISSUE_TOOL: &str = "github_create_issue";

impl Fixture {
    /// One chat turn in which the bot prepares a comment. The run stops on its card.
    fn github_prepare_comment(&mut self, id: &str, text: &str) -> (String, Value) {
        self.github_prepare_comment_as(id, text, None)
    }
    fn github_prepare_comment_as(
        &mut self,
        id: &str,
        text: &str,
        account: Option<&str>,
    ) -> (String, Value) {
        let mut arguments = json!({"repository":"charles/essai","issue":12,"text":text});
        if let Some(account) = account {
            arguments["compte"] = json!(account);
        }
        self.github_prepare(id, GITHUB_COMMENT_TOOL, arguments)
    }
    /// One chat turn in which the bot prepares an issue. The run stops on its card.
    fn github_prepare_issue(&mut self, id: &str, title: &str, body: &str) -> (String, Value) {
        self.github_prepare(
            id,
            GITHUB_ISSUE_TOOL,
            json!({"repository":"charles/essai","title":title,"body":body}),
        )
    }
    fn github_prepare(&mut self, id: &str, tool: &str, arguments: Value) -> (String, Value) {
        let conversation = self.post("/api/conversations", json!({"title":"Brouillon"}))
            ["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        self.script(vec![json!({"type":"response.output_item.done","item":{"type":"function_call","id":format!("fc-{id}"),"call_id":id,"name":tool,"arguments":arguments.to_string()}}),json!({"type":"response.completed"})]);
        self.script(vec![json!({"type":"response.completed"})]);
        self.post(
            "/api/send",
            json!({"conversation_id":conversation,"message_id":id,"body":"Prépare le brouillon."}),
        );
        let run: String = self.db().query_row("SELECT r.run_id FROM engine_runs r JOIN wrapper_engine_runs w USING(run_id) WHERE json_extract(w.input_json,'$.entry_id')=?1",[id],|r|r.get(0)).unwrap();
        wait("draft card", || {
            self.app.tick(0);
            !requests::list_for_run(&self.db(), &run).unwrap().is_empty()
        });
        let request = requests::list_for_run(&self.db(), &run).unwrap().remove(0);
        let card = self.state()["forms"]["requests"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["entry_id"] == request.entry_id)
            .unwrap()
            .clone();
        (run, card)
    }
    fn github_resolve(
        &mut self,
        card: &Value,
        submission: &str,
        decision: Value,
        text: Option<&str>,
    ) -> Result<Value, grokbot_wrapper::ApiError> {
        self.github_resolve_issue(card, submission, decision, None, text)
    }
    fn github_resolve_issue(
        &mut self,
        card: &Value,
        submission: &str,
        decision: Value,
        title: Option<&str>,
        text: Option<&str>,
    ) -> Result<Value, grokbot_wrapper::ApiError> {
        let mut body = json!({"entry_id":card["entry_id"],"conversation_id":card["route"]["target_conversation_id"],"submission_id":submission,"decision":decision});
        if let Some(text) = text {
            body["text"] = json!(text);
        }
        if let Some(title) = title {
            body["title"] = json!(title);
        }
        self.app.api(
            "POST",
            "/api/approvals/resolve",
            body.to_string().as_bytes(),
        )
    }
    fn github_settle(&mut self, run: &str) {
        wait("comment run", || {
            self.app.tick(0);
            store::required(&self.db(), run)
                .unwrap()
                .status
                .is_terminal()
        });
    }
    fn published_comments(&self) -> Vec<Value> {
        self.oauth.seen.lock().unwrap().github.comments.clone()
    }
    fn created_issues(&self) -> Vec<Value> {
        self.oauth.seen.lock().unwrap().github.created.clone()
    }
    /// Issues POSTed to the provider, whatever it answered.
    fn issue_creations(&self) -> usize {
        self.oauth
            .seen
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(url, body)| url.ends_with("/repos/charles/essai/issues") && !body.is_empty())
            .count()
    }
}

#[test]
fn github_comment_waits_on_a_draft_card_and_nothing_leaves_before_send() {
    let mut f = Fixture::new();
    f.github_connect();
    let tools = grokbot_wrapper::features::mcp::scoped_tools(&grokbot_wrapper::features::Context {
        connection: &f.db(),
        provider: &f.provider,
        db_path: &f.path,
        now_ms: 0,
        bot_id: Some("grokbot"),
        conversation_id: None,
        run_id: None,
    });
    let tool = tools
        .iter()
        .find(|t| t.name == GITHUB_COMMENT_TOOL)
        .expect("the comment tool is in the closed list");
    assert!(tool.requires_approval);

    let (run, card) = f.github_prepare_comment("draft-1", "Bonjour du brouillon.");
    let draft = &card["arguments"]["brouillon"];
    assert_eq!(draft["depot"], "charles/essai");
    assert_eq!(draft["numero"], 12);
    assert_eq!(draft["login"], "charles-test");
    assert_eq!(draft["texte"], "Bonjour du brouillon.");
    assert_eq!(card["status"]["status"], "pending");
    assert_eq!(card["policy"]["protection"], "outbound");
    assert_eq!(card["policy"]["remember"], false);
    assert!(
        f.published_comments().is_empty(),
        "nothing before the click"
    );
    assert_eq!(f.github_requests("/comments"), 0);

    // There is no “Toujours”: neither a standing allow nor a standing refusal.
    for (index, decision) in [
        json!({"always_allow":{"scope":"global"}}),
        json!({"never":{"scope":"global"}}),
    ]
    .into_iter()
    .enumerate()
    {
        let error = f
            .github_resolve(&card, &format!("always-{index}"), decision, None)
            .unwrap_err();
        assert_eq!(error.status, 400);
    }
    assert_eq!(f.github_requests("/comments"), 0);

    // Annuler publishes nothing.
    f.github_resolve(&card, "cancel-1", json!("deny"), None)
        .unwrap();
    f.github_settle(&run);
    assert!(matches!(f.outcome(&run), ToolOutcome::Failed { .. }));
    assert_eq!(f.github_requests("/comments"), 0);
    assert!(f.published_comments().is_empty());
}

#[test]
fn github_comment_publishes_the_corrected_text_once_under_the_displayed_account() {
    let mut f = Fixture::new();
    f.github_connect();
    let (run, card) = f.github_prepare_comment("draft-2", "Bonjour du brouillon.");
    assert_eq!(f.github_requests("/comments"), 0);
    let before = card["arguments"]["brouillon"]["empreinte"].clone();

    f.github_resolve(&card, "send-1", json!("allow_once"), Some("Bonjour validé"))
        .unwrap();
    f.github_settle(&run);
    let ToolOutcome::Done { output } = f.outcome(&run) else {
        panic!("{:?}", f.outcome(&run));
    };
    assert_eq!(output["compte"], "charles-test");
    assert_eq!(output["depot"], "charles/essai");
    assert_eq!(output["numero"], 12);
    assert_eq!(output["texte"], "Bonjour validé");
    assert!(output["url"].as_str().unwrap().contains("issuecomment-777"));
    assert_eq!(f.published_comments().len(), 1);
    assert_eq!(f.published_comments()[0]["body"], "Bonjour validé");
    {
        let seen = f.oauth.seen.lock().unwrap();
        let (_, auth) = seen
            .github
            .authorization
            .iter()
            .rev()
            .find(|(u, _)| u.ends_with("/comments"))
            .unwrap();
        assert_eq!(*auth, format!("Bearer {}", seen.github.live_access));
    }

    // The settled card shows the text that left, bound to a new fingerprint.
    let settled = f.state()["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["entry_id"] == card["entry_id"])
        .unwrap()
        .clone();
    assert_eq!(settled["arguments"]["brouillon"]["texte"], "Bonjour validé");
    assert_ne!(settled["arguments"]["brouillon"]["empreinte"], before);

    // A second Envoyer on the same card never calls GitHub again: the same click is
    // answered with its first receipt, another click is refused.
    f.github_resolve(&card, "send-1", json!("allow_once"), Some("Autre texte"))
        .unwrap();
    assert!(f
        .github_resolve(&card, "send-2", json!("allow_once"), Some("Autre texte"))
        .is_err());
    f.app.tick(0);
    assert_eq!(f.github_requests("/comments"), 1);
    assert_eq!(f.published_comments()[0]["body"], "Bonjour validé");

    // The model learned what was published, and no token reached it.
    let inputs = format!("{:?}", f.ai.seen.lock().unwrap().bodies);
    assert!(inputs.contains("Bonjour validé"));
    for secret in GITHUB_SECRETS {
        assert!(!inputs.contains(secret));
        assert!(!f.state().to_string().contains(secret));
    }

    // Reading issues still starts without any card.
    let listing = f.github_list("list-after-comment", "charles/essai");
    assert!(matches!(f.outcome(&listing), ToolOutcome::Done { .. }));
}

#[test]
fn github_comment_sends_the_proposed_text_when_it_is_not_edited_and_refuses_blank_edits() {
    let mut f = Fixture::new();
    f.github_connect();
    let (run, card) = f.github_prepare_comment("draft-3", "Texte proposé par le bot.");
    for (index, text) in ["", "   \n"].into_iter().enumerate() {
        let error = f
            .github_resolve(
                &card,
                &format!("blank-{index}"),
                json!("allow_once"),
                Some(text),
            )
            .unwrap_err();
        assert_eq!(error.status, 400, "{text:?}");
    }
    assert_eq!(f.github_requests("/comments"), 0);
    let still = f.state()["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["entry_id"] == card["entry_id"])
        .unwrap()
        .clone();
    assert_eq!(still["status"]["status"], "pending");

    f.github_resolve(&card, "send-3", json!("allow_once"), None)
        .unwrap();
    f.github_settle(&run);
    assert!(matches!(f.outcome(&run), ToolOutcome::Done { .. }));
    assert_eq!(f.published_comments().len(), 1);
    assert_eq!(
        f.published_comments()[0]["body"],
        "Texte proposé par le bot."
    );
}

#[test]
fn github_comment_refuses_a_bad_target_without_a_card() {
    let mut f = Fixture::new();
    // A bad target opens no card and calls nothing.
    let conversation = f.post("/api/conversations", json!({"title":"Sans compte"}))["conversation"]
        ["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.github_connect();
    let _ = conversation;
    let before = f.state()["forms"]["requests"].as_array().unwrap().len();
    let conversation = f.post("/api/conversations", json!({"title":"Cible"}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.script(vec![json!({"type":"response.output_item.done","item":{"type":"function_call","id":"fc-bad","call_id":"bad","name":GITHUB_COMMENT_TOOL,"arguments":json!({"repository":"../etc/passwd","issue":12,"text":"x"}).to_string()}}),json!({"type":"response.completed"})]);
    f.script(vec![json!({"type":"response.completed"})]);
    f.post(
        "/api/send",
        json!({"conversation_id":conversation,"message_id":"bad","body":"Commente."}),
    );
    let run: String = f.db().query_row("SELECT r.run_id FROM engine_runs r JOIN wrapper_engine_runs w USING(run_id) WHERE json_extract(w.input_json,'$.entry_id')='bad'",[],|r|r.get(0)).unwrap();
    f.github_settle(&run);
    let ToolOutcome::Failed { reason } = f.outcome(&run) else {
        panic!("{:?}", f.outcome(&run));
    };
    assert!(reason.contains("propriétaire/dépôt"), "{reason}");
    assert_eq!(
        f.state()["forms"]["requests"].as_array().unwrap().len(),
        before
    );
    assert_eq!(f.github_requests("/comments"), 0);
}

#[test]
fn github_comment_is_refused_when_the_account_changed_after_the_card() {
    let mut f = Fixture::new();
    f.github_connect();
    let (run, card) = f.github_prepare_comment("draft-4", "Texte du brouillon.");
    // A new sign-in replaced the account while the card was open.
    f.db()
        .execute(
            "UPDATE wrapper_connector_catalogue SET account_email='autre-compte',account_id='999' WHERE id='github'",
            [],
        )
        .unwrap();
    f.github_resolve(&card, "send-4", json!("allow_once"), None)
        .unwrap();
    f.github_settle(&run);
    let ToolOutcome::Failed { reason } = f.outcome(&run) else {
        panic!("{:?}", f.outcome(&run));
    };
    assert!(reason.contains("compte GitHub a changé"), "{reason}");
    assert_eq!(f.github_requests("/comments"), 0);
    assert!(f.published_comments().is_empty());
}

#[test]
fn github_draft_card_shows_only_the_result_of_its_own_call() {
    let mut f = Fixture::new();
    f.github_connect();
    let conversation = f.post("/api/conversations", json!({"title":"Deux outils"}))["conversation"]
        ["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let list = json!({"repository":"charles/essai"}).to_string();
    let draft =
        json!({"repository":"charles/essai","issue":12,"text":"Après la liste."}).to_string();
    f.script(vec![
        json!({"type":"response.output_item.done","item":{"type":"function_call","id":"fc-a","call_id":"list-a","name":GITHUB_TOOL,"arguments":list}}),
        json!({"type":"response.output_item.done","item":{"type":"function_call","id":"fc-b","call_id":"draft-b","name":GITHUB_COMMENT_TOOL,"arguments":draft}}),
        json!({"type":"response.completed"}),
    ]);
    f.script(vec![json!({"type":"response.completed"})]);
    f.post(
        "/api/send",
        json!({"conversation_id":conversation,"message_id":"two-tools","body":"Liste puis commente."}),
    );
    let run: String = f.db().query_row("SELECT r.run_id FROM engine_runs r JOIN wrapper_engine_runs w USING(run_id) WHERE json_extract(w.input_json,'$.entry_id')='two-tools'",[],|r|r.get(0)).unwrap();
    wait("draft card", || {
        f.app.tick(0);
        !requests::list_for_run(&f.db(), &run).unwrap().is_empty()
    });
    let entry = requests::list_for_run(&f.db(), &run)
        .unwrap()
        .remove(0)
        .entry_id;
    let card = f.state()["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["entry_id"] == entry)
        .unwrap()
        .clone();
    assert_eq!(card["status"]["status"], "pending");
    assert_eq!(
        card["publication_result"],
        Value::Null,
        "the listing's result is not a publication"
    );
    assert_eq!(f.github_requests("/comments"), 0);
}

#[test]
fn github_issue_waits_on_a_draft_card_without_always_and_cancel_creates_nothing() {
    let mut f = Fixture::new();
    f.github_connect();
    let (run, card) = f.github_prepare_issue("issue-1", "Essai Ezerbot", "créée depuis la carte");
    let draft = &card["arguments"]["brouillon"];
    assert_eq!(draft["action"], "Créer une issue");
    assert_eq!(draft["depot"], "charles/essai");
    assert_eq!(draft["titre"], "Essai Ezerbot");
    assert_eq!(draft["texte"], "créée depuis la carte");
    assert_eq!(draft["login"], "charles-test");
    assert_eq!(card["status"]["status"], "pending");
    assert_eq!(card["policy"]["protection"], "outbound");
    assert_eq!(card["policy"]["remember"], false);
    assert_eq!(f.issue_creations(), 0, "nothing before the click");

    // There is no “Toujours”: neither a standing allow nor a standing refusal.
    for (index, decision) in [
        json!({"always_allow":{"scope":"global"}}),
        json!({"never":{"scope":"global"}}),
    ]
    .into_iter()
    .enumerate()
    {
        let error = f
            .github_resolve(&card, &format!("issue-always-{index}"), decision, None)
            .unwrap_err();
        assert_eq!(error.status, 400);
    }
    assert_eq!(f.issue_creations(), 0);

    // Annuler creates nothing.
    f.github_resolve(&card, "issue-cancel", json!("deny"), None)
        .unwrap();
    f.github_settle(&run);
    assert!(matches!(f.outcome(&run), ToolOutcome::Failed { .. }));
    assert_eq!(f.issue_creations(), 0);
    assert!(f.created_issues().is_empty());
}

#[test]
fn github_issue_creates_one_issue_with_the_corrected_title_and_body_under_the_displayed_account() {
    let mut f = Fixture::new();
    f.github_connect();
    let (run, card) = f.github_prepare_issue("issue-2", "Essai Ezerbot", "créée depuis la carte");
    assert_eq!(f.issue_creations(), 0);
    let before = card["arguments"]["brouillon"]["empreinte"].clone();

    f.github_resolve_issue(
        &card,
        "issue-send-1",
        json!("allow_once"),
        Some("Essai validé"),
        Some("Corps validé"),
    )
    .unwrap();
    f.github_settle(&run);
    let ToolOutcome::Done { output } = f.outcome(&run) else {
        panic!("{:?}", f.outcome(&run));
    };
    assert_eq!(output["compte"], "charles-test");
    assert_eq!(output["depot"], "charles/essai");
    assert_eq!(output["numero"], 31);
    assert_eq!(output["titre"], "Essai validé");
    assert_eq!(output["texte"], "Corps validé");
    assert!(output["message"]
        .as_str()
        .unwrap()
        .contains("charles/essai#31"));
    assert!(output["url"].as_str().unwrap().ends_with("/issues/31"));
    assert_eq!(f.created_issues().len(), 1);
    assert_eq!(f.created_issues()[0]["title"], "Essai validé");
    assert_eq!(f.created_issues()[0]["body"], "Corps validé");
    {
        let seen = f.oauth.seen.lock().unwrap();
        let (_, auth) = seen
            .github
            .authorization
            .iter()
            .rev()
            .find(|(u, _)| u.ends_with("/repos/charles/essai/issues"))
            .unwrap();
        assert_eq!(*auth, format!("Bearer {}", seen.github.live_access));
    }

    // The settled card shows what left, bound to a new fingerprint, and the bubble's number.
    let settled = f.state()["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["entry_id"] == card["entry_id"])
        .unwrap()
        .clone();
    assert_eq!(settled["arguments"]["brouillon"]["titre"], "Essai validé");
    assert_eq!(settled["arguments"]["brouillon"]["texte"], "Corps validé");
    assert_ne!(settled["arguments"]["brouillon"]["empreinte"], before);
    assert_eq!(settled["publication_result"]["output"]["numero"], 31);

    // A second Envoyer on the same card never calls GitHub again: the same click is
    // answered with its first receipt, another click is refused.
    f.github_resolve_issue(
        &card,
        "issue-send-1",
        json!("allow_once"),
        Some("Autre titre"),
        Some("Autre corps"),
    )
    .unwrap();
    assert!(f
        .github_resolve_issue(
            &card,
            "issue-send-2",
            json!("allow_once"),
            Some("Autre titre"),
            Some("Autre corps"),
        )
        .is_err());
    f.app.tick(0);
    assert_eq!(f.issue_creations(), 1);
    assert_eq!(f.created_issues()[0]["title"], "Essai validé");

    // The model learned the number, and no token reached it.
    let inputs = format!("{:?}", f.ai.seen.lock().unwrap().bodies);
    assert!(inputs.contains("Essai validé") && inputs.contains("31"));
    for secret in GITHUB_SECRETS {
        assert!(!inputs.contains(secret));
        assert!(!f.state().to_string().contains(secret));
    }
}

#[test]
fn github_issue_sends_the_proposal_unedited_and_refuses_a_blank_title_or_a_comment_field() {
    let mut f = Fixture::new();
    f.github_connect();
    let (run, card) = f.github_prepare_issue("issue-3", "Titre proposé", "Corps proposé");
    for (index, title) in ["", "  ", "deux\nlignes"].into_iter().enumerate() {
        let error = f
            .github_resolve_issue(
                &card,
                &format!("issue-blank-{index}"),
                json!("allow_once"),
                Some(title),
                None,
            )
            .unwrap_err();
        assert_eq!(error.status, 400, "{title:?}");
    }
    // Deny never carries an edit.
    assert!(f
        .github_resolve_issue(&card, "issue-deny-edit", json!("deny"), Some("x"), None)
        .is_err());
    assert_eq!(f.issue_creations(), 0);
    let still = f.state()["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["entry_id"] == card["entry_id"])
        .unwrap()
        .clone();
    assert_eq!(still["status"]["status"], "pending");
    assert_eq!(still["arguments"]["brouillon"]["titre"], "Titre proposé");

    f.github_resolve(&card, "issue-send-3", json!("allow_once"), None)
        .unwrap();
    f.github_settle(&run);
    assert!(matches!(f.outcome(&run), ToolOutcome::Done { .. }));
    assert_eq!(f.created_issues().len(), 1);
    assert_eq!(f.created_issues()[0]["title"], "Titre proposé");
    assert_eq!(f.created_issues()[0]["body"], "Corps proposé");
}

#[test]
fn github_comment_card_refuses_a_title_edit_and_still_goes_through_the_same_card() {
    let mut f = Fixture::new();
    f.github_connect();
    let (run, card) = f.github_prepare_comment("comment-title", "Bonjour du brouillon.");
    assert!(f
        .github_resolve_issue(
            &card,
            "comment-title-1",
            json!("allow_once"),
            Some("Titre"),
            None
        )
        .is_err());
    assert_eq!(f.github_requests("/comments"), 0);
    f.github_resolve(
        &card,
        "comment-title-2",
        json!("allow_once"),
        Some("Bonjour validé"),
    )
    .unwrap();
    f.github_settle(&run);
    assert!(matches!(f.outcome(&run), ToolOutcome::Done { .. }));
    assert_eq!(f.published_comments().len(), 1);
    assert_eq!(f.issue_creations(), 0);
}

#[test]
fn github_issue_refuses_a_bad_target_without_a_card_and_reports_refusals_without_a_retry() {
    let mut f = Fixture::new();
    f.github_connect();
    let before = f.state()["forms"]["requests"].as_array().unwrap().len();
    let conversation = f.post("/api/conversations", json!({"title":"Cible"}))["conversation"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.script(vec![json!({"type":"response.output_item.done","item":{"type":"function_call","id":"fc-bad-issue","call_id":"bad-issue","name":GITHUB_ISSUE_TOOL,"arguments":json!({"repository":"../etc/passwd","title":"x"}).to_string()}}),json!({"type":"response.completed"})]);
    f.script(vec![json!({"type":"response.completed"})]);
    f.post(
        "/api/send",
        json!({"conversation_id":conversation,"message_id":"bad-issue","body":"Ouvre une issue."}),
    );
    let run: String = f.db().query_row("SELECT r.run_id FROM engine_runs r JOIN wrapper_engine_runs w USING(run_id) WHERE json_extract(w.input_json,'$.entry_id')='bad-issue'",[],|r|r.get(0)).unwrap();
    f.github_settle(&run);
    let ToolOutcome::Failed { reason } = f.outcome(&run) else {
        panic!("{:?}", f.outcome(&run));
    };
    assert!(reason.contains("propriétaire/dépôt"), "{reason}");
    assert_eq!(
        f.state()["forms"]["requests"].as_array().unwrap().len(),
        before
    );
    assert_eq!(f.issue_creations(), 0);

    // GitHub refuses the creation: a French sentence, one call, and the card is spent.
    f.oauth.seen.lock().unwrap().github.create_status = 422;
    let (run, card) = f.github_prepare_issue("issue-422", "Titre", "Corps");
    f.github_resolve(&card, "issue-422-send", json!("allow_once"), None)
        .unwrap();
    f.github_settle(&run);
    let ToolOutcome::Failed { reason } = f.outcome(&run) else {
        panic!("{:?}", f.outcome(&run));
    };
    assert!(reason.contains("Rien n’a été créé"), "{reason}");
    assert_eq!(f.issue_creations(), 1);
    f.app.tick(0);
    assert_eq!(f.issue_creations(), 1);

    // An outage may or may not have created it: it says so, and never calls again.
    f.oauth.seen.lock().unwrap().github.create_status = 503;
    let (run, card) = f.github_prepare_issue("issue-503", "Titre", "Corps");
    f.github_resolve(&card, "issue-503-send", json!("allow_once"), None)
        .unwrap();
    f.github_settle(&run);
    let ToolOutcome::Failed { reason } = f.outcome(&run) else {
        panic!("{:?}", f.outcome(&run));
    };
    assert!(reason.contains("peut-être été créée"), "{reason}");
    f.app.tick(0);
    assert_eq!(f.issue_creations(), 2);
}

#[test]
fn github_issue_is_refused_when_the_account_changed_after_the_card() {
    let mut f = Fixture::new();
    f.github_connect();
    let (run, card) = f.github_prepare_issue("issue-4", "Titre", "Corps");
    f.db()
        .execute(
            "UPDATE wrapper_connector_catalogue SET account_email='autre-compte',account_id='999' WHERE id='github'",
            [],
        )
        .unwrap();
    f.github_resolve(&card, "issue-send-4", json!("allow_once"), None)
        .unwrap();
    f.github_settle(&run);
    let ToolOutcome::Failed { reason } = f.outcome(&run) else {
        panic!("{:?}", f.outcome(&run));
    };
    assert!(reason.contains("compte GitHub a changé"), "{reason}");
    assert_eq!(f.issue_creations(), 0);
}

impl Fixture {
    fn github_accounts(&mut self) -> Vec<Value> {
        self.item("github")["accounts"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }
    /// A second sign-in beside the first, with its tools enabled for the default bot.
    fn github_add_account(&mut self) {
        let run = self.post(
            "/api/connectors/connect",
            json!({"id":"github","another":true}),
        )["run_id"]
            .as_str()
            .unwrap()
            .to_owned();
        self.decide(&run, json!("allow_once"));
        self.github_authorize();
        wait("second account", || {
            self.github_accounts()
                .iter()
                .filter(|a| a["status"] == "connected")
                .count()
                == 2
        });
        let second = self.github_accounts()[1].clone();
        self.post(
            "/api/mcp/select",
            json!({"id":second["server"]["id"],"bot_id":"grokbot","enabled":true}),
        );
    }
    fn vault_secret(&self, server: &Value) -> Option<Value> {
        grokbot_wrapper::features::forms::vault(&self.app.feature_context())
            .unwrap()
            .get(&format!("MCP_{}_ENV", server["id"].as_str().unwrap()).to_uppercase())
            .unwrap()
            .map(|raw| serde_json::from_str::<Value>(&raw).unwrap()["variables"].clone())
    }
    fn github_ask(&mut self, conversation: &str, id: &str, account: Option<&str>) -> Value {
        let mut arguments = json!({"repository":"charles/essai"});
        if let Some(account) = account {
            arguments["compte"] = json!(account);
        }
        let run = self.github_call_in(conversation, id, GITHUB_TOOL, arguments);
        done(self, &run)
    }
}

#[test]
fn github_second_account_is_added_beside_the_first_with_its_own_secret() {
    let mut f = Fixture::new();
    f.github_connect();
    let first = f.github_secret().unwrap();
    f.github_add_account();
    let accounts = f.github_accounts();
    assert_eq!(accounts.len(), 2);
    assert_eq!(accounts[0]["account"], "charles-test");
    assert_eq!(accounts[1]["account"], "autre-compte");
    assert!(accounts.iter().all(|a| a["status"] == "connected"));
    // The first account is untouched, and each login has a secret of its own.
    assert_eq!(f.github_secret().unwrap(), first);
    let second = f.vault_secret(&accounts[1]["server"]).unwrap();
    assert!(second["ACCESS_TOKEN"].as_str().unwrap().starts_with("ghu_"));
    assert_ne!(second["ACCESS_TOKEN"], first["ACCESS_TOKEN"]);
    assert_ne!(second["REFRESH_TOKEN"], first["REFRESH_TOKEN"]);
    assert_ne!(accounts[0]["server"]["id"], accounts[1]["server"]["id"]);
    // The card keeps following the first account; the list carries both.
    let item = f.item("github");
    assert_eq!(item["status"], "connected");
    assert_eq!(item["account"], "charles-test");
    // Two accounts share one list of tools.
    let tools = grokbot_wrapper::features::mcp::scoped_tools(&grokbot_wrapper::features::Context {
        connection: &f.db(),
        provider: &f.provider,
        db_path: &f.path,
        now_ms: 0,
        bot_id: Some("grokbot"),
        conversation_id: None,
        run_id: None,
    });
    assert_eq!(tools.iter().filter(|t| t.name == GITHUB_TOOL).count(), 1);
    // No token reaches the state or the database file.
    let public = f.state().to_string();
    let database = std::fs::read(&f.path).unwrap();
    for secret in GITHUB_SECRETS {
        assert!(!public.contains(secret));
        assert!(!database
            .windows(secret.len())
            .any(|w| w == secret.as_bytes()));
    }
}

#[test]
fn github_refused_second_account_stays_on_the_github_card() {
    let mut f = Fixture::new();
    f.github_connect();
    let run = f.post(
        "/api/connectors/connect",
        json!({"id":"github","another":true}),
    )["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&run, json!("deny"));
    let state = f.state();
    let item = f.item("github");
    let slots = item["slot_servers"].as_array().unwrap();
    // Every GitHub server, even the refused slot's, belongs to the card.
    let github: Vec<_> = state["mcp"]["servers"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["name"] == "GitHub")
        .collect();
    assert_eq!(github.len(), 2);
    assert!(github.iter().all(|s| slots.contains(&s["id"])));
    assert_eq!(f.github_accounts().len(), 1);
}

#[test]
fn github_sign_in_of_the_same_user_does_not_add_a_second_account() {
    let mut f = Fixture::new();
    f.github_connect();
    f.oauth.seen.lock().unwrap().github.same_user_next = true;
    let run = f.post(
        "/api/connectors/connect",
        json!({"id":"github","another":true}),
    )["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&run, json!("allow_once"));
    f.github_authorize();
    wait("refused twin", || {
        f.github_accounts()
            .iter()
            .any(|a| a["status"] == "reconnect")
    });
    let accounts = f.github_accounts();
    let twin = accounts
        .iter()
        .find(|a| a["status"] == "reconnect")
        .unwrap();
    assert!(twin["message"].as_str().unwrap().contains("déjà connecté"));
    assert_eq!(twin["has_tokens"], false);
    assert_eq!(accounts[0]["status"], "connected");
}

#[test]
fn github_asks_which_account_then_keeps_the_answer_for_the_conversation() {
    let mut f = Fixture::new();
    f.github_connect();
    f.github_add_account();
    let conversation = f.github_conversation();
    let before = f.github_requests("/issues");

    // Two accounts, no choice yet: a question, and GitHub is not called.
    let question = f.github_ask(&conversation, "ask-1", None);
    assert_eq!(question["choix_requis"], true);
    assert_eq!(question["comptes"], json!(["charles-test", "autre-compte"]));
    assert!(question["texte"].as_str().unwrap().contains("lequel"));
    assert_eq!(f.github_requests("/issues"), before);

    // Named: the list is that account's, and the bubble says which.
    let listing = f.github_ask(&conversation, "ask-2", Some("autre-compte"));
    assert_eq!(listing["compte"], "autre-compte");
    assert_eq!(listing["issues"][0]["titre"], "Tâche du compte de travail");
    assert!(listing["texte"]
        .as_str()
        .unwrap()
        .contains("compte GitHub : autre-compte"));
    assert_eq!(f.github_requests("/issues"), before + 1);

    // The choice holds for the rest of the conversation, with no new question.
    let again = f.github_ask(&conversation, "ask-3", None);
    assert_eq!(again["compte"], "autre-compte");
    assert_eq!(f.github_requests("/issues"), before + 2);

    // Another conversation starts without a choice.
    let other = f.github_conversation();
    assert_eq!(f.github_ask(&other, "ask-4", None)["choix_requis"], true);

    // Naming the first account switches the conversation to it.
    let first = f.github_ask(&conversation, "ask-5", Some("charles-test"));
    assert_eq!(first["compte"], "charles-test");
    assert_eq!(first["issues"][0]["titre"], "Corriger le <b>bouton</b>");
    assert_eq!(
        f.github_ask(&conversation, "ask-6", None)["compte"],
        "charles-test"
    );
    {
        let seen = f.oauth.seen.lock().unwrap();
        let second_token = seen.github.live_tokens[1].clone();
        let used: Vec<_> = seen
            .github
            .authorization
            .iter()
            .filter(|(u, _)| u.contains("/issues"))
            .map(|(_, a)| a.clone())
            .collect();
        assert_eq!(used.len(), 4);
        assert_eq!(used[0], format!("Bearer {second_token}"));
        assert_eq!(used[1], format!("Bearer {second_token}"));
        assert_ne!(used[2], format!("Bearer {second_token}"));
    }

    // A name that is not connected is refused with the accounts there are.
    let run = f.github_call_in(
        &conversation,
        "ask-7",
        GITHUB_TOOL,
        json!({"repository":"charles/essai","compte":"inconnu"}),
    );
    let reason = failed(&f, &run);
    assert!(
        reason.contains("inconnu") && reason.contains("autre-compte"),
        "{reason}"
    );
}

#[test]
fn github_with_one_account_lists_without_a_question() {
    let mut f = Fixture::new();
    f.github_connect();
    let conversation = f.github_conversation();
    let listing = f.github_ask(&conversation, "single-1", None);
    assert_eq!(listing["compte"], "charles-test");
    assert!(listing.get("choix_requis").is_none());
}

#[test]
fn github_disconnecting_the_second_account_removes_only_its_secret() {
    let mut f = Fixture::new();
    f.github_connect();
    let first = f.github_secret().unwrap();
    f.github_add_account();
    let accounts = f.github_accounts();
    let second_server = accounts[1]["server"].clone();
    let conversation = f.github_conversation();
    f.github_ask(&conversation, "gone-1", Some("autre-compte"));

    let run = f.post(
        "/api/connectors/disconnect",
        json!({"id":"github","account":accounts[1]["id"]}),
    )["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&run, json!("allow_once"));

    assert!(f.vault_secret(&second_server).is_none());
    assert_eq!(f.github_secret().unwrap(), first);
    let left = f.github_accounts();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0]["account"], "charles-test");
    assert_eq!(f.item("github")["status"], "connected");
    // The first account still lists, with no question, in the conversation that had
    // chosen the account that went away.
    let listing = f.github_ask(&conversation, "gone-2", None);
    assert_eq!(listing["compte"], "charles-test");
    assert!(listing.get("choix_requis").is_none());
    let state = f.state().to_string();
    for secret in GITHUB_SECRETS {
        assert!(!state.contains(secret));
    }
}

#[test]
fn github_comment_goes_out_under_the_chosen_account_and_asks_first_when_there_are_two() {
    let mut f = Fixture::new();
    f.github_connect();
    f.github_add_account();
    // No choice: no card opens, and the reason is the question.
    let conversation = f.github_conversation();
    let run = f.github_call_in(
        &conversation,
        "comment-ask",
        GITHUB_COMMENT_TOOL,
        json!({"repository":"charles/essai","issue":12,"text":"Bonjour."}),
    );
    let reason = failed(&f, &run);
    assert!(
        reason.contains("charles-test") && reason.contains("autre-compte"),
        "{reason}"
    );
    assert_eq!(f.github_requests("/comments"), 0);

    let (run, card) =
        f.github_prepare_comment_as("comment-2", "Bonjour du second.", Some("autre-compte"));
    assert_eq!(card["arguments"]["brouillon"]["login"], "autre-compte");
    assert_eq!(f.github_requests("/comments"), 0);
    f.github_resolve(&card, "send-second", json!("allow_once"), None)
        .unwrap();
    f.github_settle(&run);
    assert_eq!(done(&f, &run)["compte"], "autre-compte");
    let seen = f.oauth.seen.lock().unwrap();
    let second_token = seen.github.live_tokens[1].clone();
    let (_, auth) = seen
        .github
        .authorization
        .iter()
        .rev()
        .find(|(u, _)| u.ends_with("/comments"))
        .unwrap();
    assert_eq!(*auth, format!("Bearer {second_token}"));
    assert_eq!(seen.github.comments.len(), 1);
}

#[test]
fn github_draft_is_not_sent_when_its_account_lost_the_bot_activation() {
    let mut f = Fixture::new();
    f.github_connect();
    let (run, card) = f.github_prepare_comment("draft-off", "Texte du brouillon.");
    // Another account keeps the shared tools available while the first is switched off.
    f.github_add_account();
    let first = f.github_accounts()[0]["server"]["id"].clone();
    f.post(
        "/api/mcp/select",
        json!({"id":first,"bot_id":"grokbot","enabled":false}),
    );
    f.github_resolve(&card, "send-off", json!("allow_once"), None)
        .unwrap();
    f.github_settle(&run);
    let reason = failed(&f, &run);
    assert!(reason.contains("plus activés"), "{reason}");
    assert_eq!(f.github_requests("/comments"), 0);
    assert!(f.published_comments().is_empty());
}

#[test]
fn github_existing_draft_still_sends_with_its_account_when_another_account_is_added() {
    let mut f = Fixture::new();
    f.github_connect();
    let (run, card) = f.github_prepare_comment("draft-two", "Texte du brouillon.");
    // A second account makes a fresh call ambiguous, never the draft already bound to the first.
    f.github_add_account();
    f.github_resolve(&card, "send-two", json!("allow_once"), None)
        .unwrap();
    f.github_settle(&run);
    assert_eq!(done(&f, &run)["compte"], "charles-test");
    let seen = f.oauth.seen.lock().unwrap();
    let first_token = seen.github.live_tokens[0].clone();
    let (_, auth) = seen
        .github
        .authorization
        .iter()
        .rev()
        .find(|(u, _)| u.ends_with("/comments"))
        .unwrap();
    assert_eq!(*auth, format!("Bearer {first_token}"));
    assert_eq!(seen.github.comments.len(), 1);
}

#[test]
fn github_conversation_choice_is_not_replaced_by_another_account_when_it_needs_reconnecting() {
    let mut f = Fixture::new();
    f.github_connect();
    f.github_add_account();
    let conversation = f.github_conversation();
    assert_eq!(
        f.github_ask(&conversation, "keep-1", Some("charles-test"))["compte"],
        "charles-test"
    );
    f.db()
        .execute(
            "UPDATE wrapper_connector_catalogue SET phase='reconnect' WHERE id='github'",
            [],
        )
        .unwrap();
    let before = f.github_requests("/issues");
    let run = f.github_call_in(
        &conversation,
        "keep-2",
        GITHUB_TOOL,
        json!({"repository":"charles/essai"}),
    );
    let reason = failed(&f, &run);
    assert!(
        reason.contains("charles-test") && reason.contains("reconnecté"),
        "{reason}"
    );
    assert_eq!(
        f.github_requests("/issues"),
        before,
        "the other account is not called"
    );
    // Naming the other account still works and becomes the conversation's choice.
    let other = f.github_ask(&conversation, "keep-3", Some("autre-compte"));
    assert_eq!(other["compte"], "autre-compte");
    assert_eq!(
        f.github_ask(&conversation, "keep-4", None)["compte"],
        "autre-compte"
    );
}

const GMAIL_TOOL: &str = "gmail_list_recent_messages";
const GMAIL_SECRETS: [&str; 3] = [
    "gmail-access-secret",
    "gmail-refresh-secret",
    "gmail-renewed-secret",
];
const GMAIL_SCOPES: [&str; 3] = [
    "https://www.googleapis.com/auth/gmail.readonly",
    "https://www.googleapis.com/auth/gmail.compose",
    "https://www.googleapis.com/auth/gmail.modify",
];

impl Fixture {
    /// The public Google client of Ezerbot is build configuration, never a form field.
    fn with_google_client(&self) {
        std::env::set_var(
            "EZERBOT_GOOGLE_CLIENT_ID",
            "123456-fixturepublic.apps.googleusercontent.com",
        );
    }
    /// Six messages, the newest first: only the five newest are ever read.
    fn gmail_inbox(&self) {
        let inbox = (1..=6)
            .rev()
            .map(|n| {
                json!({
                    "id":format!("m{n}"),
                    "from":format!("Alice <alice{n}@example.test>"),
                    "subject":format!("Courriel {n} <b>important</b>"),
                    "date":format!("Tue, 20 Oct 2026 09:0{n}:00 +0000"),
                    "text":format!("Bonjour, ceci est le texte du courriel {n}."),
                })
            })
            .collect();
        self.oauth.seen.lock().unwrap().gmail.inbox = inbox;
    }
    fn gmail_start(&mut self) {
        self.with_google_client();
        let run = self.connect("gmail");
        self.decide(&run, json!("allow_once"));
    }
    fn gmail_connect(&mut self) {
        self.gmail_inbox();
        self.gmail_start();
        let code = self.callback("gmail", false, false);
        assert_eq!(code, 200, "{}", self.item("gmail")["message"]);
        wait("gmail connected", || {
            self.item("gmail")["status"] == "connected"
        });
        let server = self.item("gmail")["server"].clone();
        self.post(
            "/api/mcp/select",
            json!({"id":server["id"],"bot_id":"grokbot","enabled":true}),
        );
    }
    fn gmail_secret(&self) -> Option<Value> {
        let id: String = self
            .db()
            .query_row(
                "SELECT server_id FROM wrapper_connector_catalogue WHERE id='gmail'",
                [],
                |r| r.get(0),
            )
            .ok()?;
        grokbot_wrapper::features::forms::vault(&self.app.feature_context())
            .unwrap()
            .get(&format!("MCP_{id}_ENV").to_uppercase())
            .unwrap()
            .map(|raw| serde_json::from_str::<Value>(&raw).unwrap()["variables"].clone())
    }
    fn assert_no_gmail_token_kept(&self) {
        let kept = self.gmail_secret().unwrap_or_default();
        assert!(kept.get("ACCESS_TOKEN").is_none() && kept.get("REFRESH_TOKEN").is_none());
    }
    fn gmail_requests(&self, part: &str) -> usize {
        self.github_requests(part)
    }
    fn new_mail(&self) -> Vec<grokbot_wrapper::features::mcp::MailEvent> {
        grokbot_wrapper::features::mcp::gmail_new_messages(&self.app.feature_context()).unwrap()
    }
    fn add_mail(&self, id: &str, subject: &str) {
        self.oauth.seen.lock().unwrap().gmail.inbox.insert(
            0,
            json!({
                "id":id,
                "from":"Bruno <bruno@example.test>",
                "subject":subject,
                "date":"Wed, 21 Oct 2026 10:00:00 +0000",
                "text":format!("Texte de {subject}."),
            }),
        );
    }
}

#[test]
fn gmail_card_has_no_client_fields_and_explains_a_missing_google_client() {
    let mut f = Fixture::new();
    let gmail = f.item("gmail");
    assert_eq!(gmail["needs_client"], false);
    assert_eq!(gmail["configured"], false);
    assert!(gmail["unconfigured_message"]
        .as_str()
        .unwrap()
        .contains("client Google d’Ezerbot n’est pas configuré"));
    for other in ["calendar", "drive"] {
        assert_eq!(f.item(other)["needs_client"], true, "{other}");
    }
    assert_eq!(f.item("notion")["needs_client"], false);
    assert_eq!(f.item("github")["needs_client"], false);
    let before = f.state()["forms"]["requests"].as_array().unwrap().len();
    for body in [
        br#"{"id":"gmail"}"#.as_slice(),
        br#"{"id":"gmail","client_id":"mine","client_secret":"fixture-oauth-secret"}"#,
    ] {
        let error = f
            .app
            .api("POST", "/api/connectors/connect", body)
            .unwrap_err();
        assert_eq!(error.status, 400);
        assert!(
            error
                .message
                .contains("client Google d’Ezerbot n’est pas configuré"),
            "{}",
            error.message
        );
        assert!(!error.message.contains("fixture-oauth-secret"));
    }
    assert_eq!(
        f.state()["forms"]["requests"].as_array().unwrap().len(),
        before
    );
    assert_eq!(f.item("gmail")["status"], "disconnected");
    assert_eq!(f.oauth.count("/token"), 0);
    // Even with the client configured, a typed identifier or secret is refused, not stored.
    f.with_google_client();
    assert_eq!(f.item("gmail")["configured"], true);
    assert_eq!(f.item("gmail")["unconfigured_message"], Value::Null);
    let error = f
        .app
        .api(
            "POST",
            "/api/connectors/connect",
            br#"{"id":"gmail","client_id":"mine","client_secret":"fixture-oauth-secret"}"#,
        )
        .unwrap_err();
    assert_eq!(error.status, 400);
    assert!(f.gmail_secret().is_none());
}

#[test]
fn gmail_connects_with_the_public_client_and_pkce_and_never_sends_a_secret() {
    let mut f = Fixture::new();
    f.gmail_inbox();
    f.gmail_start();
    let url = f.item("gmail")["authorization_url"]
        .as_str()
        .unwrap()
        .to_owned();
    let query: std::collections::HashMap<_, _> = url::Url::parse(&url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    assert_eq!(
        query["client_id"],
        "123456-fixturepublic.apps.googleusercontent.com"
    );
    assert_eq!(query["code_challenge_method"], "S256");
    assert_eq!(query["response_type"], "code");
    assert_eq!(query["access_type"], "offline");
    assert_eq!(query["prompt"], "select_account consent");
    assert!(query["redirect_uri"].starts_with("http://127.0.0.1:"));
    assert_eq!(
        query["scope"].split(' ').collect::<Vec<_>>(),
        GMAIL_SCOPES,
        "Gmail asks for its three scopes and nothing else"
    );
    assert!(!url.contains("client_secret"));
    assert_eq!(f.callback("gmail", false, false), 200);
    wait("gmail connected", || {
        f.item("gmail")["status"] == "connected"
    });
    let item = f.item("gmail");
    assert_eq!(item["account"], "charles@example.test");
    assert_eq!(item["has_client"], true);
    {
        let seen = f.oauth.seen.lock().unwrap();
        let exchange = &seen.gmail.token_posts[0];
        assert_eq!(
            exchange["client_id"],
            "123456-fixturepublic.apps.googleusercontent.com"
        );
        assert_eq!(exchange["grant_type"], "authorization_code");
        assert!(exchange["redirect_uri"].starts_with("http://127.0.0.1:"));
        assert!(!exchange["code_verifier"].is_empty());
        assert!(!exchange.contains_key("client_secret"));
        for (_, body) in &seen.requests {
            assert!(!body.contains("client_secret"), "{body}");
        }
    }
    // The vault of this connection holds the tokens, and no client at all.
    let kept = f.gmail_secret().unwrap();
    assert!(kept["ACCESS_TOKEN"]
        .as_str()
        .unwrap()
        .starts_with("gmail-access-secret"));
    assert!(kept["REFRESH_TOKEN"].is_string());
    assert!(kept.get("CLIENT_ID").is_none() && kept.get("CLIENT_SECRET").is_none());
    for secret in GMAIL_SECRETS {
        assert!(!f.state().to_string().contains(secret));
        assert!(!std::fs::read(&f.path)
            .unwrap()
            .windows(secret.len())
            .any(|w| w == secret.as_bytes()));
    }
}

#[test]
fn gmail_refusal_or_cancellation_leaves_no_token_behind() {
    let mut f = Fixture::new();
    f.gmail_start();
    // The browser came back with another state, then with a refusal.
    assert_eq!(f.callback("gmail", false, true), 400);
    assert_eq!(f.callback("gmail", true, false), 400);
    assert_eq!(f.item("gmail")["status"], "reconnect");
    assert_eq!(f.oauth.count("/gmail/token"), 0);
    f.assert_no_gmail_token_kept();
    // A sign-in cancelled by a disconnect cannot be completed by a late callback.
    let run = f.connect("gmail");
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.item("gmail")["status"], "authorizing");
    let url = f.item("gmail")["authorization_url"]
        .as_str()
        .unwrap()
        .to_owned();
    let removed = f.post("/api/connectors/disconnect", json!({"id":"gmail"}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&removed, json!("allow_once"));
    assert_eq!(f.item("gmail")["status"], "disconnected");
    let query: std::collections::HashMap<_, _> = url::Url::parse(&url)
        .unwrap()
        .query_pairs()
        .into_owned()
        .collect();
    let mut late = url::Url::parse(&query["redirect_uri"]).unwrap();
    late.query_pairs_mut()
        .append_pair("state", &query["state"])
        .append_pair("code", "late-code");
    let _ = ureq::get(late.as_str()).call();
    assert_eq!(f.oauth.count("/gmail/token"), 0);
    assert_eq!(f.item("gmail")["status"], "disconnected");
    f.assert_no_gmail_token_kept();
}

#[test]
fn gmail_reads_the_five_latest_messages_without_a_card_and_without_a_token() {
    let mut f = Fixture::new();
    f.gmail_connect();
    let run = f.github_call("read-mail", GMAIL_TOOL, json!({}));
    let output = done(&f, &run);
    assert_eq!(output["compte"], "charles@example.test");
    let mails = output["courriels"].as_array().unwrap();
    assert_eq!(mails.len(), 5, "five latest messages, not six");
    assert_eq!(mails[0]["id"], "m6");
    assert_eq!(mails[0]["expediteur"], "Alice <alice6@example.test>");
    assert_eq!(mails[0]["objet"], "Courriel 6 <b>important</b>");
    assert_eq!(mails[0]["date"], "Tue, 20 Oct 2026 09:06:00 +0000");
    assert_eq!(
        mails[0]["texte"],
        "Bonjour, ceci est le texte du courriel 6."
    );
    assert_eq!(mails[4]["id"], "m2");
    let text = output["texte"].as_str().unwrap();
    assert!(text.contains("compte Gmail : charles@example.test"));
    assert!(text.contains("Courriel 6 <b>important</b>") && text.contains("Courriel 2"));
    assert!(!text.contains("Courriel 1 "));
    assert!(!text.contains("Version HTML"));
    {
        let seen = f.oauth.seen.lock().unwrap();
        let (_, auth) = seen
            .gmail
            .authorization
            .iter()
            .rev()
            .find(|(url, _)| url.contains("/messages/m6"))
            .unwrap();
        assert_eq!(*auth, format!("Bearer {}", seen.gmail.live_tokens[0]));
    }
    assert_eq!(f.gmail_requests("/messages?labelIds=INBOX&maxResults=5"), 1);
    // The model saw the messages, never a token.
    let inputs = format!("{:?}", f.ai.seen.lock().unwrap().bodies);
    assert!(inputs.contains("Courriel 6") && inputs.contains("alice6@example.test"));
    for secret in GMAIL_SECRETS {
        assert!(!inputs.contains(secret));
        assert!(!f.state().to_string().contains(secret));
    }
    // A smaller count is honoured; a larger one is refused rather than read.
    let run = f.github_call("read-two", GMAIL_TOOL, json!({"count":2}));
    assert_eq!(done(&f, &run)["courriels"].as_array().unwrap().len(), 2);
    let run = f.github_call("read-many", GMAIL_TOOL, json!({"count":500}));
    assert!(failed(&f, &run).contains("entre 1 et 10"));
}

#[test]
fn gmail_tools_are_offered_only_while_connected_and_enabled() {
    let mut f = Fixture::new();
    f.gmail_inbox();
    f.gmail_start();
    assert_eq!(f.callback("gmail", false, false), 200);
    wait("gmail connected", || {
        f.item("gmail")["status"] == "connected"
    });
    // Connected, but not given to the bot: a sentence, and the provider hears nothing.
    let before = f.gmail_requests("/messages");
    let run = f.github_call("not-enabled", GMAIL_TOOL, json!({}));
    assert!(failed(&f, &run).contains("Activez-le"), "not enabled");
    assert_eq!(f.gmail_requests("/messages"), before);
    // GitHub, when connected next to it, keeps its own tools.
    f.github_connect();
    let run = f.github_list("github-beside-gmail", "charles/essai");
    assert_eq!(done(&f, &run)["compte"], "charles-test");
}

#[test]
fn gmail_new_message_is_one_event_with_its_fields_and_only_once_per_identifier() {
    let mut f = Fixture::new();
    f.gmail_connect();
    // The first look only learns what is already in the box: nothing is "new" yet.
    assert!(f.new_mail().is_empty());
    assert!(f.new_mail().is_empty());
    f.add_mail("m7", "Rendez-vous demain");
    let events = f.new_mail();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.id, "m7");
    assert_eq!(event.expediteur, "Bruno <bruno@example.test>");
    assert_eq!(event.objet, "Rendez-vous demain");
    assert_eq!(event.date, "Wed, 21 Oct 2026 10:00:00 +0000");
    assert_eq!(event.texte, "Texte de Rendez-vous demain.");
    // The same identifier does not come out again, even when the box lists it again.
    assert!(f.new_mail().is_empty());
    assert!(f.new_mail().is_empty());
    // Two messages arrive: each comes out once, the older first.
    f.add_mail("m8", "Facture");
    f.add_mail("m9", "Relance");
    let events = f.new_mail();
    assert_eq!(
        events.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        ["m8", "m9"]
    );
    assert!(f.new_mail().is_empty());
    // The event is data of the mail: it carries no token.
    let shown = format!("{events:?}");
    for secret in GMAIL_SECRETS {
        assert!(!shown.contains(secret));
    }
}

#[test]
fn gmail_events_stop_when_the_account_is_not_connected() {
    let mut f = Fixture::new();
    let error =
        grokbot_wrapper::features::mcp::gmail_new_messages(&f.app.feature_context()).unwrap_err();
    assert!(
        error.message.contains("Gmail n’est pas connecté"),
        "{}",
        error.message
    );
    f.gmail_connect();
    assert!(f.new_mail().is_empty());
    let removed = f.post("/api/connectors/disconnect", json!({"id":"gmail"}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&removed, json!("allow_once"));
    assert_eq!(f.item("gmail")["status"], "disconnected");
    f.assert_no_gmail_token_kept();
    assert_eq!(f.item("gmail")["has_tokens"], false);
    assert!(grokbot_wrapper::features::mcp::gmail_new_messages(&f.app.feature_context()).is_err());
    // A new connection starts again from what the box holds, not from old identifiers.
    f.gmail_start();
    assert_eq!(f.callback("gmail", false, false), 200);
    wait("gmail reconnected", || {
        f.item("gmail")["status"] == "connected"
    });
    assert!(f.new_mail().is_empty());
}

#[test]
fn gmail_renews_an_expired_access_before_reading_and_a_dead_refresh_asks_to_reconnect() {
    let mut f = Fixture::new();
    f.gmail_connect();
    f.db()
        .execute(
            "UPDATE wrapper_connector_catalogue SET expires_at=1 WHERE id='gmail'",
            [],
        )
        .unwrap();
    let before = f.oauth.count("/gmail/token");
    let run = f.github_call("renewed-read", GMAIL_TOOL, json!({}));
    assert_eq!(done(&f, &run)["courriels"].as_array().unwrap().len(), 5);
    assert_eq!(f.oauth.count("/gmail/token"), before + 1);
    {
        let seen = f.oauth.seen.lock().unwrap();
        let refresh = seen.gmail.token_posts.last().unwrap();
        assert_eq!(refresh["grant_type"], "refresh_token");
        assert_eq!(
            refresh["client_id"],
            "123456-fixturepublic.apps.googleusercontent.com"
        );
        assert!(!refresh.contains_key("client_secret"));
    }
    // Revoked at Google: the refresh token no longer works, so the card asks to reconnect.
    f.oauth.seen.lock().unwrap().gmail.live_tokens.clear();
    f.db()
        .execute(
            "UPDATE wrapper_connector_catalogue SET expires_at=1 WHERE id='gmail'",
            [],
        )
        .unwrap();
    let vault_id: String = f
        .db()
        .query_row(
            "SELECT server_id FROM wrapper_connector_catalogue WHERE id='gmail'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let vault = grokbot_wrapper::features::forms::vault(&f.app.feature_context()).unwrap();
    let key = format!("MCP_{vault_id}_ENV").to_uppercase();
    let mut kept: Value = serde_json::from_str(&vault.get(&key).unwrap().unwrap()).unwrap();
    kept["variables"]["REFRESH_TOKEN"] = json!("gmail-dead-refresh-secret");
    vault
        .set(
            &key,
            grokbot_forms::vault::MCP_ENV_PURPOSE,
            &kept.to_string(),
        )
        .unwrap();
    let run = f.github_call("dead-read", GMAIL_TOOL, json!({}));
    assert!(failed(&f, &run).contains("Reconnectez"), "reconnect");
    assert_eq!(f.item("gmail")["status"], "reconnect");
}

#[cfg(all(target_os = "macos", debug_assertions))]
#[test]
fn gmail_connection_fails_closed_when_the_keychain_refuses_and_writes_no_file_copy() {
    let mut f = Fixture::new();
    f.gmail_inbox();
    f.with_google_client();
    let missing = std::env::temp_dir().join("ezerbot-absent-keychain-for-gmail.keychain-db");
    std::env::set_var("GROKBOT_TEST_KEYCHAIN", &missing);
    grokbot_forms::vault::enable_os_vault(true);
    let run = f.connect("gmail");
    f.decide(&run, json!("allow_once"));
    // The browser is told the connection did not come through.
    assert_eq!(f.callback("gmail", false, false), 400);
    wait("keychain refusal", || {
        f.item("gmail")["status"] == "reconnect"
    });
    let message = f.item("gmail")["message"].as_str().unwrap().to_owned();
    assert!(message.contains("trousseau"), "{message}");
    assert!(f.item("gmail")["has_tokens"] == false);
    let vault_file = f.path.with_extension("secrets.json");
    let file = std::fs::read(&vault_file).unwrap_or_default();
    let database = std::fs::read(&f.path).unwrap();
    for secret in GMAIL_SECRETS {
        assert!(!file.windows(secret.len()).any(|w| w == secret.as_bytes()));
        assert!(!database
            .windows(secret.len())
            .any(|w| w == secret.as_bytes()));
        assert!(!f.state().to_string().contains(secret));
    }
    std::env::remove_var("GROKBOT_TEST_KEYCHAIN");
    grokbot_forms::vault::enable_os_vault(false);
}

#[test]
fn gmail_read_in_flight_never_uses_a_replacement_account_or_a_bot_that_lost_gmail() {
    let mut f = Fixture::new();
    f.gmail_connect();
    let path = f.path.clone();
    let swap = move || {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.busy_timeout(Duration::from_secs(4)).unwrap();
        c.execute(
            "UPDATE wrapper_connector_catalogue SET account_email='autre@example.test' WHERE id='gmail'",
            [],
        )
        .unwrap();
    };
    f.oauth.seen.lock().unwrap().gmail.during_listing = Some(Box::new(swap));
    let before = f.gmail_requests("/messages/m");
    let run = f.github_call("swap-account", GMAIL_TOOL, json!({}));
    assert!(failed(&f, &run).contains("a changé"), "account changed");
    assert_eq!(
        f.gmail_requests("/messages/m"),
        before,
        "no message of the replacement mailbox was read"
    );
    // The bot loses Gmail while the listing is answered.
    f.db()
        .execute(
            "UPDATE wrapper_connector_catalogue SET account_email='charles@example.test' WHERE id='gmail'",
            [],
        )
        .unwrap();
    let path = f.path.clone();
    let revoke = move || {
        let c = rusqlite::Connection::open(&path).unwrap();
        c.busy_timeout(Duration::from_secs(4)).unwrap();
        c.execute("DELETE FROM wrapper_mcp_bots", []).unwrap();
    };
    f.oauth.seen.lock().unwrap().gmail.during_listing = Some(Box::new(revoke));
    let run = f.github_call("lose-activation", GMAIL_TOOL, json!({}));
    assert!(failed(&f, &run).contains("a changé"), "activation lost");
    assert_eq!(f.gmail_requests("/messages/m"), before);
}

#[test]
fn gmail_older_mail_entering_the_watched_window_is_not_a_new_message() {
    let mut f = Fixture::new();
    f.gmail_inbox();
    // Thirty messages: the watch only looks at the newest 25.
    {
        let mut seen = f.oauth.seen.lock().unwrap();
        let inbox = (1..=30)
            .rev()
            .map(|n| json!({"id":format!("m{n}"),"from":"Alice <alice@example.test>","subject":format!("Courriel {n}"),"date":"Tue, 20 Oct 2026 09:00:00 +0000","text":"Texte."}))
            .collect();
        seen.gmail.inbox = inbox;
    }
    f.gmail_start();
    assert_eq!(f.callback("gmail", false, false), 200);
    wait("gmail connected", || {
        f.item("gmail")["status"] == "connected"
    });
    assert!(f.new_mail().is_empty());
    // One message is archived: the 26th enters the window, and it is old mail.
    f.oauth.seen.lock().unwrap().gmail.inbox.remove(0);
    assert!(f.new_mail().is_empty());
    // A genuine arrival still comes out, once.
    f.add_mail("m31", "Vraie nouveauté");
    let events = f.new_mail();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, "m31");
    assert!(f.new_mail().is_empty());
}

#[test]
fn gmail_watch_survives_the_disconnection_of_another_google_service_with_the_same_address() {
    let mut f = Fixture::new();
    f.gmail_connect();
    assert!(f.new_mail().is_empty());
    let run = f.connect("calendar");
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.callback("calendar", false, false), 200);
    f.db()
        .execute(
            "UPDATE wrapper_connector_catalogue SET account_email='charles@example.test' WHERE id='calendar'",
            [],
        )
        .unwrap();
    f.add_mail("m7", "Arrivé avant la déconnexion de l’agenda");
    let removed = f.post("/api/connectors/disconnect", json!({"id":"calendar"}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&removed, json!("allow_once"));
    assert_eq!(f.item("calendar")["status"], "disconnected");
    assert_eq!(f.item("gmail")["status"], "connected");
    let events = f.new_mail();
    assert_eq!(
        events.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
        ["m7"],
        "Gmail's baseline was not reset by another service"
    );
}

const SLACK_READ: &str = "slack_read_channel";

impl Fixture {
    /// The public client of Ezerbot's Slack app is build configuration, never a form field.
    fn with_slack_client(&self) {
        std::env::set_var("EZERBOT_SLACK_CLIENT_ID", SLACK_CLIENT);
    }
    /// Connect in one click, finish the browser leg, and enable the tool for the default bot.
    fn slack_connect(&mut self) {
        self.with_slack_client();
        let run = self.connect("slack");
        self.decide(&run, json!("allow_once"));
        assert_eq!(self.item("slack")["status"], "authorizing");
        assert_eq!(self.callback("slack", false, false), 200);
        wait("slack connected", || {
            self.item("slack")["status"] == "connected"
        });
        let server = self.item("slack")["server"].clone();
        self.post(
            "/api/mcp/select",
            json!({"id":server["id"],"bot_id":"grokbot","enabled":true}),
        );
    }
    fn slack_requests(&self, part: &str) -> Vec<(String, String)> {
        self.oauth
            .seen
            .lock()
            .unwrap()
            .requests
            .iter()
            .filter(|(url, _)| url.starts_with("/slack/") && url.contains(part))
            .cloned()
            .collect()
    }
    fn slack_secret(&self) -> Option<Value> {
        let id: String = self
            .db()
            .query_row(
                "SELECT server_id FROM wrapper_connector_catalogue WHERE id='slack'",
                [],
                |r| r.get(0),
            )
            .ok()?;
        grokbot_wrapper::features::forms::vault(&self.app.feature_context())
            .unwrap()
            .get(&format!("MCP_{id}_ENV").to_uppercase())
            .unwrap()
            .map(|raw| serde_json::from_str::<Value>(&raw).unwrap()["variables"].clone())
    }
    fn slack_read(&mut self, id: &str, arguments: Value) -> String {
        self.github_call(id, SLACK_READ, arguments)
    }
    /// Nothing of the access token is in the state, the database, the run or the model's input.
    fn assert_slack_token_nowhere(&self, run: &str) {
        let inputs = format!("{:?}", self.ai.seen.lock().unwrap().bodies);
        let events = format!("{:?}", store::events(&self.db(), run, 0).unwrap());
        let database = std::fs::read(&self.path).unwrap();
        for secret in [SLACK_ACCESS, "xoxp-"] {
            assert!(!inputs.contains(secret), "model input holds {secret}");
            assert!(!events.contains(secret), "run events hold {secret}");
            assert!(!database
                .windows(secret.len())
                .any(|w| w == secret.as_bytes()));
        }
    }
}

#[test]
fn slack_card_has_no_client_fields_and_explains_a_missing_client() {
    let mut f = Fixture::new();
    let slack = f.item("slack");
    assert_eq!(slack["needs_client"], false);
    assert_eq!(slack["configured"], false);
    assert_eq!(slack["redirect_uri"], Value::Null);
    let said = slack["unconfigured_message"].as_str().unwrap();
    assert!(
        said.contains("client Slack d’Ezerbot n’est pas configuré"),
        "{said}"
    );
    let before = f.state()["forms"]["requests"].as_array().unwrap().len();
    for body in [
        br#"{"id":"slack"}"#.as_slice(),
        br#"{"id":"slack","client_id":"mine","client_secret":"fixture-oauth-secret"}"#,
    ] {
        let error = f
            .app
            .api("POST", "/api/connectors/connect", body)
            .unwrap_err();
        assert_eq!(error.status, 400);
        assert!(
            error
                .message
                .contains("client Slack d’Ezerbot n’est pas configuré"),
            "{}",
            error.message
        );
        assert!(!error.message.contains("fixture-oauth-secret"));
    }
    assert_eq!(
        f.state()["forms"]["requests"].as_array().unwrap().len(),
        before,
        "a missing client opens no approval card"
    );
    assert!(f.slack_requests("").is_empty());
    assert_eq!(f.item("slack")["status"], "disconnected");
    // With the client configured, the card says so, and a typed secret is refused, not stored.
    f.with_slack_client();
    let slack = f.item("slack");
    assert_eq!(slack["configured"], true);
    assert_eq!(slack["unconfigured_message"], Value::Null);
    assert_eq!(slack["needs_client"], false);
    let error = f
        .app
        .api(
            "POST",
            "/api/connectors/connect",
            br#"{"id":"slack","client_id":"mine","client_secret":"fixture-oauth-secret"}"#,
        )
        .unwrap_err();
    assert_eq!(error.status, 400);
    assert!(f.slack_secret().is_none());
}

#[test]
fn slack_connects_with_the_public_client_and_pkce_and_sends_no_client_secret() {
    let mut f = Fixture::new();
    f.with_slack_client();
    let run = f.connect("slack");
    // The approval names the vault, which may be a local file on an unsigned host.
    f.wait_approval(&run);
    let request = f.state()["forms"]["requests"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["run_id"] == run)
        .unwrap()
        .clone();
    let connection = request["arguments"]["catalogue"]["connexion"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        connection.contains("conservé dans le coffre"),
        "{connection}"
    );
    assert!(
        !connection.contains("jamais dans un fichier"),
        "{connection}"
    );
    f.decide(&run, json!("allow_once"));
    let item = f.item("slack");
    assert_eq!(item["status"], "authorizing");
    let authorize = url::Url::parse(item["authorization_url"].as_str().unwrap()).unwrap();
    let query: std::collections::HashMap<_, _> = authorize.query_pairs().into_owned().collect();
    assert_eq!(query["client_id"], SLACK_CLIENT);
    assert_eq!(query["code_challenge_method"], "S256");
    assert!(query["redirect_uri"].starts_with("http://127.0.0.1:"));
    assert!(!query.contains_key("client_secret"));
    assert!(query["scope"].contains("channels:history"));
    assert_eq!(f.callback("slack", false, false), 200);
    wait("slack connected", || {
        f.item("slack")["status"] == "connected"
    });
    let exchanges = f.slack_requests("/token");
    assert_eq!(exchanges.len(), 1);
    let sent: std::collections::HashMap<String, String> =
        url::form_urlencoded::parse(exchanges[0].1.as_bytes())
            .into_owned()
            .collect();
    assert_eq!(sent["client_id"], SLACK_CLIENT);
    assert!(sent.contains_key("code_verifier"));
    assert!(
        !sent.contains_key("client_secret"),
        "no client secret leaves the Mac: {sent:?}"
    );
    let item = f.item("slack");
    assert_eq!(item["account"], "Atelier Ezerbot");
    assert_eq!(item["has_tokens"], true);
    assert_eq!(item["needs_client"], false);
    assert_eq!(item["server"]["tools"][0]["name"], "read_channel");
    // The vault holds the user token and no client at all.
    let kept = f.slack_secret().unwrap();
    assert_eq!(kept["ACCESS_TOKEN"], SLACK_ACCESS);
    assert!(kept.get("CLIENT_SECRET").is_none() && kept.get("CLIENT_ID").is_none());
    assert!(!f.state().to_string().contains(SLACK_ACCESS));
}

#[test]
fn slack_reads_a_channel_without_a_card_names_the_messages_and_keeps_the_token_out_of_the_thread() {
    let mut f = Fixture::new();
    f.slack_connect();
    let run = f.slack_read("read-general", json!({"channel":"#general"}));
    let output = done(&f, &run);
    assert_eq!(output["compte"], "Atelier Ezerbot");
    assert_eq!(output["canal"], "general");
    let messages = output["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 2, "a channel join is not a message");
    assert_eq!(messages[0]["auteur"], "Charles Test");
    assert_eq!(messages[0]["texte"], "Bonjour à toute l’équipe");
    assert_eq!(messages[1]["auteur"], "Alice Martin");
    assert_eq!(
        messages[1]["texte"],
        "Le déploiement est terminé, merci @Charles Test."
    );
    let text = output["texte"].as_str().unwrap();
    assert!(
        text.contains("Atelier Ezerbot") && text.contains("#general"),
        "{text}"
    );
    assert!(
        text.contains("Charles Test : Bonjour à toute l’équipe"),
        "{text}"
    );
    assert!(text.find("Bonjour").unwrap() < text.find("déploiement").unwrap());
    assert!(!text.contains("<@") && !text.contains("a rejoint"));
    assert_eq!(output["autres_messages"], true);
    assert_eq!(f.slack_requests("conversations.list").len(), 1);
    let history = f.slack_requests("conversations.history");
    assert_eq!(history.len(), 1);
    assert_eq!(fields_of(&history[0].0)["channel"], "C0GENERAL");
    // The model saw the messages, and no token reached it, the thread or the database.
    let inputs = format!("{:?}", f.ai.seen.lock().unwrap().bodies);
    assert!(inputs.contains("Alice Martin") && inputs.contains("Bonjour à toute l’équipe"));
    f.assert_slack_token_nowhere(&run);
    assert!(!f.state().to_string().contains(SLACK_ACCESS));
}

#[test]
fn slack_reads_by_id_or_private_name_and_says_what_a_refused_read_means() {
    let mut f = Fixture::new();
    f.slack_connect();
    let run = f.slack_read("read-id", json!({"channel":"C0GENERAL","limit":1}));
    done(&f, &run);
    assert_eq!(
        f.slack_requests("conversations.list").len(),
        0,
        "an identifier needs no lookup by name"
    );
    assert_eq!(
        fields_of(&f.slack_requests("conversations.history")[0].0)["limit"],
        "1"
    );
    let run = f.slack_read("read-private", json!({"channel":"projet-prive"}));
    assert!(done(&f, &run)["texte"]
        .as_str()
        .unwrap()
        .contains("Le budget est validé"));
    let run = f.slack_read("read-outside", json!({"channel":"dehors"}));
    let reason = failed(&f, &run);
    assert!(reason.contains("n’êtes pas membre"), "{reason}");
    let run = f.slack_read("read-unknown", json!({"channel":"inconnu"}));
    let reason = failed(&f, &run);
    assert!(
        reason.contains("Aucun canal Slack ne s’appelle inconnu"),
        "{reason}"
    );
    // The unknown name stopped at the lookup: no read went out for it.
    assert_eq!(f.slack_requests("conversations.history").len(), 3);
    f.assert_slack_token_nowhere(&run);
}

#[test]
fn slack_revoked_access_asks_to_reconnect_and_calls_nothing_else() {
    let mut f = Fixture::new();
    f.slack_connect();
    f.oauth.seen.lock().unwrap().slack.live = None;
    let run = f.slack_read("read-revoked", json!({"channel":"general"}));
    let reason = failed(&f, &run);
    assert!(reason.contains("Reconnectez Slack"), "{reason}");
    assert_eq!(f.item("slack")["status"], "reconnect");
    assert_eq!(f.slack_requests("conversations.history").len(), 0);
}

#[test]
fn slack_disconnect_removes_the_secret_and_says_whether_slack_confirmed_the_revocation() {
    let mut f = Fixture::new();
    f.slack_connect();
    let run = f.post("/api/connectors/disconnect", json!({"id":"slack"}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.item("slack")["status"], "disconnected");
    assert_eq!(f.item("slack")["warning"], false);
    assert_eq!(f.slack_requests("/revoke").len(), 1);
    assert!(f.slack_secret().is_none());
    f.oauth.seen.lock().unwrap().slack.revoke_error = true;
    f.slack_connect();
    let run = f.post("/api/connectors/disconnect", json!({"id":"slack"}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.item("slack")["warning"], true);
    assert!(
        f.slack_secret().is_none(),
        "the local token goes either way"
    );
}

#[test]
fn slack_missing_scope_drops_the_insufficient_token_and_the_next_connect_asks_for_consent_again() {
    let mut f = Fixture::new();
    f.slack_connect();
    let run = f.slack_read("read-limited", json!({"channel":"C0LIMITES1"}));
    let reason = failed(&f, &run);
    assert!(reason.contains("Reconnectez Slack"), "{reason}");
    let item = f.item("slack");
    assert_eq!(item["status"], "reconnect");
    assert_eq!(item["has_tokens"], false);
    assert!(
        f.slack_secret().is_none(),
        "the insufficient token is not kept"
    );
    // Reconnecting goes through the browser again instead of reusing that token.
    let exchanges = f.slack_requests("/token").len();
    let run = f.connect("slack");
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.item("slack")["status"], "authorizing");
    assert_eq!(f.callback("slack", false, false), 200);
    wait("slack reconnected", || {
        f.item("slack")["status"] == "connected"
    });
    assert_eq!(f.slack_requests("/token").len(), exchanges + 1);
}

#[test]
fn slack_names_the_workspace_with_the_token_when_the_exchange_only_gives_its_id() {
    let mut f = Fixture::new();
    f.oauth.seen.lock().unwrap().slack.team_id_only = true;
    f.slack_connect();
    assert_eq!(f.item("slack")["account"], "Atelier Ezerbot");
    let run = f.slack_read("read-named", json!({"channel":"general"}));
    let output = done(&f, &run);
    assert_eq!(output["compte"], "Atelier Ezerbot");
    assert!(output["texte"]
        .as_str()
        .unwrap()
        .contains("espace Atelier Ezerbot"));
}

#[test]
fn slack_forgets_a_client_typed_under_an_earlier_version_at_the_next_sign_in_and_at_disconnect() {
    let mut f = Fixture::new();
    f.slack_connect();
    let id: String = f
        .db()
        .query_row(
            "SELECT server_id FROM wrapper_connector_catalogue WHERE id='slack'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let key = format!("MCP_{id}_ENV").to_uppercase();
    let vault = grokbot_wrapper::features::forms::vault(&f.app.feature_context()).unwrap();
    let legacy = |tokens: bool| {
        let mut variables =
            json!({"CLIENT_ID":"old-typed-client","CLIENT_SECRET":"old-typed-secret"});
        if tokens {
            variables["ACCESS_TOKEN"] = json!("xoxp-old-token");
        }
        json!({ "variables": variables }).to_string()
    };
    vault
        .set(&key, grokbot_forms::vault::MCP_ENV_PURPOSE, &legacy(true))
        .unwrap();
    f.db()
        .execute(
            "UPDATE wrapper_connector_catalogue SET phase='reconnect' WHERE id='slack'",
            [],
        )
        .unwrap();
    let run = f.connect("slack");
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.callback("slack", false, false), 200);
    wait("slack connected", || {
        f.item("slack")["status"] == "connected"
    });
    let kept = f.slack_secret().unwrap();
    assert_eq!(kept["ACCESS_TOKEN"], SLACK_ACCESS);
    assert!(kept.get("CLIENT_SECRET").is_none() && kept.get("CLIENT_ID").is_none());
    // A disconnect keeps no client registration for Slack, even one an earlier version stored.
    vault
        .set(&key, grokbot_forms::vault::MCP_ENV_PURPOSE, &legacy(false))
        .unwrap();
    let run = f.post("/api/connectors/disconnect", json!({"id":"slack"}))["run_id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.decide(&run, json!("allow_once"));
    assert!(f.slack_secret().is_none());
}

#[cfg(all(target_os = "macos", debug_assertions))]
#[test]
fn slack_connection_fails_closed_when_the_keychain_refuses_and_writes_no_file_copy() {
    let mut f = Fixture::new();
    f.with_slack_client();
    let missing = std::env::temp_dir().join("ezerbot-absent-keychain-for-slack.keychain-db");
    std::env::set_var("GROKBOT_TEST_KEYCHAIN", &missing);
    grokbot_forms::vault::enable_os_vault(true);
    let run = f.connect("slack");
    f.decide(&run, json!("allow_once"));
    assert_eq!(f.callback("slack", false, false), 400);
    assert_eq!(f.item("slack")["status"], "reconnect");
    let message = f.item("slack")["message"].as_str().unwrap().to_owned();
    assert!(message.contains("trousseau"), "{message}");
    assert!(f.item("slack")["has_tokens"] == false);
    let vault_file = f.path.with_extension("secrets.json");
    let file = std::fs::read(&vault_file).unwrap_or_default();
    let database = std::fs::read(&f.path).unwrap();
    for secret in [SLACK_ACCESS.as_bytes(), b"xoxp-"] {
        assert!(!file.windows(secret.len()).any(|w| w == secret));
        assert!(!database.windows(secret.len()).any(|w| w == secret));
    }
    assert!(!f.state().to_string().contains(SLACK_ACCESS));
    std::env::remove_var("GROKBOT_TEST_KEYCHAIN");
    grokbot_forms::vault::enable_os_vault(false);
}
