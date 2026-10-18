//! Housekeeping safety across every durable reference carrier and restart.
use base64::{engine::general_purpose::STANDARD, Engine};
use grokbot_attachments::{documents, staging};
use grokbot_memory::facts::{self, Fact, FactStatus, Provenance, Scope, Source};
use grokbot_wrapper::{features::storage, App};
use rusqlite::{params, Connection};
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, SystemTime},
};

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x01\0\0\0\x01storage";
struct Fixture {
    app: App,
    path: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "ezerbot-storage-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("workspace.sqlite");
        Self {
            app: App::open(&path).unwrap(),
            path,
        }
    }
    fn db(&self) -> Connection {
        let c = Connection::open(&self.path).unwrap();
        c.busy_timeout(Duration::from_secs(3)).unwrap();
        c
    }
    fn post(&mut self, path: &str, data: Value) -> Value {
        self.app
            .api("POST", path, &serde_json::to_vec(&data).unwrap())
            .unwrap()
    }
    fn info(&mut self) -> Value {
        self.app.api("GET", "/api/storage", b"").unwrap()
    }
    fn image(&mut self, suffix: u8) -> String {
        self.app
            .api("POST", "/api/attachments", &[PNG, &[suffix]].concat())
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn document(&mut self, name: &str) -> String {
        self.post(
            "/api/documents",
            json!({"name":name,"data":STANDARD.encode(format!("Document {name}"))}),
        )["id"]
            .as_str()
            .unwrap()
            .into()
    }
    fn preview(&mut self) -> Value {
        self.post("/api/storage/preview", json!({}))
    }
    fn clean(&mut self, preview: &Value) -> Value {
        self.post(
            "/api/storage/clean",
            json!({"token":preview["token"],"confirm":true}),
        )
    }
    fn blob(&self, id: &str) -> PathBuf {
        self.path
            .with_extension("sqlite.attachments")
            .join(id.strip_prefix("attachment_").unwrap())
    }
    fn restart(&mut self) {
        self.app = App::open(&self.path).unwrap();
    }
    fn fact(&self, bot: &str, scope: Scope, text: &str) -> Fact {
        facts::remember(
            &self.db(),
            &Fact {
                fact_id: String::new(),
                bot_id: bot.into(),
                scope,
                text: text.into(),
                provenance: Provenance {
                    conversation_id: None,
                    entry_id: None,
                    source: Source::UserStated,
                },
                learned_at: 1,
                status: FactStatus::Active,
                sensitive: false,
            },
        )
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.path.parent().unwrap());
    }
}

#[test]
fn dry_run_refusal_confirmation_usage_and_audit_survive_restart() {
    let mut f = Fixture::new();
    let image = f.image(1);
    let document = f.document("orphelin.txt");
    let info = f.info();
    assert_eq!(info["usage"]["image_count"], 1);
    assert_eq!(info["usage"]["document_count"], 1);
    assert!(info["usage"]["documents"].as_u64().unwrap() > 0);
    let plan = f.preview();
    assert_eq!(plan["count"], 2);
    assert!(plan["bytes"].as_u64().unwrap() > PNG.len() as u64);
    assert!(f.blob(&image).exists());
    let error = f
        .app
        .api(
            "POST",
            "/api/storage/clean",
            &serde_json::to_vec(&json!({"token":plan["token"],"confirm":false})).unwrap(),
        )
        .unwrap_err();
    assert_eq!(error.status, 403);
    assert!(f.blob(&image).exists());
    f.restart();
    assert_eq!(f.clean(&plan)["deleted"], 2);
    assert!(!f.blob(&image).exists());
    assert!(staging::get(&f.db(), &image).unwrap().is_none());
    assert!(documents::get(&f.db(), &document).unwrap().is_none());
    f.restart();
    assert_eq!(f.preview()["count"], 0);
    let count:i64=f.db().query_row("SELECT COUNT(*) FROM wrapper_storage_deletions WHERE state='deleted' AND reason='user_confirmed'",[],|r|r.get(0)).unwrap();
    assert_eq!(count, 2);
    assert!(f
        .app
        .api(
            "POST",
            "/api/storage/clean",
            &serde_json::to_vec(&json!({"token":plan["token"],"confirm":true})).unwrap()
        )
        .is_err());
}

