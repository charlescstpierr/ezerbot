//! Sélection persistée, sessions et fichiers privés distincts. Les workers partagent
//! leur registre mais chaque opération réseau garde sa session d’origine.
use super::*;

#[derive(Clone)]
pub struct Provider {
    sharing: Session,
    codex: Session,
    selected: Arc<Mutex<bool>>,
    selection_file: PathBuf,
}

/// Small streamed probe. No output cap: a reasoning model would spend 16 tokens
/// before any visible answer and look disconnected. Minimal effort is sent only
/// when this model advertises it.
fn probe_model(codex: bool, models: &[Model]) -> String {
    models
        .first()
        .map(|model| model.slug.clone())
        .unwrap_or_else(|| if codex { codex::DEFAULT_MODEL } else { "gpt-5" }.into())
}

fn service_refusal(status: &Value) -> String {
    let detail = status["connection_test"]["message"]
        .as_str()
        .filter(|message| !message.is_empty())
        .unwrap_or("OpenAI n’a pas répondu.");
    format!("OpenAI n’a pas confirmé la connexion. Aucune réponse n’est produite. {detail}")
}

fn connection_probe_body(model: &str, efforts: &[Effort]) -> Value {
    let mut body = json!({
        "model": model,
        "input": [{"role": "user", "content": "Réponds seulement OK."}],
        "instructions": "Test de connexion. Réponds seulement OK.",
        "store": false,
        "stream": true
    });
    if efforts.iter().any(|effort| effort.effort == "minimal") {
        body["reasoning"] = json!({"effort": "minimal"});
    }
    body
}

impl Provider {
    pub fn new(db: &Path) -> Self {
        Self::with_backends(db, Endpoints::default(), Endpoints::codex(), true)
    }

    /// Injection historique : les tests existants ciblent explicitement le partage d’abonnement.
    pub fn with_endpoints(db: &Path, endpoints: Endpoints, open_browser: bool) -> Self {
        let provider = Self::with_backends(db, endpoints, Endpoints::codex(), open_browser);
        *lock(&provider.selected) = false;
        provider
    }

    pub fn with_backends(
        db: &Path,
        sharing: Endpoints,
        codex: Endpoints,
        open_browser: bool,
    ) -> Self {
        let sharing = Session::with_endpoints(db, sharing, open_browser, false);
        let codex = Session::with_endpoints(db, codex, open_browser, true);
        let selection_file = db.with_extension("provider.json");
        let saved = fs::read(&selection_file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<String>(&bytes).ok());
        // Ne jamais déconnecter silencieusement une installation déjà liée au partage.
        let selected = saved.map_or_else(|| !sharing.connected(), |id| id != "sharing");
        Self {
            sharing,
            codex,
            selected: Arc::new(Mutex::new(selected)),
            selection_file,
        }
    }

    fn session(&self) -> Session {
        if *lock(&self.selected) {
            self.codex.clone()
        } else {
            self.sharing.clone()
        }
    }

