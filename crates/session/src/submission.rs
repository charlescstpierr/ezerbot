use std::path::Path;
use std::sync::Arc;

use grokbot_domain::{
    AttemptId, AuthenticatedActor, ConversationId, DeliveryReceipt, MessageId, ReconciliationId,
    Refusal, SubmissionState, TranscriptEntryId,
};
use grokbot_store::{SessionStore, StoredJournal};
use grokbot_submission_journal::{CommittedSubmission, JournalError, PreparedMessage};

use crate::SessionError;

#[derive(Debug)]
pub struct SendRequest {
    owner: Arc<()>,
    actor: AuthenticatedActor,
    conversation_id: ConversationId,
    message_id: MessageId,
    body: String,
    attempt: AttemptId,
}

impl SendRequest {
    pub fn actor(&self) -> &AuthenticatedActor {
        &self.actor
    }
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }
    pub fn message_id(&self) -> &MessageId {
        &self.message_id
    }
    pub fn body(&self) -> &str {
        &self.body
    }
    pub fn attempt(&self) -> &AttemptId {
        &self.attempt
    }
}

#[derive(Debug)]
pub struct ReconciliationRequest {
    owner: Arc<()>,
    actor: AuthenticatedActor,
    conversation_id: ConversationId,
    message_id: MessageId,
    reconciliation: ReconciliationId,
}