#[test]
fn every_reference_path_is_retained_including_skill_history_and_delegated_children() {
    let mut f = Fixture::new();
    let mut images = Vec::new();
    for i in 0..8 {
        images.push(f.image(i));
    }
    let db = f.db();
    // All message links remain roots, even for deleted/reset/regenerated threads.
    staging::link_attachments(&db, "deleted-thread", "old-message", &images[0..1]).unwrap();
    // Durable drafts: no age cutoff, including a restored UI session.
    f.post(
        "/api/storage/drafts",
        json!({"draft_id":"draft-thread","attachments":[images[1]]}),
    );
    // Mixed-family order index, retained regardless of main-thread projection.
    db.execute(
        "INSERT INTO wrapper_attachment_order VALUES ('task-child','follow-up',?1,0)",
        [&images[2]],
    )
    .unwrap();
    // Opaque child snapshots and arbitrarily nested future reference carriers.
    db.execute_batch("CREATE TABLE test_delegated_children (snapshot TEXT); CREATE TABLE test_regenerate_history (snapshot TEXT)").unwrap();
    db.execute(
        "INSERT INTO test_delegated_children VALUES (?1)",
        [json!({"parent":{"selected":[{"attachment_id":images[3]}]}}).to_string()],
    )
    .unwrap();
    db.execute(
        "INSERT INTO test_regenerate_history VALUES (?1)",
        [json!({"previous_run":images[4]}).to_string()],
    )
    .unwrap();
    // Real skill create + version snapshot: current and historical URLs protect.
    let skill=f.post("/api/skills",json!({"bot_id":"grokbot","name":"Photo","description":"Guide","body":format!("![photo](/api/attachments/{})",images[5])}));
    let skill_id = skill["skill"]["skill_id"].as_str().expect("skill id");
    db.execute(
        "UPDATE skills_library SET body=?1,version=version+1 WHERE skill_id=?2",
        params![
            format!("![suite](/api/attachments/{})", images[6]),
            skill_id
        ],
    )
    .unwrap();
    // A bare SHA path is a reference too, not just the public attachment ID.
    db.execute(
        "INSERT INTO test_delegated_children VALUES (?1)",
        [format!(
            "workspace.sqlite.attachments/{}",
            images[7].strip_prefix("attachment_").unwrap()
        )],
    )
    .unwrap();
    let doc = f.document("enfant.txt");
    documents::link(&db, "child-task", "input", std::slice::from_ref(&doc)).unwrap();
    db.execute("UPDATE attachments_staged SET staged_at=0", [])
        .unwrap();
    db.execute("UPDATE wrapper_storage_staged SET staged_at=0", [])
        .unwrap();
    let extra = f.image(99);
    f.restart();
    let plan = f.preview();
    assert_eq!(plan["count"], 1, "{plan}");
    f.clean(&plan);
    assert!(!f.blob(&extra).exists());
    for image in images {
        assert!(staging::read(&f.db(), &f.path, &image).is_ok(), "{image}");
    }
    assert!(documents::get(&db, &doc).unwrap().is_some());
}

#[test]
fn markdown_links_inside_document_extractions_are_roots_and_corruption_fails_closed() {
    let mut f = Fixture::new();
    let image = f.image(7);
    let document=f.post("/api/documents",json!({"name":"lien.md","data":STANDARD.encode(format!("![photo](/api/attachments/{image})"))}));
    // The orphan document itself is removable, but its outgoing link remains
    // conservative for this sweep; no live document can lose its linked image.
    let plan = f.preview();
    assert_eq!(plan["count"], 1);
    f.clean(&plan);
    assert!(f.blob(&image).exists());
    let record = json!({"id":document["id"],"text":"bad"}).to_string();
    f.db()
        .execute(
            "INSERT INTO attachments_documents VALUES ('broken',?1)",
            [record],
        )
        .unwrap();
    // A non-object extraction cannot safely yield its complete reference set.
    f.db()
        .execute(
            "UPDATE attachments_documents SET record='[]' WHERE id='broken'",
            [],
        )
        .unwrap();
    assert_eq!(
        f.app
            .api("POST", "/api/storage/preview", b"{}")
            .unwrap_err()
            .status,
        409
    );
    assert!(f.blob(&image).exists());
}

#[test]
fn reference_added_after_preview_wins_and_new_orphans_are_not_silently_added() {
    let mut f = Fixture::new();
    let image = f.image(1);
    let plan = f.preview();
    f.post(
        "/api/storage/drafts",
        json!({"draft_id":"draft","attachments":[image]}),
    );
    let later = f.image(2);
    let output = f.clean(&plan);
    assert_eq!(output["deleted"], 0);
    assert_eq!(output["retained"], 1);
    assert!(f.blob(&image).exists());
    assert!(f.blob(&later).exists());
}