    pub fn select_backend(&self, backend: &str) -> Result<(), Failure> {
        let codex = match backend {
            "codex" => true,
            "sharing" => false,
            _ => return Err(Failure::other("Mode de connexion inconnu.")),
        };
        let mut selected = lock(&self.selected);
        if *selected == codex {
            return Ok(());
        }
        if [&self.sharing, &self.codex].iter().any(|session| {
            session.inner.local_revoked.load(Ordering::SeqCst)
                && session
                    .read()
                    .is_some_and(|credentials| credentials.usable(session.inner.codex))
        }) {
            return Err(Failure::other(
                "Terminez d’abord l’effacement des identifiants sur ce Mac.",
            ));
        }
        if self.has_run_workers() {
            return Err(Failure::other(
                "Attendez la fin des réponses et des tâches avant de changer de connexion.",
            ));
        }
        self.sharing.cancel_signin();
        self.codex.cancel_signin();
        let temporary = self.selection_file.with_extension("json.tmp");
        if let Some(parent) = temporary
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).map_err(storage_failure)?;
        }
        let mut options = fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary).map_err(storage_failure)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(storage_failure)?;
        }
        file.write_all(&serde_json::to_vec(backend).expect("nom constant"))
            .map_err(storage_failure)?;
        file.sync_all().map_err(storage_failure)?;
        fs::rename(temporary, &self.selection_file).map_err(storage_failure)?;
        *selected = codex;
        Ok(())
    }

    /// Stable account binding without storing tokens in demonstration metadata.
    pub(crate) fn demonstration_identity(&self) -> String {
        let session = self.session();
        let _guard = lock(&session.inner.credentials);
        let c = session.read().unwrap_or_default();
        format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&json!([session.inner.codex, c.account_id, c.subject,]))
                    .unwrap_or_default()
            )
        )
    }

    pub fn connected(&self) -> bool {
        self.session().connected()
    }
    pub fn status(&self) -> Value {
        let session = self.session();
        let mut value = session.status();
        let signed_in = value["connected"] == true;
        value["signed_in"] = json!(signed_in);
        value["titling"] = json!(self.sharing.inner.titling.load(Ordering::SeqCst) > 0);
        value["simulation"] = json!(self.simulation_allowed());
        // A token on disk is not a connection. Confirm with one real Responses call.
        if signed_in
            && lock(&session.inner.probe)["state"]
                .as_str()
                .unwrap_or("idle")
                == "idle"
        {
            self.test_connection();
        }
        let probe = lock(&session.inner.probe).clone();
        let verified = signed_in && probe["state"] == "ok";
        value["connection_test"] = probe;
        value["connected"] = json!(verified);
        value["connections"] =
            json!({"codex":self.codex.connected(), "sharing":self.sharing.connected()});
        value
    }

    /// Blocks until the selected account has a successful Responses call, or fails closed.
    pub(crate) fn ensure_service(&self) -> Result<(), Failure> {
        if !self.connected() {
            return Err(Failure::signin());
        }
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let status = self.status();
            if status["connected"] == true {
                return Ok(());
            }
            let state = status["connection_test"]["state"]
                .as_str()
                .unwrap_or("idle");
            if state == "failed" || state == "unavailable" {
                return Err(Failure::other(service_refusal(&status)));
            }
            if Instant::now() >= deadline {
                return Err(Failure::other(
                    "OpenAI n’a pas confirmé la connexion à temps. Aucune réponse n’est produite.",
                ));
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
    /// Quota of the selected Ezerbot-owned Codex session only. No credentials
    /// from the CLI, pi, or another application's keychain are consulted.
    pub fn codex_usage(&self, refresh: bool) -> Value {
        if !*lock(&self.selected) {
            return json!({"status":"unsupported"});
        }
        self.codex.quota_status(refresh)
    }
    pub(crate) fn simulation_allowed(&self) -> bool {
        cfg!(debug_assertions) && self.sharing.inner.simulation.load(Ordering::SeqCst)
    }
    /// Debug-only local fake control; never changes production deadlines.
    #[cfg(debug_assertions)]
    pub fn configure_fake_chat(&self, timeout: Duration, simulation: bool) {
        self.configure_fake_deadlines(timeout, timeout, timeout * 5);
        for session in [&self.sharing, &self.codex] {
            if session.inner.endpoints.api.starts_with("http://127.0.0.1:") {
                session.inner.simulation.store(simulation, Ordering::SeqCst);
            }
        }
    }
    /// Loopback only. A test can keep the production overall deadline while
    /// shortening the read timeout that ureq must actually apply.
    #[cfg(debug_assertions)]
    pub fn configure_fake_deadlines(&self, first: Duration, stall: Duration, total: Duration) {
        for session in [&self.sharing, &self.codex] {
            if session.inner.endpoints.api.starts_with("http://127.0.0.1:") {
                *lock(&session.inner.deadlines) = super::reliable::Deadlines {
                    first,
                    stall,
                    total,
                };
            }
        }
    }
    pub(crate) fn test_connection(&self) {
        let session = self.session();
        if !session.connected() {
            *lock(&session.inner.probe) = json!({"state":"unavailable", "message":"Aucun compte connecté. Connectez-vous via Codex ci-dessus."});
            return;
        }
        let generation = {
            let mut probe = lock(&session.inner.probe);
            if probe["state"] == "running" {
                return;
            }
            *probe = json!({"state":"running"});
            session.inner.probe_generation.load(Ordering::SeqCst)
        };
        thread::spawn(move || {
            let mut failure = None;
            let mut responded = false;
            let mut capped = false;
            let models = session.models();
            let model = probe_model(session.inner.codex, &models);
            let efforts = models
                .first()
                .map(|model| model.efforts.clone())
                .unwrap_or_default();
            let result = session.reliable_stream(
                connection_probe_body(&model, &efforts),
                &AtomicBool::new(false),
                Some("connection-test"),
                &mut |_, error| {
                    if let Some(error) = error {
                        failure = Some(error.message.clone());
                    }
                    Ok(())
                },
                &mut |event| {
                    match &event {
                        grokbot_engine::contracts::ModelEvent::Completed => responded = true,
                        grokbot_engine::contracts::ModelEvent::Incomplete { reason }
                            if reason == "max_output_tokens" =>
                        {
                            // A reasoning model that spends the cap still proves the service answers.
                            responded = true;
                            capped = true;
                        }
                        _ => {}
                    }
                    Ok(())
                },
            );
            if capped {
                failure = None;
            }
            if !session.connected() {
                failure = Some(Failure::signin().message);
            }
            let mut probe = lock(&session.inner.probe);
            if session.inner.probe_generation.load(Ordering::SeqCst) != generation {
                return; // A previous account's result must never label a new session.
            }
            *probe = if responded && result.is_ok() && failure.is_none() {
                json!({"state":"ok","message":"OK — ChatGPT répond."})
            } else {
                json!({"state":"failed","message":failure.unwrap_or_else(|| "La connexion a été interrompue. Réessayez.".into())})
            };
        });
    }
    pub fn start_signin(&self) -> Result<String, Failure> {
        self.session().start_signin()
    }
    pub fn cancel_signin(&self) {
        self.session().cancel_signin();
    }
    pub fn mark_welcomed(&self) -> std::io::Result<()> {
        self.session().mark_welcomed()
    }
    pub fn signout(&self) -> Result<bool, Failure> {
        self.session().signout()
    }
    pub fn models(&self) -> Vec<Model> {
        self.session().models()
    }
    pub fn refresh_models(&self) {
        self.session().refresh_models();
    }
    pub(crate) fn retired_model_replacement(&self, model: &str) -> Option<&'static str> {
        (self.session().inner.codex && matches!(model, "gpt-5.5" | "gpt-5.3-codex-spark"))
            .then_some(codex::DEFAULT_MODEL)
    }
    pub(crate) fn accepts_images(&self, model: &str) -> bool {
        !self.session().inner.codex || codex::models().iter().any(|m| m.slug == model)
    }
    pub(crate) fn image_description_model(&self, model: &str) -> String {
        if self.accepts_images(model) {
            model.into()
        } else {
            codex::DEFAULT_MODEL.into()
        }
    }
    pub fn responses(&self, body: Value) -> Result<ureq::Response, Failure> {
        self.session().responses(body)
    }
    pub(crate) fn responses_background(&self, body: Value) -> Result<ureq::Response, Failure> {
        self.session().responses_background(body)
    }
    pub(crate) fn background_ready(&self) -> bool {
        self.session()
            .read()
            .is_some_and(|c| c.expires_at > seconds() + REFRESH_MARGIN)
    }
    pub(crate) fn diagnostics(&self, response: &ureq::Response) -> crate::upstream::Diagnostics {
        self.sharing.diagnostics(response)
    }
    pub(crate) fn extract_memory(
        &self,
        instructions: &str,
        input: &str,
    ) -> Result<String, Failure> {
        self.session().extract_memory(instructions, input)
    }
    pub(crate) fn review(&self, body: Value) -> Result<String, Failure> {
        self.session().review(body)
    }
    pub(super) fn stream_canonical(
        &self,
        request: &grokbot_engine::contracts::ModelRequest,
        emit: &mut dyn FnMut(
            grokbot_engine::contracts::ModelEvent,
        ) -> grokbot_engine::contracts::Result<()>,
    ) -> grokbot_engine::contracts::Result<()> {
        self.session().reliable_stream(
            crate::gateway::request_body(request)?,
            &AtomicBool::new(false),
            None,
            &mut |_, _| Ok(()),
            emit,
        )
    }
    pub(crate) fn stream_run(
        &self,
        request: &grokbot_engine::contracts::ModelRequest,
        cancel: &AtomicBool,
        turn: Option<&str>,
        metadata: &mut MetadataSink<'_>,
        emit: &mut dyn FnMut(
            grokbot_engine::contracts::ModelEvent,
        ) -> grokbot_engine::contracts::Result<()>,
    ) -> grokbot_engine::contracts::Result<()> {
        self.session()
            .stream_run(request, cancel, turn, metadata, emit)
    }
    pub(crate) fn mcp(&self) -> &crate::mcp::Manager {
        self.sharing.mcp()
    }
    pub(crate) fn claim_run(&self, id: &str) -> Option<Arc<AtomicBool>> {
        self.sharing.claim_run(id)
    }
    pub(crate) fn interrupting(&self) -> bool {
        self.sharing.interrupting()
    }
    pub(crate) fn interrupt_runs(&self) {
        self.sharing.interrupt_runs();
        self.session().account_all_live();
    }
    pub(crate) fn has_run_workers(&self) -> bool {
        self.sharing.has_run_workers()
    }
    pub(crate) fn release_run(&self, id: &str) {
        self.sharing.release_run(id);
    }
    pub(crate) fn cancel_run(&self, id: &str) {
        self.sharing.cancel_run(id);
        // Le drapeau vit sur la session partage ; le socket, sur la session qui streame.
        self.session().account_live_cancel(id);
    }
    #[cfg(test)]
    pub(crate) fn pause_turn(&self, id: &str) {
        self.session().pause_turn(id);
    }
    #[cfg(test)]
    pub(crate) fn resume_turns(&self) {
        self.session().resume_turns();
    }
    pub(crate) fn title_started(&self) {
        self.sharing.title_started();
    }
    pub(crate) fn title_finished(&self) {
        self.sharing.title_finished();
    }
}

