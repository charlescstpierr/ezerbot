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

    /// Stable account binding without storing tokens in demonstration metadata.
    pub(crate) fn demonstration_identity(&self) -> String {
        let session = self.session();
        let _guard = lock(&session.inner.credentials);
        let c = session.read().unwrap_or_default();
        format!(
            "{:x}",
            Sha256::digest(
                serde_json::to_vec(&json!([
                    session.inner.codex,
                    session.inner.local_revoked.load(Ordering::SeqCst),
                    c.subject,
                    c.account_id,
                    c.email,
                    c.client_id,
                ]))
                .unwrap_or_default()
            )
        )
    }

    pub fn connected(&self) -> bool {
        self.session().connected()
    }
    pub fn status(&self) -> Value {
        let mut value = self.session().status();
        value["titling"] = json!(self.sharing.inner.titling.load(Ordering::SeqCst) > 0);
        value["connections"] =
            json!({"codex":self.codex.connected(), "sharing":self.sharing.connected()});
        value
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
    pub(super) fn responses_with_options(
        &self,
        body: Value,
        fallback: bool,
        notices: &mut Vec<String>,
    ) -> Result<ureq::Response, Failure> {
        self.session()
            .responses_with_options(body, fallback, notices)
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
    pub(crate) fn stream_run(
        &self,
        request: &grokbot_engine::contracts::ModelRequest,
        cancel: &AtomicBool,
        metadata: &mut MetadataSink<'_>,
        emit: &mut dyn FnMut(
            grokbot_engine::contracts::ModelEvent,
        ) -> grokbot_engine::contracts::Result<()>,
    ) -> grokbot_engine::contracts::Result<()> {
        self.session().stream_run(request, cancel, metadata, emit)
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
}