#[test]
fn startup_only_removes_unreferenced_staging_older_than_24_hours() {
    let mut f = Fixture::new();
    let old = f.image(1);
    let fresh = f.image(2);
    let sent = f.image(3);
    let draft = f.image(4);
    let doc = f.document("ancien.txt");
    let recent_doc = f.document("recent.txt");
    let db = f.db();
    for id in [&old, &sent, &draft] {
        db.execute(
            "UPDATE attachments_staged SET staged_at=0 WHERE id=?1",
            [id],
        )
        .unwrap();
    }
    staging::link_attachments(&db, "thread", "sent", std::slice::from_ref(&sent)).unwrap();
    f.post(
        "/api/storage/drafts",
        json!({"draft_id":"thread","attachments":[draft]}),
    );
    db.execute(
        "UPDATE wrapper_storage_staged SET staged_at=0 WHERE id=?1",
        [&doc],
    )
    .unwrap();
    // Caption caches/uses are not human references; abandoned preparation cleans too.
    db.execute(
        "INSERT INTO attachments_caption_uses VALUES ('thread',?1,'model','ctx',0)",
        [old.strip_prefix("attachment_").unwrap()],
    )
    .unwrap();
    f.restart();
    assert!(!f.blob(&old).exists());
    assert!(f.blob(&fresh).exists());
    assert!(f.blob(&sent).exists());
    assert!(f.blob(&draft).exists());
    assert!(documents::get(&db, &doc).unwrap().is_none());
    assert!(documents::get(&db, &recent_doc).unwrap().is_some());
    assert_eq!(db.query_row("SELECT COUNT(*) FROM wrapper_storage_deletions WHERE reason='startup_24h' AND state='deleted'",[],|r|r.get::<_,i64>(0)).unwrap(),2);
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(state["storage"]["drafts"]["thread"][0]["id"], draft);
}

