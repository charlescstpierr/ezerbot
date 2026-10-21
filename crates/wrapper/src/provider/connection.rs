//! Une seule connexion de production : le compte ChatGPT via OAuth Codex.
//! L'injection historique de /v1 reste réservée aux tests de protocole locaux.
use super::*;

#[derive(Clone)]
pub struct Provider {
    primary: Session,
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
        Self {
            primary: Session::with_endpoints(db, Endpoints::codex(), true, true),
        }
    }

    /// Injection historique : les tests existants ciblent explicitement le partage d’abonnement.
    pub fn with_endpoints(db: &Path, endpoints: Endpoints, open_browser: bool) -> Self {
        // No runtime route can enable the retired subscription-sharing protocol.
        // Existing /v1 protocol fixtures inject a loopback server explicitly.
        if !endpoints.api.starts_with("http://127.0.0.1:") {
            return Self::new(db);
        }
        Self {
            primary: Session::with_endpoints(db, endpoints, open_browser, false),
        }
    }

    pub fn with_backends(
        db: &Path,
        _sharing: Endpoints,
        codex: Endpoints,
        open_browser: bool,
    ) -> Self {
        Self {
            primary: Session::with_endpoints(db, codex, open_browser, true),
        }
    }

    fn session(&self) -> Session {
        self.primary.clone()
    }

    pub fn select_backend(&self, backend: &str) -> Result<(), Failure> {
        if backend == "codex" {
            Ok(()) // Compatibility for older clients, never a second account.
        } else {
            Err(Failure::other(
                "Une seule connexion est disponible : votre compte ChatGPT.",
            ))
        }
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
        value["titling"] = json!(self.primary.inner.titling.load(Ordering::SeqCst) > 0);
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
        value
    }

    /// Quota of the selected Ezerbot-owned Codex session only. No credentials
    /// from the CLI, pi, or another application's keychain are consulted.
    pub fn codex_usage(&self, refresh: bool) -> Value {
        if !self.primary.inner.codex {
            return json!({"status":"unsupported"});
        }
        self.primary.quota_status(refresh)
    }
    pub(crate) fn simulation_allowed(&self) -> bool {
        cfg!(debug_assertions) && self.primary.inner.simulation.load(Ordering::SeqCst)
    }
    /// Debug-only local fake control; never changes production deadlines.
    #[cfg(debug_assertions)]
    pub fn configure_fake_chat(&self, timeout: Duration, simulation: bool) {
        self.configure_fake_deadlines(timeout, timeout, timeout * 5);
        let session = &self.primary;
        if session.inner.endpoints.api.starts_with("http://127.0.0.1:") {
            session.inner.simulation.store(simulation, Ordering::SeqCst);
        }
    }
    /// Loopback only. A test can keep the production overall deadline while
    /// shortening the read timeout that ureq must actually apply.
    #[cfg(debug_assertions)]
    pub fn configure_fake_deadlines(&self, first: Duration, stall: Duration, total: Duration) {
        let session = &self.primary;
        if session.inner.endpoints.api.starts_with("http://127.0.0.1:") {
            *lock(&session.inner.deadlines) = super::reliable::Deadlines {
                first,
                stall,
                total,
            };
        }
    }
    pub(crate) fn test_connection(&self) {
        self.test_connection_with_refresh(false);
    }
    /// Explicit refresh test uses the app's own grant in the network worker.
    /// It never exposes credentials, changes accounts, or waits on the UI thread.
    pub(crate) fn test_connection_with_refresh(&self, refresh: bool) {
        let session = self.session();
        if !session.connected() {
            *lock(&session.inner.probe) = json!({"state":"unavailable", "message":"Aucun compte connecté. Connectez votre compte ChatGPT ci-dessus."});
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
            let mut visible = false;
            let models = session.models();
            let models = if models.is_empty() {
                session.fetch_models().unwrap_or_default()
            } else {
                models
            };
            let model = probe_model(session.inner.codex, &models);
            let efforts = models
                .first()
                .map(|model| model.efforts.clone())
                .unwrap_or_default();
            let renewed = if refresh {
                session.bearer(true).map(|_| ())
            } else {
                Ok(())
            };
            let result = match renewed {
                Err(error) => {
                    failure = Some(error.message);
                    Err(grokbot_engine::contracts::Error::InvalidInput(
                        "refresh failed".into(),
                    ))
                }
                Ok(()) => session.reliable_stream(
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
                            grokbot_engine::contracts::ModelEvent::TextDelta { text } => {
                                visible |= !text.trim().is_empty();
                            }
                            _ => {}
                        }
                        Ok(())
                    },
                ),
            };
            if !session.connected() {
                failure = Some(Failure::signin().message);
            }
            let mut probe = lock(&session.inner.probe);
            if session.inner.probe_generation.load(Ordering::SeqCst) != generation {
                return; // A previous account's result must never label a new session.
            }
            *probe = if responded && visible && result.is_ok() && failure.is_none() {
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
        let models = self.models();
        !self.primary.inner.codex
            || (model != "gpt-5.3-codex-spark"
                && if models.is_empty() {
                    codex::models()
                } else {
                    models
                }
                .iter()
                .any(|m| m.slug == model))
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
        self.primary.diagnostics(response)
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
        self.primary.mcp()
    }
    pub(crate) fn claim_run(&self, id: &str) -> Option<Arc<AtomicBool>> {
        self.primary.claim_run(id)
    }
    pub(crate) fn interrupting(&self) -> bool {
        self.primary.interrupting()
    }
    pub(crate) fn interrupt_runs(&self) {
        self.primary.interrupt_runs();
        self.session().account_all_live();
    }
    pub(crate) fn has_run_workers(&self) -> bool {
        self.primary.has_run_workers()
    }
    pub(crate) fn release_run(&self, id: &str) {
        self.primary.release_run(id);
    }
    pub(crate) fn cancel_run(&self, id: &str) {
        self.primary.cancel_run(id);
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
        self.primary.title_started();
    }
    pub(crate) fn title_finished(&self) {
        self.primary.title_finished();
    }

    /// API base of the selected session. Image calls derive their host from this.
    pub(crate) fn image_api_base(&self) -> String {
        self.session().inner.endpoints.api.clone()
    }

    /// Bearer of the current ChatGPT session. Image generation uses this token
    /// and never copies it into the account vault.
    pub(crate) fn session_bearer(&self) -> Result<String, Failure> {
        self.session().bearer(false)
    }
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
        assert!(provider.select_backend("sharing").is_err());
        assert_eq!(provider.status()["backend"], "codex");
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