fn storage_failure(_: std::io::Error) -> Failure {
    Failure::other("Impossible d’enregistrer le mode de connexion sur ce Mac.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_image_fallback_and_retired_selection_use_supported_default_only() {
        let db = std::env::temp_dir()
            .join(format!("ezerbot-model-selection-{}", new_host_id()))
            .join("workspace.sqlite");
        let provider =
            Provider::with_backends(&db, Endpoints::default(), Endpoints::codex(), false);
        assert!(provider.accepts_images(codex::DEFAULT_MODEL));
        for model in ["gpt-5.5", "gpt-5.3-codex-spark", "unknown"] {
            assert!(!provider.accepts_images(model));
            assert_eq!(
                provider.image_description_model(model),
                codex::DEFAULT_MODEL
            );
        }
        for retired in ["gpt-5.5", "gpt-5.3-codex-spark"] {
            assert_eq!(
                provider.retired_model_replacement(retired),
                Some(codex::DEFAULT_MODEL)
            );
        }
        assert_eq!(provider.retired_model_replacement("gpt-6-sol"), None);
        assert_eq!(provider.image_description_model("gpt-6-sol"), "gpt-6-sol");
        *lock(&provider.selected) = false;
        assert!(provider.accepts_images("gpt-5.5"));
        assert_eq!(provider.image_description_model("gpt-5.5"), "gpt-5.5");
        assert_eq!(provider.retired_model_replacement("gpt-5.5"), None);
    }

    #[test]
    fn probe_requests_minimal_reasoning_only_when_the_model_lists_it() {
        let bare = connection_probe_body("gpt-5", &[]);
        assert!(bare.get("max_output_tokens").is_none());
        assert!(bare.get("reasoning").is_none());
        assert_eq!(
            bare["instructions"],
            "Test de connexion. Réponds seulement OK."
        );
        let listed = connection_probe_body(
            "gpt-5",
            &[
                Effort {
                    effort: "minimal".into(),
                    description: "Court".into(),
                },
                Effort {
                    effort: "high".into(),
                    description: "Long".into(),
                },
            ],
        );
        assert_eq!(listed["reasoning"]["effort"], "minimal");
        assert!(listed.get("max_output_tokens").is_none());
        let other = connection_probe_body(
            "gpt-5.5",
            &[Effort {
                effort: "low".into(),
                description: "Léger".into(),
            }],
        );
        assert!(other.get("reasoning").is_none());
    }

    #[test]
    fn empty_codex_catalog_probes_the_supported_default_not_a_retired_model() {
        assert_eq!(probe_model(true, &[]), codex::DEFAULT_MODEL);
        assert_ne!(probe_model(true, &[]), "gpt-5.5");
        assert_eq!(probe_model(false, &[]), "gpt-5");
        let listed = [Model {
            slug: "gpt-6-sol".into(),
            name: "GPT-6 Sol".into(),
            description: String::new(),
            efforts: vec![],
            default_effort: None,
            tiers: vec![],
        }];
        assert_eq!(probe_model(true, &listed), "gpt-6-sol");
    }
}