#[test]
fn atomic_upload_draft_survives_restart_then_explicit_removal_allows_cleanup() {
    let mut f = Fixture::new();
    let image = f
        .app
        .api("POST", "/api/attachments?draft_id=thread", PNG)
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let doc = f.post(
        "/api/documents",
        json!({"name":"brouillon.txt","data":STANDARD.encode("Brouillon"),"draft_id":"thread"}),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    f.restart();
    assert_eq!(f.preview()["count"], 0);
    f.post(
        "/api/storage/drafts",
        json!({"draft_id":"thread","attachments":[],"removed":[image,doc]}),
    );
    let plan = f.preview();
    assert_eq!(plan["count"], 2);
    f.clean(&plan);
}

#[test]
fn removing_an_inflight_upload_wins_before_or_after_publication() {
    for cancel_first in [true, false] {
        let mut f = Fixture::new();
        let cancel =
            json!({"draft_id":"thread","attachments":[],"cancelled_uploads":["upload-one"]});
        if cancel_first {
            f.post("/api/storage/drafts", cancel.clone());
        }
        let file = f
            .app
            .api(
                "POST",
                "/api/attachments?draft_id=thread&upload_id=upload-one",
                PNG,
            )
            .unwrap();
        if !cancel_first {
            f.post("/api/storage/drafts", cancel);
        }
        f.restart();
        let state = f.app.api("GET", "/api/state", b"").unwrap();
        assert!(state["storage"]["drafts"].as_object().unwrap().is_empty());
        let plan = f.preview();
        assert_eq!(plan["count"], 1);
        f.clean(&plan);
        assert!(!f.blob(file["id"].as_str().unwrap()).exists());
    }
}

#[test]
fn cancelled_document_upload_and_duplicate_selection_never_resurrect_or_drop_a_draft() {
    let mut f = Fixture::new();
    f.post(
        "/api/storage/drafts",
        json!({"draft_id":"thread","attachments":[],"cancelled_uploads":["doc-upload"]}),
    );
    f.post("/api/documents",json!({"name":"annule.txt","data":STANDARD.encode("Annulation"),"draft_id":"thread","upload_id":"doc-upload"}));
    let image = f
        .app
        .api(
            "POST",
            "/api/attachments?draft_id=thread&upload_id=first",
            PNG,
        )
        .unwrap();
    f.app
        .api(
            "POST",
            "/api/attachments?draft_id=thread&upload_id=duplicate",
            PNG,
        )
        .unwrap();
    f.post(
        "/api/storage/drafts",
        json!({"draft_id":"thread","attachments":[],"cancelled_uploads":["duplicate"]}),
    );
    assert_eq!(
        f.preview()["count"],
        1,
        "only the cancelled document is disposable"
    );
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(state["storage"]["drafts"]["thread"][0]["id"], image["id"]);
    f.post(
        "/api/storage/drafts",
        json!({"draft_id":"thread","attachments":[],"cancelled_uploads":["first"]}),
    );
    assert_eq!(f.preview()["count"], 2);
}

#[test]
fn interrupted_cleanup_intents_replay_and_restaging_cancels_old_intents() {
    let mut f = Fixture::new();
    let image = f.image(1);
    let retained = f.image(2);
    let db = f.db();
    // Exact durable state after intent commit, before any filesystem unlink.
    db.execute("DELETE FROM attachments_staged WHERE id=?1", [&image])
        .unwrap();
    for id in [&image, &retained] {
        db.execute("INSERT INTO wrapper_storage_deletions(id,kind,bytes,reason,state,created_at) VALUES (?1,'image',1,'user_confirmed','pending',0)",[id]).unwrap();
    }
    f.restart();
    assert!(!f.blob(&image).exists());
    assert!(f.blob(&retained).exists());
    assert!(staging::read(&db, &f.path, &retained).is_ok());
    assert_eq!(
        db.query_row(
            "SELECT state FROM wrapper_storage_deletions WHERE id=?1",
            [&retained],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "retained"
    );
    // Crash after unlink but before marking completion is equally replayable.
    db.execute(
        "UPDATE wrapper_storage_deletions SET state='pending' WHERE id=?1",
        [&image],
    )
    .unwrap();
    f.restart();
    assert_eq!(f.preview()["count"], 1);
}

#[test]
fn failure_to_commit_intent_never_unlinks_anything() {
    let mut f = Fixture::new();
    let image = f.image(1);
    let doc = f.document("erreur.txt");
    let plan = f.preview();
    f.db().execute_batch("CREATE TRIGGER fail_cleanup BEFORE INSERT ON wrapper_storage_deletions BEGIN SELECT RAISE(ABORT,'forced failure'); END;").unwrap();
    assert!(f
        .app
        .api(
            "POST",
            "/api/storage/clean",
            &serde_json::to_vec(&json!({"token":plan["token"],"confirm":true})).unwrap()
        )
        .is_err());
    assert!(f.blob(&image).exists());
    assert!(staging::get(&f.db(), &image).unwrap().is_some());
    assert!(documents::get(&f.db(), &doc).unwrap().is_some());
    f.db().execute_batch("DROP TRIGGER fail_cleanup").unwrap();
    assert_eq!(f.clean(&plan)["deleted"], 2);
}

#[test]
fn symlinks_unknown_files_and_external_directory_are_never_touched() {
    use std::os::unix::fs::symlink;
    let mut f = Fixture::new();
    let image = f.image(1);
    let dir = f.blob(&image).parent().unwrap().to_owned();
    let outside = f.path.parent().unwrap().join("outside");
    fs::create_dir(&outside).unwrap();
    let external = outside.join("precieux");
    fs::write(&external, b"keep").unwrap();
    let fake = "a".repeat(64);
    symlink(&external, dir.join(&fake)).unwrap();
    fs::write(dir.join("unknown.txt"), b"keep").unwrap();
    let plan = f.preview();
    assert_eq!(plan["count"], 1);
    f.clean(&plan);
    assert_eq!(fs::read(&external).unwrap(), b"keep");
    assert!(dir.join("unknown.txt").exists());
    fs::rename(&dir, f.path.parent().unwrap().join("saved-attachments")).unwrap();
    symlink(&outside, &dir).unwrap();
    assert_eq!(
        f.app.api("GET", "/api/storage", b"").unwrap_err().status,
        409
    );
    storage::startup(&f.db(), &f.path, i64::MAX).unwrap_err();
    assert_eq!(fs::read(&external).unwrap(), b"keep");
}

#[test]
fn old_rolled_back_blobs_and_temporaries_clean_but_fresh_and_unknown_files_stay() {
    let mut f = Fixture::new();
    let image = f.image(1);
    let db = f.db();
    db.execute("DELETE FROM attachments_staged WHERE id=?1", [&image])
        .unwrap();
    let path = f.blob(&image);
    let dir = path.parent().unwrap();
    let temp = dir.join(format!(".{}.tmp", "b".repeat(64)));
    let fresh = dir.join(format!(".{}.tmp", "c".repeat(64)));
    for p in [&temp, &fresh] {
        fs::write(p, b"temporary").unwrap();
    }
    for p in [&path, &temp] {
        fs::File::open(p)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(86401)),
            )
            .unwrap();
    }
    f.restart();
    assert!(!path.exists());
    assert!(!temp.exists());
    assert!(fresh.exists());
}

