//! Mémoire d'un sujet : visible de tous ses fils, écrite seulement après l'accord de Charles.
use grokbot_engine::{
    compaction,
    contracts::{ContextSource, ToolOutcome},
};
use grokbot_wrapper::{ApiError, App};
use rusqlite::Connection;
use serde_json::{json, Value};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    app: App,
    path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "topic-memory-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("workspace.sqlite");
        let app = App::open(&path).unwrap();
        Self { app, path }
    }

    fn db(&self) -> Connection {
        Connection::open(&self.path).unwrap()
    }

    fn post(&mut self, path: &str, body: Value) -> Value {
        self.post_result(path, body).unwrap()
    }

    fn post_result(&mut self, path: &str, body: Value) -> Result<Value, ApiError> {
        self.app
            .api("POST", path, &serde_json::to_vec(&body).unwrap())
    }

    fn state(&mut self) -> Value {
        self.app.api("GET", "/api/state", b"").unwrap()
    }

    fn topic(&mut self, name: &str) -> String {
        self.post("/api/topics", json!({"name": name}))["topic"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    /// Un fil du bot par défaut, rangé dans `topic` quand il y en a un.
    fn thread(&mut self, title: &str, topic: Option<&str>) -> String {
        let id = self.post("/api/conversations", json!({"title": title}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        if let Some(topic) = topic {
            self.post(
                "/api/topics/assign",
                json!({"conversation_id": id, "topic_id": topic}),
            );
        }
        id
    }

    fn group(&mut self, title: &str, topic: &str) -> String {
        self.post(
            "/api/bots",
            json!({"id": "second-bot", "name": "Second", "color": "cyan", "instructions": "x"}),
        );
        let id = self.post(
            "/api/conversations",
            json!({"title": title, "bot_ids": ["grokbot", "second-bot"]}),
        )["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        self.post(
            "/api/topics/assign",
            json!({"conversation_id": id, "topic_id": topic}),
        );
        id
    }

    fn tool(&self, bot: &str, conversation: &str, name: &str, args: Value) -> ToolOutcome {
        self.app
            .invoke_tool_in(bot, Some(conversation), name, args)
            .unwrap()
    }

    /// Le bot propose ; rien n'est encore retenu. Renvoie l'identifiant de la carte.
    fn propose(&self, conversation: &str, text: &str) -> String {
        match self.tool(
            "grokbot",
            conversation,
            "propose_topic_memory",
            json!({"text": text}),
        ) {
            ToolOutcome::Done { output } => output["offer_id"].as_str().unwrap().to_owned(),
            ToolOutcome::Failed { reason } => panic!("proposition refusée : {reason}"),
            _ => panic!("issue inattendue"),
        }
    }

    fn accept(&mut self, offer: &str) -> Value {
        self.post("/api/topics/memory/accept", json!({"offer_id": offer}))["fact"].clone()
    }

    /// Ce que le modèle lit de la mémoire (bot, équipe, sujet) pour ce fil.
    fn memory(&self, conversation: &str) -> String {
        self.app
            .context_preview(conversation)
            .unwrap()
            .into_iter()
            .filter(|block| block.source == ContextSource::Memory)
            .map(|block| block.text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn recall(&self, conversation: &str, query: &str) -> String {
        match self.tool(
            "grokbot",
            conversation,
            "recall_memory",
            json!({"query": query, "limit": 10}),
        ) {
            ToolOutcome::Done { output } => output["facts"].to_string(),
            _ => panic!("recall_memory a échoué"),
        }
    }

    fn topic_facts(&mut self, topic: &str) -> Vec<Value> {
        self.state()["topics"]["memory"][topic]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    fn card(&mut self, conversation: &str, offer: &str) -> Value {
        let state = self.state();
        state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["id"] == conversation)
            .unwrap()["messages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["id"] == offer)
            .unwrap_or_else(|| panic!("pas de carte {offer} dans le fil"))
            .clone()
    }
}

const FACT: &str = "Le camion est réservé pour le 1er juillet";

#[test]
fn un_fait_de_sujet_cree_dans_le_fil_a_est_utilise_dans_le_fil_b() {
    let mut f = Fixture::new();
    let topic = f.topic("Déménagement");
    let a = f.thread("Camion", Some(&topic));
    let b = f.thread("Adresse", Some(&topic));
    let group = f.group("Équipe", &topic);

    let offer = f.propose(&a, FACT);
    // La carte attend Charles : rien n'est lu, nulle part.
    let card = f.card(&a, &offer);
    assert_eq!(card["kind"], "topic_memory_offer");
    assert_eq!(card["payload"]["status"], "pending");
    assert_eq!(card["payload"]["text"], FACT);
    assert!(f.topic_facts(&topic).is_empty());
    assert!(!f.memory(&b).contains(FACT));

    let fact = f.accept(&offer);
    assert_eq!(fact["text"], FACT);
    assert_eq!(fact["conversation_id"], a.as_str());

    for conversation in [&a, &b, &group] {
        assert!(f.memory(conversation).contains(FACT), "{conversation}");
        assert!(f.recall(conversation, "camion").contains(FACT));
    }
    let facts = f.topic_facts(&topic);
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0]["conversation_title"], "Camion");

    // Après « Retenir », la carte devient « Retenu pour ce sujet ».
    let card = f.card(&a, &offer);
    assert_eq!(card["payload"]["status"], "accepted");
    assert_eq!(card["payload"]["topic_name"], "Déménagement");
    assert_eq!(card["payload"]["text"], FACT);
    // Accepter deux fois ne crée pas deux faits.
    f.accept(&offer);
    assert_eq!(f.topic_facts(&topic).len(), 1);
}

#[test]
fn le_fait_n_est_visible_ni_dans_un_autre_sujet_ni_dans_un_fil_sans_sujet() {
    let mut f = Fixture::new();
    let topic = f.topic("Déménagement");
    let other = f.topic("Impôts");
    let a = f.thread("Camion", Some(&topic));
    let elsewhere = f.thread("Déclaration", Some(&other));
    let loose = f.thread("Sans sujet", None);
    let offer = f.propose(&a, FACT);
    f.accept(&offer);

    assert!(f.memory(&a).contains(FACT));
    for conversation in [&elsewhere, &loose] {
        assert!(!f.memory(conversation).contains(FACT));
        assert!(!f.recall(conversation, "camion").contains(FACT));
    }
    assert!(f.topic_facts(&other).is_empty());

    // Changer le fil de sujet lui retire aussitôt la lecture.
    f.post(
        "/api/topics/assign",
        json!({"conversation_id": a, "topic_id": other}),
    );
    assert!(!f.memory(&a).contains(FACT));
    // Un fil sans sujet ne peut même pas proposer.
    match f.tool(
        "grokbot",
        &loose,
        "propose_topic_memory",
        json!({"text": "x"}),
    ) {
        ToolOutcome::Failed { reason } => assert!(reason.contains("aucun sujet")),
        _ => panic!("un fil sans sujet ne propose rien"),
    }
}

#[test]
fn la_memoire_privee_du_bot_n_est_pas_copiee_dans_le_sujet_sans_carte_confirmee() {
    let mut f = Fixture::new();
    let topic = f.topic("Déménagement");
    let a = f.thread("Camion", Some(&topic));
    let b = f.thread("Adresse", Some(&topic));
    let private = "Je préfère déménager le matin";
    f.app
        .invoke_tool_in(
            "grokbot",
            Some(&a),
            "remember",
            json!({"text": private, "sensitive": false, "source": "user_stated"}),
        )
        .unwrap();
    // Le bot retient pour lui ; le sujet n'a rien reçu, même avec une carte en attente.
    assert!(f.topic_facts(&topic).is_empty());
    f.propose(&a, FACT);
    assert!(f.topic_facts(&topic).is_empty());
    assert!(!f.memory(&b).contains(FACT));
    assert_eq!(
        f.db()
            .query_row("SELECT COUNT(*) FROM wrapper_topic_facts", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    // La mémoire privée du bot reste lisible, comme avant.
    assert!(f.memory(&a).contains(private));
}

#[test]
fn pas_maintenant_n_ecrit_rien_dans_la_memoire_du_sujet() {
    let mut f = Fixture::new();
    let topic = f.topic("Déménagement");
    let a = f.thread("Camion", Some(&topic));
    let b = f.thread("Adresse", Some(&topic));
    let offer = f.propose(&a, FACT);

    f.post("/api/topics/memory/dismiss", json!({"offer_id": offer}));

    assert!(f.topic_facts(&topic).is_empty());
    for conversation in [&a, &b] {
        assert!(!f.memory(conversation).contains(FACT));
    }
    let card = f.card(&a, &offer);
    assert_eq!(card["payload"]["status"], "dismissed");
    assert_eq!(card["payload"]["text"], "");
    let stored: String = f
        .db()
        .query_row(
            "SELECT text FROM wrapper_topic_memory_offers WHERE id = ?1",
            [&offer],
            |r| r.get(0),
        )
        .unwrap();
    assert!(stored.is_empty());
    // Une carte écartée ne se retient plus.
    let error = f
        .post_result("/api/topics/memory/accept", json!({"offer_id": offer}))
        .unwrap_err();
    assert_eq!(error.status, 409);
    assert!(f.topic_facts(&topic).is_empty());
}

#[test]
fn un_fil_qui_a_quitte_le_sujet_ne_peut_plus_faire_retenir_sa_proposition() {
    let mut f = Fixture::new();
    let topic = f.topic("Déménagement");
    let other = f.topic("Impôts");
    let a = f.thread("Camion", Some(&topic));
    let offer = f.propose(&a, FACT);
    f.post(
        "/api/topics/assign",
        json!({"conversation_id": a, "topic_id": other}),
    );

    let error = f
        .post_result("/api/topics/memory/accept", json!({"offer_id": offer}))
        .unwrap_err();
    assert_eq!(error.status, 409);
    assert!(f.topic_facts(&topic).is_empty() && f.topic_facts(&other).is_empty());
}

#[test]
fn oublier_retire_le_fait_de_tous_les_contextes_et_regenere_les_resumes() {
    let mut f = Fixture::new();
    let topic = f.topic("Déménagement");
    let a = f.thread("Camion", Some(&topic));
    let b = f.thread("Adresse", Some(&topic));
    let group = f.group("Équipe", &topic);
    let keep = f.accept(&f.propose(&a, "Le budget est de 800 euros"));
    let fact = f.accept(&f.propose(&a, FACT));
    let before: Vec<u64> = [&a, &b, &group]
        .iter()
        .map(|c| compaction::revision(&f.db(), c).unwrap())
        .collect();

    let response = f.post("/api/topics/memory/forget", json!({"fact_id": fact["id"]}));
    assert_eq!(response["forgotten"], json!([fact["id"]]));
    assert!(!response.to_string().contains("camion"));

    for conversation in [&a, &b, &group] {
        assert!(!f.memory(conversation).contains(FACT), "{conversation}");
        assert!(!f.recall(conversation, "camion").contains(FACT));
        assert!(f.memory(conversation).contains("budget"));
    }
    // Les résumés de tous les fils du sujet sont invalidés : ils se régénèrent sans le fait.
    for (conversation, before) in [&a, &b, &group].iter().zip(before) {
        assert!(compaction::revision(&f.db(), conversation).unwrap() > before);
    }
    let facts = f.topic_facts(&topic);
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0]["id"], keep["id"]);
    let leftover: i64 = f
        .db()
        .query_row(
            "SELECT COUNT(*) FROM wrapper_topic_facts WHERE text LIKE '%camion%'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(leftover, 0);
    // La carte du fil d'origine ne ressort pas le texte oublié.
    let offer: String = f
        .db()
        .query_row(
            "SELECT id FROM wrapper_topic_memory_offers WHERE fact_id = ?1",
            [fact["id"].as_str().unwrap()],
            |r| r.get(0),
        )
        .unwrap();
    let card = f.card(&a, &offer);
    assert_eq!(card["payload"]["forgotten"], true);
    assert_eq!(card["payload"]["text"], "");

    let error = f
        .post_result("/api/topics/memory/forget", json!({"fact_id": fact["id"]}))
        .unwrap_err();
    assert_eq!(error.status, 404);
}

#[test]
fn supprimer_le_sujet_efface_ses_faits_seulement_apres_confirmation_et_garde_les_faits_prives() {
    let mut f = Fixture::new();
    let topic = f.topic("Déménagement");
    let a = f.thread("Camion", Some(&topic));
    let b = f.thread("Adresse", Some(&topic));
    let private = "Je préfère déménager le matin";
    f.app
        .invoke_tool_in(
            "grokbot",
            Some(&a),
            "remember",
            json!({"text": private, "sensitive": false, "source": "user_stated"}),
        )
        .unwrap();
    f.accept(&f.propose(&a, FACT));
    let pending = f.propose(&b, "Le garde-meuble ferme à 18 h");

    // Sans la confirmation de Charles, le sujet et sa mémoire restent.
    let error = f
        .post_result("/api/topics/delete", json!({"id": topic}))
        .unwrap_err();
    assert_eq!(error.status, 409);
    assert!(error.message.contains("1 souvenir"));
    assert_eq!(f.topic_facts(&topic).len(), 1);
    assert!(f.memory(&b).contains(FACT));

    f.post(
        "/api/topics/delete",
        json!({"id": topic, "forget_memory": true}),
    );

    let state = f.state();
    assert!(state["topics"]["items"].as_array().unwrap().is_empty());
    assert!(state["topics"]["memory"].as_object().unwrap().is_empty());
    let rows: i64 = f
        .db()
        .query_row("SELECT COUNT(*) FROM wrapper_topic_facts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows, 0);
    // Une proposition en attente ne survit pas non plus au sujet.
    let stored: (String, String) = f
        .db()
        .query_row(
            "SELECT status, text FROM wrapper_topic_memory_offers WHERE id = ?1",
            [&pending],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(stored, ("dismissed".into(), String::new()));
    // Les fils et la mémoire privée du bot restent.
    assert!(!f.memory(&b).contains(FACT));
    assert!(f.memory(&a).contains(private));
    let listed = f.app.api("GET", "/api/memory?bot_id=grokbot", b"").unwrap();
    assert!(listed["facts"].to_string().contains(private));
    assert_eq!(
        state["conversations"].as_array().unwrap().len(),
        f.state()["conversations"].as_array().unwrap().len()
    );
}

#[test]
fn supprimer_un_sujet_sans_fait_n_exige_aucune_confirmation() {
    let mut f = Fixture::new();
    let topic = f.topic("Vide");
    f.thread("Fil", Some(&topic));
    f.post("/api/topics/delete", json!({"id": topic}));
    assert!(f.state()["topics"]["items"].as_array().unwrap().is_empty());
}

#[test]
fn la_meme_proposition_en_attente_ne_fait_qu_une_carte_et_un_fait_deja_retenu_n_en_fait_pas() {
    let mut f = Fixture::new();
    let topic = f.topic("Déménagement");
    let a = f.thread("Camion", Some(&topic));
    let first = f.propose(&a, FACT);
    assert_eq!(
        f.propose(&a, "  Le camion est   réservé pour le 1er juillet "),
        first
    );
    f.accept(&first);
    match f.tool("grokbot", &a, "propose_topic_memory", json!({"text": FACT})) {
        ToolOutcome::Done { output } => assert_eq!(output["status"], "already_retained"),
        _ => panic!("un fait déjà retenu ne se repropose pas"),
    }
}