impl ReconciliationRequest {
    pub fn actor(&self) -> &AuthenticatedActor {
        &self.actor
    }
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }
    pub fn message_id(&self) -> &MessageId {
        &self.message_id
    }
    pub fn reconciliation(&self) -> &ReconciliationId {
        &self.reconciliation
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SendOutcome {
    Accepted(DeliveryReceipt),
    Refused(Refusal),
    OutcomeUnknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LookupOutcome {
    Found(DeliveryReceipt),
    NotFound,
    Refused(Refusal),
    Pending,
    Unknown,
}

/// Une commande réalise un seul effet de transport, sans renvoi automatique.
pub trait SubmissionTransport {
    fn send(&mut self, request: &SendRequest) -> SendOutcome;
    fn lookup(&mut self, request: &ReconciliationRequest) -> LookupOutcome;
}

pub struct SubmissionSession {
    actor: AuthenticatedActor,
    owner: Arc<()>,
    store: SessionStore,
    journal: Option<StoredJournal>,
    send_counter: u64,
    reconciliation_counter: u64,
    revoked: bool,
}

impl SubmissionSession {
    pub fn open(path: &Path, actor: AuthenticatedActor) -> Result<Self, SessionError> {
        let mut store = SessionStore::open(path, &actor.account_id)?;
        let mut journal = store.load_journal()?;
        if journal.adopt_generation(&actor)? {
            store.save_journal(&mut journal)?;
        }
        Ok(Self {
            actor,
            owner: Arc::new(()),
            store,
            journal: Some(journal),
            send_counter: 0,
            reconciliation_counter: 0,
            revoked: false,
        })
    }
    pub fn actor(&self) -> &AuthenticatedActor {
        &self.actor
    }
    pub fn state(
        &self,
        conversation: &ConversationId,
        message: &MessageId,
    ) -> Result<Option<SubmissionState>, SessionError> {
        Ok(self
            .journal
            .as_ref()
            .ok_or(SessionError::ReloadRequired)?
            .get(&self.actor.account_id, conversation, message))
    }
    pub fn prepare(
        &mut self,
        conversation: ConversationId,
        message: MessageId,
        body: String,
    ) -> Result<SubmissionState, SessionError> {
        self.ensure_active()?;
        let prepared = PreparedMessage {
            actor: self.actor.clone(),
            conversation_id: conversation,
            message_id: message,
            body,
        };
        let state = self.journal_mut()?.prepare(prepared)?;
        self.persist()?;
        Ok(state)
    }
    /// Valide `Dispatching` sur disque avant de rendre une requête d'envoi.
    pub fn begin_send(
        &mut self,
        conversation: &ConversationId,
        message: &MessageId,
    ) -> Result<SendRequest, SessionError> {
        self.ensure_active()?;
        let item = self.current(conversation, message)?;
        let actor = self.actor.clone();
        let number = next_counter(&mut self.send_counter)?;
        let attempt =
            AttemptId::new(format!("send-{number}")).map_err(|_| SessionError::CounterExhausted)?;
        self.journal_mut()?
            .dispatch(&actor, conversation, message, attempt.clone())?;
        self.persist()?;
        Ok(SendRequest {
            owner: Arc::clone(&self.owner),
            actor,
            conversation_id: conversation.clone(),
            message_id: message.clone(),
            body: item.body,
            attempt,
        })
    }

    /// N'annonce le résultat de cette tentative qu'après sa sauvegarde.
    pub fn apply_send(
        &mut self,
        request: &SendRequest,
        outcome: SendOutcome,
    ) -> Result<SubmissionState, SessionError> {
        self.ensure_request(&request.owner, &request.actor)?;
        let current = self.current(&request.conversation_id, &request.message_id)?;
        if current.state
            != (SubmissionState::Dispatching {
                attempt: request.attempt.clone(),
            })
        {
            return Err(SessionError::StaleRequest);
        }
        let actor = self.actor.clone();
        let state = match outcome {
            SendOutcome::Accepted(receipt) => {
                self.check_receipt(&request.conversation_id, &request.message_id, &receipt)?;
                self.journal_mut()?.observe_accept(&actor, receipt)?
            }
            SendOutcome::Refused(reason) => self.journal_mut()?.observe_refusal(
                &actor,
                &request.conversation_id,
                &request.message_id,
                reason,
            )?,
            SendOutcome::OutcomeUnknown => {
                let reconciliation = self.next_reconciliation()?;
                self.journal_mut()?.mark_outcome_unknown(
                    &actor,
                    &request.conversation_id,
                    &request.message_id,
                    reconciliation,
                )?
            }
        };
        self.persist()?;
        Ok(state)
    }

    /// Chaque recherche renouvelle et sauvegarde son token avant le lookup.
    pub fn begin_reconciliation(
        &mut self,
        conversation: &ConversationId,
        message: &MessageId,
    ) -> Result<ReconciliationRequest, SessionError> {
        self.ensure_active()?;
        let actor = self.actor.clone();
        let reconciliation = self.next_reconciliation()?;
        self.journal_mut()?.mark_outcome_unknown(
            &actor,
            conversation,
            message,
            reconciliation.clone(),
        )?;
        self.persist()?;
        Ok(ReconciliationRequest {
            owner: Arc::clone(&self.owner),
            actor,
            conversation_id: conversation.clone(),
            message_id: message.clone(),
            reconciliation,
        })
    }

    pub fn apply_reconciliation(
        &mut self,
        request: &ReconciliationRequest,
        outcome: LookupOutcome,
    ) -> Result<SubmissionState, SessionError> {
        self.ensure_request(&request.owner, &request.actor)?;
        let current = self.current(&request.conversation_id, &request.message_id)?;
        if current.state
            != (SubmissionState::OutcomeUnknown {
                reconciliation: request.reconciliation.clone(),
            })
        {
            return Err(SessionError::StaleRequest);
        }
        let actor = self.actor.clone();
        let state = match outcome {
            LookupOutcome::Found(receipt) => {
                self.check_receipt(&request.conversation_id, &request.message_id, &receipt)?;
                self.journal_mut()?.observe_reconciliation_found(
                    &actor,
                    &request.reconciliation,
                    receipt,
                )?
            }
            LookupOutcome::NotFound => self.journal_mut()?.observe_reconciliation_not_found(
                &actor,
                &request.conversation_id,
                &request.message_id,
                &request.reconciliation,
            )?,
            LookupOutcome::Refused(reason) => self.journal_mut()?.observe_refusal(
                &actor,
                &request.conversation_id,
                &request.message_id,
                reason,
            )?,
            LookupOutcome::Pending | LookupOutcome::Unknown => current.state,
        };
        self.persist()?;
        Ok(state)
    }
    pub fn send_with(
        &mut self,
        conversation: &ConversationId,
        message: &MessageId,
        transport: &mut impl SubmissionTransport,
    ) -> Result<SubmissionState, SessionError> {
        let request = self.begin_send(conversation, message)?;
        let outcome = transport.send(&request);
        self.apply_send(&request, outcome)
    }
    pub fn reconcile_with(
        &mut self,
        conversation: &ConversationId,
        message: &MessageId,
        transport: &mut impl SubmissionTransport,
    ) -> Result<SubmissionState, SessionError> {
        let request = self.begin_reconciliation(conversation, message)?;
        let outcome = transport.lookup(&request);
        self.apply_reconciliation(&request, outcome)
    }
    /// Oublie une soumission qui n’est pas livrée. Un message accepté ou écho reste.
    pub fn discard_undelivered(
        &mut self,
        conversation: &ConversationId,
        message: &MessageId,
    ) -> Result<(), SessionError> {
        self.ensure_active()?;
        let actor = self.actor.clone();
        self.journal_mut()?
            .discard_undelivered(&actor, conversation, message)?;
        self.persist()?;
        Ok(())
    }
    pub fn observe_echo(
        &mut self,
        conversation: &ConversationId,
        message: &MessageId,
        entry: TranscriptEntryId,
    ) -> Result<SubmissionState, SessionError> {
        self.ensure_active()?;
        let actor = self.actor.clone();
        let state = self
            .journal_mut()?
            .observe_echo(&actor, conversation, message, entry)?;
        self.persist()?;
        Ok(state)
    }
    pub fn revoke(&mut self) {
        self.revoked = true;
    }

    fn ensure_active(&self) -> Result<(), SessionError> {
        if self.revoked {
            return Err(SessionError::Revoked);
        }
        if self.journal.as_ref().is_some_and(|journal| {
            journal.committed().any(|item| {
                item.account_id == self.actor.account_id && item.generation > self.actor.generation
            })
        }) {
            return Err(JournalError::StaleAccount.into());
        }
        Ok(())
    }
    fn ensure_request(
        &mut self,
        owner: &Arc<()>,
        actor: &AuthenticatedActor,
    ) -> Result<(), SessionError> {
        self.ensure_active()?;
        // Le rechargement change le propriétaire avant cette comparaison.
        // Un ancien token textuel redevenu courant ne réactive pas sa requête.
        if self.journal.is_none() {
            self.journal_mut()?;
        }
        if Arc::ptr_eq(&self.owner, owner) && actor == &self.actor {
            Ok(())
        } else {
            Err(SessionError::StaleRequest)
        }
    }
    fn current(
        &mut self,
        conversation: &ConversationId,
        message: &MessageId,
    ) -> Result<CommittedSubmission, SessionError> {
        let actor = self.actor.clone();
        let item = self
            .journal_mut()?
            .committed_get(&actor.account_id, conversation, message)
            .cloned()
            .ok_or(JournalError::NotFound)?;
        if !actor.accepts_continuation(item.generation) {
            return Err(JournalError::StaleAccount.into());
        }
        Ok(item)
    }
    fn check_receipt(
        &self,
        conversation: &ConversationId,
        message: &MessageId,
        receipt: &DeliveryReceipt,
    ) -> Result<(), SessionError> {
        if receipt.account_id == self.actor.account_id
            && &receipt.conversation_id == conversation
            && &receipt.message_id == message
        {
            Ok(())
        } else {
            Err(SessionError::InvalidReceipt)
        }
    }
    fn next_reconciliation(&mut self) -> Result<ReconciliationId, SessionError> {
        // Une réouverture peut recharger un token issu d'un ancien compteur.
        // Le propriétaire est nouveau, et le token durable doit aussi tourner.
        self.journal_mut()?;
        let journal = self.journal.as_ref().ok_or(SessionError::ReloadRequired)?;
        loop {
            let number = next_counter(&mut self.reconciliation_counter)?;
            let candidate = ReconciliationId::new(format!("lookup-{number}"))
                .map_err(|_| SessionError::CounterExhausted)?;
            if !journal.committed().any(|item| matches!(
                &item.state,
                SubmissionState::OutcomeUnknown { reconciliation } if reconciliation == &candidate
            )) {
                return Ok(candidate);
            }
        }
    }
    fn journal_mut(&mut self) -> Result<&mut StoredJournal, SessionError> {
        if self.journal.is_none() {
            self.owner = Arc::new(());
            self.journal = Some(self.store.load_journal()?);
        }
        self.ensure_active()?;
        self.journal.as_mut().ok_or(SessionError::ReloadRequired)
    }
    fn persist(&mut self) -> Result<(), SessionError> {
        let journal = self.journal.as_mut().ok_or(SessionError::ReloadRequired)?;
        if let Err(save) = self.store.save_journal(journal) {
            self.journal = None;
            self.owner = Arc::new(());
            return match self.store.load_journal() {
                Ok(journal) => {
                    self.journal = Some(journal);
                    Err(SessionError::Store(save))
                }
                Err(reload) => Err(SessionError::ReloadFailed {
                    save: Box::new(save),
                    reload: Box::new(reload),
                }),
            };
        }
        Ok(())
    }
}

fn next_counter(counter: &mut u64) -> Result<u64, SessionError> {
    let next = counter
        .checked_add(1)
        .ok_or(SessionError::CounterExhausted)?;
    *counter = next;
    Ok(next)
}