#[test]
fn deleted_bot_private_facts_have_confirmation_redaction_and_scope_guards() {
    let mut f = Fixture::new();
    let bot=f.post("/api/bots",json!({"id":"removed","name":"Ancien","color":"citron","shape":"galet","description":"","instructions":""}));
    assert!(bot["bot"].is_object());
    let private = f.fact("removed", Scope::Private, "Un fait à purger");
    let team = f.fact("removed", Scope::Team, "Un fait partagé");
    let live = f.fact("grokbot", Scope::Private, "Fait à garder");
    f.post("/api/bots/delete", json!({"id":"removed"}));
    let info = f.info();
    assert_eq!(info["orphan_facts"].as_array().unwrap().len(), 1);
    assert_eq!(info["orphan_facts"][0]["text"], private.text);
    let deny = f
        .app
        .api(
            "POST",
            "/api/storage/facts/delete",
            &serde_json::to_vec(&json!({"fact_id":private.fact_id,"confirm":false})).unwrap(),
        )
        .unwrap_err();
    assert_eq!(deny.status, 403);
    for id in [&team.fact_id, &live.fact_id] {
        assert!(f
            .app
            .api(
                "POST",
                "/api/storage/facts/delete",
                &serde_json::to_vec(&json!({"fact_id":id,"confirm":true})).unwrap()
            )
            .is_err());
    }
    f.post(
        "/api/storage/facts/delete",
        json!({"fact_id":private.fact_id,"confirm":true}),
    );
    f.restart();
    assert!(f.info()["orphan_facts"].as_array().unwrap().is_empty());
    assert_eq!(
        facts::get(&f.db(), &private.fact_id).unwrap().unwrap().text,
        ""
    );
    assert_eq!(
        facts::get(&f.db(), &team.fact_id).unwrap().unwrap().text,
        team.text
    );
    assert_eq!(
        facts::get(&f.db(), &live.fact_id).unwrap().unwrap().text,
        live.text
    );
}

#[test]
fn purging_old_private_versions_does_not_erase_their_shared_successor() {
    let mut f = Fixture::new();
    let old = f.fact("deleted-owner", Scope::Private, "Ancienne préférence");
    let latest = facts::correct(
        &f.db(),
        "deleted-owner",
        &old.fact_id,
        "Préférence partagée",
        &old.provenance,
        2,
    )
    .unwrap();
    grokbot_memory::promotion::promote(
        &f.db(),
        "deleted-owner",
        std::slice::from_ref(&latest.fact_id),
    )
    .unwrap();
    let rows = f.info();
    assert_eq!(rows["orphan_facts"].as_array().unwrap().len(), 1);
    f.post(
        "/api/storage/facts/delete",
        json!({"fact_id":old.fact_id,"confirm":true}),
    );
    let shared = facts::get(&f.db(), &latest.fact_id).unwrap().unwrap();
    assert_eq!(shared.scope, Scope::Team);
    assert_eq!(shared.text, "Préférence partagée");
    assert!(facts::get(&f.db(), &old.fact_id)
        .unwrap()
        .unwrap()
        .text
        .is_empty());
}

