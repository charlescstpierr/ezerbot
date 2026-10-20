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

    pub fn connected(&self) -> bool {
        self.session().connected()
    }
    pub fn status(&self) -> Value {
        let mut value = self.session().status();
        value["titling"] = json!(self.sharing.inner.titling.load(Ordering::SeqCst) > 0);
        value["simulation"] = json!(self.simulation_allowed());
        value["connection_test"] = lock(&self.session().inner.probe).clone();
        value["connections"] =
            json!({"codex":self.codex.connected(), "sharing":self.sharing.connected()});
        value
    }
    pub(crate) fn simulation_allowed(&self) -> bool {
        cfg!(debug_assertions) && self.sharing.inner.simulation.load(Ordering::SeqCst)
    }
    /// Debug-only local fake control; never changes production deadlines.
    #[cfg(debug_assertions)]
    pub fn configure_fake_chat(&self, timeout: Duration, simulation: bool) {
        for session in [&self.sharing, &self.codex] {
            if session.inner.endpoints.api.starts_with("http://127.0.0.1:") {
                *lock(&session.inner.deadlines) = super::reliable::Deadlines {
                    first: timeout,
                    stall: timeout,
                    total: timeout * 5,
                };
                session.inner.simulation.store(simulation, Ordering::SeqCst);
            }
        }
    }
    pub(crate) fn test_connection(&self) {
        let session = self.session();
        {
            let mut probe = lock(&session.inner.probe);
            if probe["state"] == "running" {
                return;
            }
            *probe = json!({"state":"running"});
        }
        thread::spawn(move || {
            let mut failure = None;
            let mut completed = false;
            let model = session
                .models()
                .first()
                .map(|model| model.slug.clone())
                .unwrap_or_else(|| {
                    if session.inner.codex {
                        "gpt-5.5"
                    } else {
                        "gpt-5"
                    }
                    .into()
                });
            let result = session.reliable_stream(
                json!({"model": model,
                    "input":[{"role":"user","content":"Réponds seulement OK."}],
                    "instructions":"Test de connexion. Réponds seulement OK.",
                    "store":false,"stream":true,"max_output_tokens":16}),
                &AtomicBool::new(false),
                Some("connection-test"),
                &mut |_, error| {
                    if let Some(error) = error {
                        failure = Some(error.message.clone());
                    }
                    Ok(())
                },
                &mut |event| {
                    completed |= matches!(event, grokbot_engine::contracts::ModelEvent::Completed);
                    Ok(())
                },
            );
            if !session.connected() {
                failure = Some(Failure::signin().message);
            }
            *lock(&session.inner.probe) = if completed && result.is_ok() && failure.is_none() {
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
    pub(crate) fn accepts_images(&self, model: &str) -> bool {
        !self.session().inner.codex || model != "gpt-5.3-codex-spark"
    }
    pub(crate) fn image_description_model(&self, model: &str) -> String {
        if self.accepts_images(model) {
            model.into()
        } else {
            "gpt-5.5".into()
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
    }
    pub(crate) fn has_run_workers(&self) -> bool {
        self.sharing.has_run_workers()
    }
    pub(crate) fn release_run(&self, id: &str) {
        self.sharing.release_run(id);
    }
    pub(crate) fn cancel_run(&self, id: &str) {
        self.sharing.cancel_run(id);
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
