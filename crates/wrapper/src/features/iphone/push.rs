//! RFC 8291 aes128gcm + RFC 8292 VAPID. ring supplies all cryptographic
//! primitives; no hand-written cipher. Push contains generic titles, never text,
//! secret names, tool arguments, or credentials. A reply is titled with its bot's
//! name. No production endpoints in debug.
use super::*;
use ring::{
    aead, agreement, hmac,
    rand::SystemRandom,
    signature::{self, KeyPair},
};
use std::sync::atomic::{AtomicBool, Ordering};
static BUSY: AtomicBool = AtomicBool::new(false);

pub(super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS iphone_vapid(id INTEGER PRIMARY KEY CHECK(id=1),pkcs8 BLOB NOT NULL);")?;
    let exists: bool = conn.query_row("SELECT EXISTS(SELECT 1 FROM iphone_vapid)", [], |r| {
        r.get(0)
    })?;
    if !exists {
        let key = signature::EcdsaKeyPair::generate_pkcs8(
            &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            &SystemRandom::new(),
        )
        .map_err(|_| bad(500, "Clé de notifications indisponible."))?;
        conn.execute("INSERT INTO iphone_vapid VALUES(1,?1)", [key.as_ref()])?;
    }
    Ok(())
}
fn key(conn: &Connection) -> Result<signature::EcdsaKeyPair, ApiError> {
    let bytes: Vec<u8> = conn.query_row("SELECT pkcs8 FROM iphone_vapid WHERE id=1", [], |r| {
        r.get(0)
    })?;
    signature::EcdsaKeyPair::from_pkcs8(
        &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
        &bytes,
        &SystemRandom::new(),
    )
    .map_err(|_| bad(500, "Clé de notifications illisible."))
}
pub(super) fn public_key(conn: &Connection) -> Result<String, ApiError> {
    Ok(B64.encode(key(conn)?.public_key().as_ref()))
}
#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Keys {
    p256dh: String,
    auth: String,
}
#[derive(Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct Subscription {
    endpoint: String,
    #[serde(rename = "expirationTime", default)]
    expiration: Option<u64>,
    keys: Keys,
}
fn endpoint_allowed(endpoint: &str) -> bool {
    let Ok(url) = url::Url::parse(endpoint) else {
        return false;
    };
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return false;
    }
    #[cfg(debug_assertions)]
    {
        // The sole debug destination is explicitly configured loopback.
        std::env::var("GROKBOT_FAKE_PUSH").ok().is_some_and(|base| {
            let Ok(base_url) = url::Url::parse(&base) else {
                return false;
            };
            base_url.scheme() == "http"
                && base_url.host_str() == Some("127.0.0.1")
                && base_url.port().is_some()
                && endpoint.starts_with(&format!("{}/", base.trim_end_matches('/')))
        })
    }
    #[cfg(not(debug_assertions))]
    {
        url.scheme() == "https"
            && url.port_or_known_default() == Some(443)
            && url.host_str().is_some_and(|host| {
                host == "fcm.googleapis.com"
                    || host == "updates.push.services.mozilla.com"
                    || host == "web.push.apple.com"
                    || host.ends_with(".push.apple.com")
            })
    }
}
pub(super) fn subscribe(conn: &Connection, id: &str, body: &[u8]) -> Result<Value, ApiError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        subscription: Option<Subscription>,
    }
    let input: Input = crate::parse(body)?;
    if let Some(ref sub) = input.subscription {
        if sub.endpoint.len() > 4096 || !endpoint_allowed(&sub.endpoint) {
            return Err(bad(
                400,
                "Service push non autorisé (les tests utilisent uniquement un faux service local).",
            ));
        }
        let public = B64
            .decode(&sub.keys.p256dh)
            .map_err(|_| bad(400, "Clé push invalide."))?;
        let auth = B64
            .decode(&sub.keys.auth)
            .map_err(|_| bad(400, "Secret push invalide."))?;
        if public.len() != 65 || public[0] != 4 || auth.len() != 16 {
            return Err(bad(400, "Clés push invalides."));
        }
        // Validate the point before persisting a subscription.
        encrypt(sub, b"validation")?;
    }
    conn.execute(
        "UPDATE iphone_devices SET subscription=?2,push_error=NULL WHERE id=?1",
        params![id, input.subscription.map(|sub| json!(sub).to_string())],
    )?;
    // Subscription is opt-in for future events, not a replay of the inbox.
    conn.execute(
        "INSERT OR IGNORE INTO iphone_push_claims SELECT ?1,id FROM wrapper_notifications",
        [id],
    )?;
    Ok(json!({"ok":true}))
}
// HKDF-Extract + one-block HKDF-Expand (all derived lengths <= SHA256 length).
fn derive(salt: &[u8], ikm: &[u8], info: &[u8], length: usize) -> Vec<u8> {
    let prk = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, salt), ikm);
    let mut input = info.to_vec();
    input.push(1);
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, prk.as_ref()), &input).as_ref()[..length].to_vec()
}
fn crypto_error() -> ApiError {
    bad(400, "Le chiffrement de notification a échoué.")
}
fn encrypt(sub: &Subscription, payload: &[u8]) -> Result<Vec<u8>, ApiError> {
    if payload.len() > 3000 {
        return Err(crypto_error());
    }
    let public = B64.decode(&sub.keys.p256dh).map_err(|_| crypto_error())?;
    let auth = B64.decode(&sub.keys.auth).map_err(|_| crypto_error())?;
    let private =
        agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &SystemRandom::new())
            .map_err(|_| crypto_error())?;
    let server = private.compute_public_key().map_err(|_| crypto_error())?;
    let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, &public);
    let mut info = b"WebPush: info\0".to_vec();
    info.extend_from_slice(&public);
    info.extend_from_slice(server.as_ref());
    let ikm = agreement::agree_ephemeral(private, &peer, |shared| derive(&auth, shared, &info, 32))
        .map_err(|_| crypto_error())?;
    let mut salt = [0; 16];
    getrandom::getrandom(&mut salt).map_err(|_| crypto_error())?;
    let cek = derive(&salt, &ikm, b"Content-Encoding: aes128gcm\0", 16);
    let nonce = derive(&salt, &ikm, b"Content-Encoding: nonce\0", 12);
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_128_GCM, &cek).map_err(|_| crypto_error())?,
    );
    let mut encrypted = payload.to_vec();
    encrypted.push(2); // last-record delimiter
    key.seal_in_place_append_tag(
        aead::Nonce::try_assume_unique_for_key(&nonce).map_err(|_| crypto_error())?,
        aead::Aad::empty(),
        &mut encrypted,
    )
    .map_err(|_| crypto_error())?;
    let mut result = salt.to_vec();
    result.extend_from_slice(&4096u32.to_be_bytes());
    result.push(65);
    result.extend_from_slice(server.as_ref());
    result.extend(encrypted);
    Ok(result)
}
fn vapid(conn: &Connection, endpoint: &str) -> Result<String, ApiError> {
    let url = url::Url::parse(endpoint).map_err(|_| crypto_error())?;
    let header = B64.encode(br#"{"typ":"JWT","alg":"ES256"}"#);
    let claims = B64.encode(
        json!({"aud":url.origin().ascii_serialization(),"exp":crate::now()/1000+3600}).to_string(),
    );
    let unsigned = format!("{header}.{claims}");
    let key = key(conn)?;
    let signature = key
        .sign(&SystemRandom::new(), unsigned.as_bytes())
        .map_err(|_| crypto_error())?;
    Ok(format!(
        "vapid t={unsigned}.{}, k={}",
        B64.encode(signature.as_ref()),
        B64.encode(key.public_key().as_ref())
    ))
}
struct Job {
    id: String,
    sub: Subscription,
    payload: Vec<u8>,
    notification: String,
}
pub(super) fn tick(ctx: &Context<'_>) -> Result<(), ApiError> {
    let enabled: bool = ctx.connection.query_row(
        "SELECT enabled AND push_tick<?1-5000 FROM iphone_settings WHERE id=1",
        [ctx.now_ms],
        |r| r.get(0),
    )?;
    if !enabled || BUSY.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            BUSY.store(false, Ordering::SeqCst);
        }
    }
    let reset = Reset;
    ctx.connection.execute(
        "UPDATE iphone_settings SET push_tick=?1 WHERE id=1",
        [ctx.now_ms],
    )?;
    super::super::notifications::project(ctx.connection, ctx.now_ms, false)?;
    let prefs = super::super::notifications::settings(ctx.connection)?;
    let mut stmt=ctx.connection.prepare("SELECT d.id,d.subscription,n.id,n.kind,n.conversation_id,n.card_id,
        COALESCE((SELECT b.name FROM wrapper_bots b WHERE n.kind='reply' AND b.account_id=n.account_id AND b.id=COALESCE(
            (SELECT r.bot_id FROM engine_runs r WHERE r.run_id=substr(n.id,7)),
            (SELECT c.bot_id FROM wrapper_conversations c WHERE c.account_id=n.account_id AND c.id=n.conversation_id))),n.title),
        d.visible,d.conversation_id,d.last_seen
        FROM iphone_devices d CROSS JOIN wrapper_notifications n WHERE d.status='active' AND d.subscription IS NOT NULL
        AND n.created_at>=d.created_at AND n.unread=1 AND NOT EXISTS(SELECT 1 FROM iphone_push_claims p WHERE p.device_id=d.id AND p.notification_id=n.id)
        AND EXISTS(SELECT 1 FROM wrapper_conversations c WHERE c.id=n.conversation_id AND c.deleted=0)
        ORDER BY n.created_at LIMIT 32")?;
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, bool>(7)?,
                r.get::<_, Option<String>>(8)?,
                r.get::<_, u64>(9)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    drop(stmt);
    let mut jobs = vec![];
    for (id, raw, notification, kind, conversation, card, title, visible, open, seen) in rows {
        // Claim before sending, independently of desktop claims: no replay after
        // restart, opt-in or DND. Stale presence cannot suppress forever.
        ctx.connection.execute(
            "INSERT OR IGNORE INTO iphone_push_claims VALUES(?1,?2)",
            params![id, notification],
        )?;
        let pending = if matches!(kind.as_str(), "approval" | "form" | "secret") {
            ctx.connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM forms_requests WHERE id=?1 AND status='pending')",
                [notification.trim_start_matches("input:")],
                |r| r.get::<_, bool>(0),
            )?
        } else {
            true
        };
        if !pending
            || !prefs.permits(&kind)
            || (visible
                && open.as_deref() == Some(&conversation)
                && ctx.now_ms.saturating_sub(seen) < 20_000)
        {
            continue;
        }
        if let Ok(sub) = serde_json::from_str::<Subscription>(&raw) {
            jobs.push(Job{id,sub,payload:json!({"title":title,"kind":kind,"id":notification,"conversation_id":conversation,"card_id":card}).to_string().into_bytes(),notification});
        }
    }
    if jobs.is_empty() {
        return Ok(());
    }
    let path = ctx.db_path.to_path_buf();
    ctx.provider.spawn_background(move || {
        let _reset = reset;
        let Ok(conn) = Connection::open(path) else {
            return;
        };
        let _ = conn.busy_timeout(std::time::Duration::from_secs(3));
        for job in jobs {
            // Revocation/disable between queueing and network delivery wins.
            let subscription = json!(job.sub).to_string();
            let live=conn.query_row("SELECT EXISTS(SELECT 1 FROM iphone_devices d,iphone_settings s,wrapper_notifications n WHERE d.id=?1 AND d.status='active' AND d.subscription=?2 AND s.enabled=1 AND n.id=?3 AND n.unread=1 AND (d.visible=0 OR COALESCE(d.conversation_id,'')<>n.conversation_id OR d.last_seen<=?4-20000))",params![job.id,subscription,job.notification,crate::now()],|r|r.get::<_,bool>(0)).unwrap_or(false);
            let kind: Option<String> = conn
                .query_row(
                    "SELECT kind FROM wrapper_notifications WHERE id=?1",
                    [&job.notification],
                    |r| r.get(0),
                )
                .optional()
                .unwrap_or(None);
            let permitted = settings_permit(&conn, kind.as_deref());
            if !live || !permitted {
                continue;
            }
            let result = send(&conn, &job);
            let (gone, error) = match result {
                Ok(()) => (false, None),
                Err((gone, text)) => (gone, Some(text)),
            };
            let _=conn.execute("UPDATE iphone_devices SET push_error=?2,subscription=CASE WHEN ?3 THEN NULL ELSE subscription END WHERE id=?1 AND subscription=?4",params![job.id,error,gone,subscription]);
        }
    });
    Ok(())
}
fn settings_permit(conn: &Connection, kind: Option<&str>) -> bool {
    super::super::notifications::settings(conn)
        .ok()
        .is_some_and(|prefs| kind.is_some_and(|kind| prefs.permits(kind)))
}
fn send(conn: &Connection, job: &Job) -> Result<(), (bool, String)> {
    if !endpoint_allowed(&job.sub.endpoint) {
        return Err((true, "Service push refusé.".into()));
    }
    let body = encrypt(&job.sub, &job.payload).map_err(|e| (false, e.message))?;
    let authorization = vapid(conn, &job.sub.endpoint).map_err(|e| (false, e.message))?;
    let agent = ureq::AgentBuilder::new()
        .redirects(0)
        .timeout(std::time::Duration::from_secs(4))
        .build();
    match agent.post(&job.sub.endpoint).set("Authorization",&authorization).set("TTL","300").set("Urgency","normal").set("Content-Encoding","aes128gcm").set("Content-Type","application/octet-stream").send_bytes(&body){
        Ok(_)=>Ok(()),Err(ureq::Error::Status(status,_))=>Err((status==404||status==410,format!("Service push : erreur {status}. Réactivez les notifications sur l’iPhone si nécessaire."))),Err(_)=>Err((false,"Service push injoignable. L’alerte reste dans Notifications.".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    /// GROKBOT_FAKE_PUSH and BUSY are process-wide: one fake-push scenario at a time.
    #[cfg(debug_assertions)]
    static FAKE_PUSH: std::sync::Mutex<()> = std::sync::Mutex::new(());
    #[test]
    fn hkdf_matches_rfc5869_sha256_test_case_one() {
        let output = derive(
            &(0u8..=12).collect::<Vec<_>>(),
            &[0x0b; 22],
            &(0xf0u8..=0xf9).collect::<Vec<_>>(),
            32,
        );
        assert_eq!(
            output
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf"
        );
    }
    #[test]
    fn rfc8291_envelope_can_be_decrypted_by_recipient() {
        let private =
            agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &SystemRandom::new())
                .unwrap();
        let public = private.compute_public_key().unwrap();
        let auth = [7; 16];
        let sub = Subscription {
            endpoint: "https://web.push.apple.com/test".into(),
            expiration: None,
            keys: Keys {
                p256dh: B64.encode(public.as_ref()),
                auth: B64.encode(auth),
            },
        };
        let packet = encrypt(&sub, b"bonjour").unwrap();
        assert_eq!(&packet[16..20], &4096u32.to_be_bytes());
        assert_eq!(packet[20], 65);
        let mut info = b"WebPush: info\0".to_vec();
        info.extend_from_slice(public.as_ref());
        info.extend_from_slice(&packet[21..86]);
        let ikm = agreement::agree_ephemeral(
            private,
            &agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, &packet[21..86]),
            |shared| derive(&auth, shared, &info, 32),
        )
        .unwrap();
        let cek = derive(&packet[..16], &ikm, b"Content-Encoding: aes128gcm\0", 16);
        let nonce = derive(&packet[..16], &ikm, b"Content-Encoding: nonce\0", 12);
        let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &cek).unwrap());
        let mut ciphertext = packet[86..].to_vec();
        let plain = key
            .open_in_place(
                aead::Nonce::try_assume_unique_for_key(&nonce).unwrap(),
                aead::Aad::empty(),
                &mut ciphertext,
            )
            .unwrap();
        assert_eq!(plain, b"bonjour\x02");
    }
    #[test]
    fn vapid_is_signed_and_persistent() {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        let before = public_key(&conn).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(before, public_key(&conn).unwrap());
        let header = vapid(&conn, "https://web.push.apple.com/path").unwrap();
        let token = header
            .strip_prefix("vapid t=")
            .unwrap()
            .split(',')
            .next()
            .unwrap();
        let pieces: Vec<_> = token.split('.').collect();
        let signature = B64.decode(pieces[2]).unwrap();
        signature::UnparsedPublicKey::new(
            &signature::ECDSA_P256_SHA256_FIXED,
            B64.decode(before).unwrap(),
        )
        .verify(
            format!("{}.{}", pieces[0], pieces[1]).as_bytes(),
            &signature,
        )
        .unwrap();
        let claims: Value = serde_json::from_slice(&B64.decode(pieces[1]).unwrap()).unwrap();
        assert_eq!(claims["aud"], "https://web.push.apple.com");
    }
    #[cfg(debug_assertions)]
    #[test]
    fn debug_never_sends_to_a_real_push_service() {
        assert!(!endpoint_allowed("https://web.push.apple.com/path"));
        assert!(!endpoint_allowed("http://169.254.169.254/latest"));
    }
    /// Opens an RFC 8291 packet the way the phone does.
    #[cfg(debug_assertions)]
    fn open_packet(private: agreement::EphemeralPrivateKey, auth: &[u8], packet: &[u8]) -> Vec<u8> {
        let public = private.compute_public_key().unwrap();
        let mut info = b"WebPush: info\0".to_vec();
        info.extend_from_slice(public.as_ref());
        info.extend_from_slice(&packet[21..86]);
        let ikm = agreement::agree_ephemeral(
            private,
            &agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, &packet[21..86]),
            |shared| derive(auth, shared, &info, 32),
        )
        .unwrap();
        let cek = derive(&packet[..16], &ikm, b"Content-Encoding: aes128gcm\0", 16);
        let nonce = derive(&packet[..16], &ikm, b"Content-Encoding: nonce\0", 12);
        let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &cek).unwrap());
        let mut ciphertext = packet[86..].to_vec();
        let plain = key
            .open_in_place(
                aead::Nonce::try_assume_unique_for_key(&nonce).unwrap(),
                aead::Aad::empty(),
                &mut ciphertext,
            )
            .unwrap();
        plain[..plain.len() - 1].to_vec()
    }
    #[cfg(debug_assertions)]
    #[test]
    fn reply_push_is_titled_with_the_bot_and_carries_no_reply_text() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            time::{Duration, Instant},
        };
        let _serial = FAKE_PUSH.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let previous = std::env::var_os("GROKBOT_FAKE_PUSH");
        std::env::set_var("GROKBOT_FAKE_PUSH", &base);
        struct Restore(Option<std::ffi::OsString>);
        impl Drop for Restore {
            fn drop(&mut self) {
                if let Some(value) = &self.0 {
                    std::env::set_var("GROKBOT_FAKE_PUSH", value)
                } else {
                    std::env::remove_var("GROKBOT_FAKE_PUSH")
                }
            }
        }
        let _restore = Restore(previous);
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                if let Ok((stream, _)) = listener.accept() {
                    break stream;
                }
                assert!(Instant::now() < deadline, "fake push timeout");
                std::thread::sleep(Duration::from_millis(10));
            };
            stream.set_nonblocking(false).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut data = vec![];
            let mut buf = [0; 4096];
            let body = loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                data.extend_from_slice(&buf[..n]);
                if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                    let length: usize = String::from_utf8_lossy(&data[..at])
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                                .map(|(_, v)| v.trim().parse().unwrap())
                        })
                        .unwrap();
                    if data.len() >= at + 4 + length {
                        break data[at + 4..at + 4 + length].to_vec();
                    }
                }
            };
            write!(
                stream,
                "HTTP/1.1 201 Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            )
            .unwrap();
            body
        });
        let root = grokbot_test_support::TestDir::new("iphone-reply-push");
        let mut app = App::open(&root.join("workspace.sqlite")).unwrap();
        app.api(
            "POST",
            "/api/bots",
            br#"{"id":"atlas","name":"Atlas","color":"cyan"}"#,
        )
        .unwrap();
        let conversation = app
            .api("POST", "/api/conversations", br#"{"bot_id":"atlas"}"#)
            .unwrap()["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let private =
            agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &SystemRandom::new())
                .unwrap();
        let public = private.compute_public_key().unwrap();
        let ctx = app.feature_context();
        let conn = ctx.connection;
        conn.execute("UPDATE iphone_settings SET enabled=1", [])
            .unwrap();
        conn.execute("UPDATE wrapper_notification_settings SET settings=json_set(settings,'$.enabled',json('true'))",[]).unwrap();
        conn.execute("INSERT INTO iphone_devices(id,name,credential_hash,receipt_hash,status,expires,created_at) VALUES('device','iPhone','hash','receipt','active',0,0)",[]).unwrap();
        subscribe(conn,"device",&json!({"subscription":{"endpoint":format!("{base}/push"),"expirationTime":null,"keys":{"p256dh":B64.encode(public.as_ref()),"auth":B64.encode([9;16])}}}).to_string().into_bytes()).unwrap();
        conn.execute("INSERT INTO wrapper_notifications VALUES('local-workspace','reply:run-1','reply',?1,'reply-run-1','Réponse terminée',?2,1,0)",params![conversation,crate::now()]).unwrap();
        tick(&ctx).unwrap();
        let packet = server.join().unwrap();
        while BUSY.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let payload: Value =
            serde_json::from_slice(&open_packet(private, &[9; 16], &packet)).unwrap();
        assert_eq!(
            payload,
            json!({"title":"Atlas","kind":"reply","id":"reply:run-1","conversation_id":conversation,"card_id":"reply-run-1"})
        );
        drop(app);
    }
    #[cfg(debug_assertions)]
    #[test]
    fn fake_push_policy_claims_restart_and_expired_subscription() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
            time::{Duration, Instant},
        };
        let _serial = FAKE_PUSH.lock().unwrap_or_else(|e| e.into_inner());
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let previous = std::env::var_os("GROKBOT_FAKE_PUSH");
        std::env::set_var("GROKBOT_FAKE_PUSH", &base);
        struct Restore(Option<std::ffi::OsString>);
        impl Drop for Restore {
            fn drop(&mut self) {
                if let Some(value) = &self.0 {
                    std::env::set_var("GROKBOT_FAKE_PUSH", value)
                } else {
                    std::env::remove_var("GROKBOT_FAKE_PUSH")
                }
            }
        }
        let _restore = Restore(previous);
        let (tx, rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            for status in [201, 410] {
                let mut stream = loop {
                    if let Ok((stream, _)) = listener.accept() {
                        break stream;
                    }
                    assert!(Instant::now() < deadline, "fake push timeout");
                    std::thread::sleep(Duration::from_millis(10));
                };
                // Darwin can inherit O_NONBLOCK from the accepting listener.
                // Read the complete fake HTTP request with a bounded timeout.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut data = vec![];
                let mut buf = [0; 4096];
                loop {
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    data.extend_from_slice(&buf[..n]);
                    if let Some(at) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&data[..at]);
                        let length: usize = headers
                            .lines()
                            .find_map(|line| {
                                line.split_once(':')
                                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                                    .map(|(_, v)| v.trim().parse().unwrap())
                            })
                            .unwrap();
                        if data.len() >= at + 4 + length {
                            assert!(headers
                                .to_ascii_lowercase()
                                .contains("content-encoding: aes128gcm"));
                            assert!(headers.contains("vapid t="));
                            break;
                        }
                    }
                }
                write!(
                    stream,
                    "HTTP/1.1 {status} Test\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
                tx.send(()).unwrap();
            }
        });
        let root = grokbot_test_support::TestDir::new("iphone-push");
        let path = root.join("workspace.sqlite");
        let mut app = App::open(&path).unwrap();
        let conversation = app.api("POST", "/api/conversations", b"{}").unwrap()["conversation"]
            ["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let public =
            agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &SystemRandom::new())
                .unwrap()
                .compute_public_key()
                .unwrap();
        let ctx = app.feature_context();
        let conn = ctx.connection;
        conn.execute("UPDATE iphone_settings SET enabled=1", [])
            .unwrap();
        conn.execute("INSERT INTO iphone_devices(id,name,credential_hash,receipt_hash,status,expires,created_at) VALUES('device','iPhone','hash','receipt','active',0,0)",[]).unwrap();
        subscribe(conn,"device",&json!({"subscription":{"endpoint":format!("{base}/push"),"expirationTime":null,"keys":{"p256dh":B64.encode(public.as_ref()),"auth":B64.encode([8;16])}}}).to_string().into_bytes()).unwrap();
        let now = crate::now();
        let add = |id: &str, kind: &str| {
            conn.execute("INSERT INTO wrapper_notifications VALUES('local-workspace',?1,?2,?3,'card','Événement',?4,1,0)",params![id,kind,conversation,now]).unwrap();
            conn.execute("UPDATE iphone_settings SET push_tick=0", [])
                .unwrap();
        };
        conn.execute("UPDATE wrapper_notification_settings SET settings=json_set(settings,'$.enabled',json('true'),'$.reply',json('false'))",[]).unwrap();
        add("muted", "reply");
        tick(&ctx).unwrap();
        conn.execute("UPDATE wrapper_notification_settings SET settings=json_set(settings,'$.reply',json('true'),'$.dnd',json('true'))",[]).unwrap();
        add("dnd", "routine");
        tick(&ctx).unwrap();
        conn.execute(
            "UPDATE wrapper_notification_settings SET settings=json_set(settings,'$.dnd',json('false'))",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE iphone_devices SET visible=1,conversation_id=?1,last_seen=?2",
            params![conversation, now],
        )
        .unwrap();
        add("open", "reply");
        tick(&ctx).unwrap();
        assert!(rx.try_recv().is_err());
        conn.execute(
            "UPDATE iphone_devices SET visible=1,conversation_id=NULL",
            [],
        )
        .unwrap();
        add("sent", "reply");
        tick(&ctx).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        while BUSY.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            conn.query_row("SELECT count(*) FROM iphone_push_claims", [], |r| r
                .get::<_, u64>(0))
                .unwrap(),
            4
        );
        // Persisted claims prevent resend after the Mac restarts.
        drop(app);
        let app = App::open(&path).unwrap();
        let ctx = app.feature_context();
        let conn = ctx.connection;
        conn.execute("UPDATE iphone_settings SET push_tick=0", [])
            .unwrap();
        tick(&ctx).unwrap();
        assert!(rx.try_recv().is_err());
        conn.execute("INSERT INTO wrapper_notifications VALUES('local-workspace','gone','reply',?1,'card','Événement',?2,1,0)",params![conversation,now]).unwrap();
        conn.execute("UPDATE iphone_settings SET push_tick=0", [])
            .unwrap();
        tick(&ctx).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).unwrap();
        while BUSY.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(10));
        }
        let (cleared, error): (bool, String) = conn
            .query_row(
                "SELECT subscription IS NULL,push_error FROM iphone_devices WHERE id='device'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert!(cleared);
        assert!(error.contains("410"));
        server.join().unwrap();
        drop(app);
    }
}