#[test]
fn private_purge_refuses_busy_runs_and_rolls_back_storage_errors() {
    use grokbot_engine::{
        contracts::{RunBudgets, RunKind, Trigger, TurnEnd},
        model::{self, Run, RunStatus},
    };
    let mut f = Fixture::new();
    let fact = f.fact(
        "deleted-owner",
        Scope::Private,
        "Ne pas effacer pendant une réponse",
    );
    let db = f.db();
    let tx = grokbot_wrapper::features::write_transaction(&db).unwrap();
    model::start(
        &tx,
        &Run {
            run_id: "busy".into(),
            bot_id: "deleted-owner".into(),
            conversation_id: "thread".into(),
            trigger: Trigger::UserMessage {
                message_id: "thread/message".into(),
                attempt: 0,
            },
            kind: RunKind::Chat,
            status: RunStatus::Queued,
            task_status: None,
            turn_end: None,
            budgets: RunBudgets::default(),
            started_at: 1,
        },
    )
    .unwrap();
    model::set_running_at(&tx, "busy", 1).unwrap();
    tx.commit().unwrap();
    let input = json!({"fact_id":fact.fact_id,"confirm":true}).to_string();
    assert_eq!(
        f.app
            .api("POST", "/api/storage/facts/delete", input.as_bytes())
            .unwrap_err()
            .status,
        409
    );
    assert_eq!(
        facts::get(&db, &fact.fact_id).unwrap().unwrap().text,
        fact.text
    );
    let tx = grokbot_wrapper::features::write_transaction(&db).unwrap();
    model::cancel(&tx, "busy", TurnEnd::Stopped).unwrap();
    tx.commit().unwrap();
    db.execute_batch("CREATE TRIGGER reject_purge BEFORE UPDATE ON memory_facts BEGIN SELECT RAISE(ABORT,'forced failure'); END;").unwrap();
    assert!(f
        .app
        .api("POST", "/api/storage/facts/delete", input.as_bytes())
        .is_err());
    assert_eq!(
        facts::get(&db, &fact.fact_id).unwrap().unwrap().text,
        fact.text
    );
    assert_eq!(
        db.query_row(
            "SELECT COUNT(*) FROM wrapper_storage_deletions WHERE kind='fact'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    db.execute_batch("DROP TRIGGER reject_purge").unwrap();
    f.post(
        "/api/storage/facts/delete",
        json!({"fact_id":fact.fact_id,"confirm":true}),
    );
    assert!(facts::get(&db, &fact.fact_id)
        .unwrap()
        .unwrap()
        .text
        .is_empty());
}

#[test]
fn draft_uploads_and_housekeeping_keep_the_local_request_gate() {
    use grokbot_wrapper::{body_limit, validate_headers_for_path, ATTACHMENT_BODY_LIMIT};
    let mut headers = vec![
        ("Host".into(), "127.0.0.1:1234".into()),
        ("Content-Type".into(), "image/png".into()),
    ];
    assert!(
        validate_headers_for_path(1234, "POST", "/api/attachments?draft_id=thread", &headers)
            .is_err()
    );
    headers.push(("X-Grokbot-Request".into(), "local".into()));
    assert!(
        validate_headers_for_path(1234, "POST", "/api/attachments?draft_id=thread", &headers)
            .is_ok()
    );
    assert_eq!(
        body_limit("/api/attachments?draft_id=thread"),
        ATTACHMENT_BODY_LIMIT
    );
    assert_eq!(
        body_limit("/api/attachments?draft_id=task%3Achild%3Aparent"),
        ATTACHMENT_BODY_LIMIT
    );
    assert_eq!(
        body_limit("/api/attachments?x=1"),
        grokbot_wrapper::BODY_LIMIT
    );
    let mut f = Fixture::new();
    let file = f
        .app
        .api(
            "POST",
            "/api/attachments?draft_id=task%3Achild%3Aparent",
            PNG,
        )
        .unwrap();
    let state = f.app.api("GET", "/api/state", b"").unwrap();
    assert_eq!(
        state["storage"]["drafts"]["task:child:parent"][0]["id"],
        file["id"]
    );
    headers[1].1 = "application/json".into();
    headers.push(("Origin".into(), "https://outside.invalid".into()));
    for path in [
        "/api/storage/clean",
        "/api/storage/facts/delete",
        "/api/storage/drafts",
    ] {
        assert!(validate_headers_for_path(1234, "POST", path, &headers).is_err());
    }
}

#[test]
fn expired_preview_never_authorizes_deletion() {
    let mut f = Fixture::new();
    let image = f.image(1);
    let plan = f.preview();
    f.db()
        .execute("UPDATE wrapper_storage_previews SET created_at=0", [])
        .unwrap();
    assert_eq!(
        f.app
            .api(
                "POST",
                "/api/storage/clean",
                &serde_json::to_vec(&json!({"token":plan["token"],"confirm":true})).unwrap()
            )
            .unwrap_err()
            .status,
        409
    );
    assert!(f.blob(&image).exists());
}
