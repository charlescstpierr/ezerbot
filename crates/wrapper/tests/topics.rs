//! Sujets : regrouper des fils, sans jamais toucher aux conversations elles-mêmes.
use grokbot_wrapper::{ApiError, App};
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
            "topics-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("workspace.sqlite");
        let app = App::open(&path).unwrap();
        Self { app, path }
    }

    /// Referme puis rouvre la même base, comme après un redémarrage de l'app.
    fn restart(&mut self) {
        self.app = App::open(&self.path).unwrap();
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

    fn chat(&mut self, title: &str) -> String {
        self.post("/api/conversations", json!({"title": title}))["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn group(&mut self, title: &str) -> String {
        self.post(
            "/api/bots",
            json!({"id": "second-bot", "name": "Second", "color": "cyan", "instructions": "x"}),
        );
        self.post(
            "/api/conversations",
            json!({"title": title, "bot_ids": ["grokbot", "second-bot"]}),
        )["conversation"]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    fn put(&mut self, conversation: &str, topic: Option<&str>) {
        self.post(
            "/api/topics/assign",
            json!({"conversation_id": conversation, "topic_id": topic}),
        );
    }

    fn members_of(&mut self, topic: &str) -> Vec<String> {
        let state = self.state();
        let mut ids: Vec<String> = state["topics"]["membership"]
            .as_object()
            .unwrap()
            .iter()
            .filter(|(_, value)| value.as_str() == Some(topic))
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        ids
    }

    fn conversation_ids(&mut self) -> Vec<String> {
        let state = self.state();
        let mut ids: Vec<String> = state["conversations"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap().to_owned())
            .collect();
        ids.sort();
        ids
    }
}

fn sorted(mut ids: Vec<String>) -> Vec<String> {
    ids.sort();
    ids
}

#[test]
fn un_sujet_et_ses_deux_fils_survivent_au_redemarrage() {
    let mut f = Fixture::new();
    let topic = f.post(
        "/api/topics",
        json!({"name": "Déménagement", "description": "Tout ce qui touche au 1er juillet"}),
    )["topic"]
        .clone();
    let id = topic["id"].as_str().unwrap().to_owned();
    assert_eq!(topic["name"], "Déménagement");
    assert_eq!(topic["description"], "Tout ce qui touche au 1er juillet");
    let a = f.chat("Camion");
    let b = f.chat("Adresse");
    f.put(&a, Some(&id));
    f.put(&b, Some(&id));
    assert_eq!(f.members_of(&id), sorted(vec![a.clone(), b.clone()]));

    f.restart();
    let state = f.state();
    let items = state["topics"]["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], id.as_str());
    assert_eq!(items[0]["name"], "Déménagement");
    assert_eq!(f.members_of(&id), sorted(vec![a, b]));
}

#[test]
fn supprimer_le_sujet_laisse_les_deux_conversations_intactes_sans_sujet() {
    let mut f = Fixture::new();
    let id = f.topic("À défaire");
    let a = f.chat("Premier");
    let b = f.chat("Second");
    f.put(&a, Some(&id));
    f.put(&b, Some(&id));
    let before = f.conversation_ids();

    f.post("/api/topics/delete", json!({"id": id}));

    f.restart();
    let state = f.state();
    assert!(state["topics"]["items"].as_array().unwrap().is_empty());
    assert!(state["topics"]["membership"]
        .as_object()
        .unwrap()
        .is_empty());
    assert_eq!(f.conversation_ids(), before);
    let titles: Vec<&str> = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["title"].as_str().unwrap())
        .collect();
    assert!(titles.contains(&"Premier") && titles.contains(&"Second"));
    // Le sujet supprimé ne peut plus recevoir de fil.
    let error = f
        .post_result(
            "/api/topics/assign",
            json!({"conversation_id": a, "topic_id": id}),
        )
        .unwrap_err();
    assert_eq!(error.status, 404);
}

#[test]
fn un_groupe_et_un_fil_de_bot_seul_partagent_un_sujet() {
    let mut f = Fixture::new();
    let id = f.topic("Mixte");
    let solo = f.chat("Bot seul");
    let group = f.group("Équipe");
    f.put(&solo, Some(&id));
    f.put(&group, Some(&id));
    assert_eq!(f.members_of(&id), sorted(vec![solo, group]));
}

#[test]
fn une_conversation_n_appartient_qu_a_un_seul_sujet() {
    let mut f = Fixture::new();
    let first = f.topic("Ancien");
    let second = f.topic("Nouveau");
    let conversation = f.chat("Voyageuse");
    f.put(&conversation, Some(&first));
    f.put(&conversation, Some(&second));
    assert!(f.members_of(&first).is_empty());
    assert_eq!(f.members_of(&second), vec![conversation.clone()]);

    f.restart();
    assert!(f.members_of(&first).is_empty());
    assert_eq!(f.members_of(&second), vec![conversation.clone()]);

    // Retirer la conversation du sujet la laisse sans sujet.
    f.put(&conversation, None);
    assert!(f.members_of(&second).is_empty());
    assert!(f.conversation_ids().contains(&conversation));
}

#[test]
fn une_conversation_supprimee_disparait_du_sujet_sans_erreur() {
    let mut f = Fixture::new();
    let id = f.topic("Nettoyage");
    let keep = f.chat("Garde");
    let gone = f.chat("Part");
    f.put(&keep, Some(&id));
    f.put(&gone, Some(&id));

    f.post("/api/conversations/delete", json!({"id": gone}));

    assert_eq!(f.members_of(&id), vec![keep.clone()]);
    f.restart();
    assert_eq!(f.members_of(&id), vec![keep]);
    let error = f
        .post_result(
            "/api/topics/assign",
            json!({"conversation_id": gone, "topic_id": id}),
        )
        .unwrap_err();
    assert_eq!(error.status, 409);
}

#[test]
fn nouveau_fil_dans_un_sujet_cree_une_conversation_deja_classee() {
    let mut f = Fixture::new();
    let id = f.topic("Projet");
    let created = f.post(
        "/api/conversations",
        json!({"title": "Premier fil", "topic_id": id}),
    );
    let conversation = created["conversation"]["id"].as_str().unwrap().to_owned();
    assert_eq!(f.members_of(&id), vec![conversation]);
}

#[test]
fn un_sujet_introuvable_ne_cree_aucun_fil() {
    let mut f = Fixture::new();
    let before = f.conversation_ids();
    let error = f
        .post_result(
            "/api/conversations",
            json!({"title": "Orphelin", "topic_id": "inexistant"}),
        )
        .unwrap_err();
    assert_eq!(error.status, 404);
    assert_eq!(f.conversation_ids(), before);
}

#[test]
fn renommer_et_decrire_un_sujet() {
    let mut f = Fixture::new();
    let id = f.topic("Brouillon");
    let renamed = f.post(
        "/api/topics/update",
        json!({"id": id, "name": "Définitif", "description": "Une note"}),
    );
    assert_eq!(renamed["topic"]["name"], "Définitif");
    assert_eq!(renamed["topic"]["description"], "Une note");
    // Une description vide l'efface ; le nom ne change pas.
    let cleared = f.post("/api/topics/update", json!({"id": id, "description": ""}));
    assert_eq!(cleared["topic"]["name"], "Définitif");
    assert_eq!(cleared["topic"]["description"], "");
    for body in [
        json!({"id": id}),
        json!({"id": id, "name": "   "}),
        json!({"id": id, "name": "x".repeat(81)}),
        json!({"id": "inconnu", "name": "Autre"}),
    ] {
        assert!(f.post_result("/api/topics/update", body).is_err());
    }
    assert!(f.post_result("/api/topics", json!({"name": ""})).is_err());
}

#[test]
fn les_conversations_sans_sujet_restent_comme_avant() {
    let mut f = Fixture::new();
    let loose = f.chat("Libre");
    let state = f.state();
    assert!(state["topics"]["items"].as_array().unwrap().is_empty());
    assert!(!state["topics"]["membership"]
        .as_object()
        .unwrap()
        .contains_key(&loose));
    let conversation = state["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == loose.as_str())
        .unwrap();
    assert_eq!(conversation["title"], "Libre");
}
