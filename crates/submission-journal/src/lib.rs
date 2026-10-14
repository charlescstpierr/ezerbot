//! Journal durable d'une soumission jusqu'à l'écho du transcript.
//!
//! La durabilité est simulée en mémoire. Les enregistrements validés vivent
//! dans un `Vec`. La transaction ouverte tient dans une zone non validée.
//! [`Journal::crash_and_recover`] jette cette zone et recharge la projection
//! consultable depuis les seuls enregistrements validés.

#![forbid(unsafe_code)]

use std::fmt;

use grokbot_domain::{
    AccountGeneration, AccountId, AttemptId, AuthenticatedActor, ConversationId, DeliveryReceipt,
    MessageId, ReconciliationId, Refusal, SubmissionState, TranscriptEntryId,
};

/// Brouillon complet écrit avant l'envoi.
///
/// La clé durable est `(account_id, conversation_id, message_id)`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreparedMessage {
    pub actor: AuthenticatedActor,
    pub conversation_id: ConversationId,
    pub message_id: MessageId,
    pub body: String,
}

/// Décision à prendre avant de rejouer un envoi déjà journalisé.
///
/// Consulter cette décision ne crée pas un second message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RetryDecision {
    /// Pas encore accepté : l'appelant peut renvoyer le même message_id.
    ResendSameId,
    /// Déjà accepté ou écho : ne pas renvoyer.
    AlreadyDurable { state: SubmissionState },
    /// Effet incertain : ne pas renvoyer à l'aveugle.
    NeedsReconciliation { state: SubmissionState },
}

/// Erreur du journal. Aucune variante ne modifie l'état déjà validé.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum JournalError {
    /// Aucune soumission pour cette clé.
    NotFound,
    /// Même clé, corps différent. L'enregistrement existant est conservé.
    Conflict,
    /// La génération du compte ne correspond plus à celle de la soumission.
    StaleAccount,
    /// Le domaine refuse la transition demandée.
    IllegalTransition(String),
}

impl fmt::Display for JournalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("soumission introuvable"),
            Self::Conflict => formatter
                .write_str("conflit : cette clé de message existe déjà avec un autre corps"),
            Self::StaleAccount => {
                formatter.write_str("génération de compte périmée : réponse ignorée")
            }
            Self::IllegalTransition(detail) => {
                write!(formatter, "transition illégale : {detail}")
            }
        }
    }
}

impl std::error::Error for JournalError {}

/// Soumission validée, rechargée depuis un stockage.
///
/// L'image ne contient pas la zone non validée : une coupure l'a déjà jetée.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommittedSubmission {
    /// Compte propriétaire de la clé.
    pub account_id: AccountId,
    /// Génération capturée à la préparation.
    pub generation: AccountGeneration,
    /// Conversation de la clé.
    pub conversation_id: ConversationId,
    /// Identifiant stable du message.
    pub message_id: MessageId,
    /// Corps utilisé pour l'idempotence.
    pub body: String,
    /// Dernier état validé.
    pub state: SubmissionState,
}

/// Image d'une soumission. Le corps sert à détecter un conflit d'idempotence.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Record {
    account_id: AccountId,
    conversation_id: ConversationId,
    message_id: MessageId,
    body: String,
    /// Génération capturée à la préparation. Les réponses ultérieures doivent
    /// être acceptées par `AuthenticatedActor::accepts_continuation`.
    generation: AccountGeneration,
    state: SubmissionState,
}

impl Record {
    fn matches(
        &self,
        account: &AccountId,
        conversation: &ConversationId,
        message: &MessageId,
    ) -> bool {
        &self.account_id == account
            && &self.conversation_id == conversation
            && &self.message_id == message
    }

    fn same_key(&self, other: &Self) -> bool {
        self.matches(&other.account_id, &other.conversation_id, &other.message_id)
    }
}

/// Journal d'envoi. Les champs restent privés : l'état consultable passe par [`Journal::get`].
#[derive(Debug)]
pub struct Journal {
    /// Enregistrements dont le commit a réussi, du plus ancien au plus récent.
    committed: Vec<Record>,
    /// Étapes de la transaction ouverte. Vide hors transaction.
    uncommitted: Vec<Record>,
    /// Projection consultable. Elle suit la transaction en cours, puis elle est
    /// reconstruite depuis `committed` au commit et après un crash.
    projection: Vec<Record>,
}

impl Default for Journal {
    fn default() -> Self {
        Self::new()
    }
}

impl Journal {
    /// Journal vide : aucun enregistrement validé, aucune transaction ouverte.
    pub fn new() -> Self {
        Self {
            committed: Vec::new(),
            uncommitted: Vec::new(),
            projection: Vec::new(),
        }
    }

    /// Écrit le brouillon dans une transaction validée.
    ///
    /// La transaction passe par `Prepared`, puis `Queued`, et ne valide qu'un
    /// seul enregistrement : l'état `Queued`. Même clé et même corps : retourne
    /// l'état existant sans second enregistrement. Même clé et corps différent :
    /// [`JournalError::Conflict`].
    pub fn prepare(&mut self, message: PreparedMessage) -> Result<SubmissionState, JournalError> {
        self.abandon_open_transaction();

        if let Some(existing) = self.lookup(
            &message.actor.account_id,
            &message.conversation_id,
            &message.message_id,
        ) {
            self.ensure_continuation(&message.actor, existing.generation)?;
            if existing.body == message.body {
                return Ok(existing.state.clone());
            }
            return Err(JournalError::Conflict);
        }

        let mut record = Record {
            account_id: message.actor.account_id,
            conversation_id: message.conversation_id,
            message_id: message.message_id,
            body: message.body,
            generation: message.actor.generation,
            state: SubmissionState::Prepared,
        };
        self.stage(record.clone());

        let queued = match record.state.clone().queue() {
            Ok(state) => state,
            Err(err) => {
                self.crash_and_recover();
                return Err(illegal(err));
            }
        };
        record.state = queued.clone();
        self.stage(record);
        self.commit()?;
        Ok(queued)
    }

    /// Passe de `Queued` à `Dispatching` pour cette tentative, puis valide la transaction.
    pub fn dispatch(
        &mut self,
        actor: &AuthenticatedActor,
        conversation_id: &ConversationId,
        message_id: &MessageId,
        attempt: AttemptId,
    ) -> Result<SubmissionState, JournalError> {
        self.abandon_open_transaction();
        self.transit(actor, conversation_id, message_id, true, |state| {
            state.begin_dispatch(attempt)
        })
    }

    /// Enregistre l'acceptation et la valide immédiatement.
    ///
    /// Répéter le même message déjà accepté renvoie l'état existant, donc le
    /// reçu déjà journalisé, sans nouvel enregistrement.
    pub fn observe_accept(
        &mut self,
        actor: &AuthenticatedActor,
        receipt: DeliveryReceipt,
    ) -> Result<SubmissionState, JournalError> {
        self.abandon_open_transaction();
        self.accept_receipt(actor, receipt, true)
    }

    /// Passe à `Echoed` depuis `AcceptedAwaitingEcho` ou `OutcomeUnknown`, puis valide.
    pub fn observe_echo(
        &mut self,
        actor: &AuthenticatedActor,
        conversation_id: &ConversationId,
        message_id: &MessageId,
        entry: TranscriptEntryId,
    ) -> Result<SubmissionState, JournalError> {
        self.abandon_open_transaction();
        self.transit(actor, conversation_id, message_id, true, |state| {
            state.echo(entry)
        })
    }

    /// Passe à `OutcomeUnknown` depuis `Dispatching` ou renouvelle son token, puis valide.
    /// Une acceptation validée reste acquise même si l'écho tarde.
    pub fn mark_outcome_unknown(
        &mut self,
        actor: &AuthenticatedActor,
        conversation_id: &ConversationId,
        message_id: &MessageId,
        reconciliation: ReconciliationId,
    ) -> Result<SubmissionState, JournalError> {
        self.abandon_open_transaction();
        self.transit(actor, conversation_id, message_id, true, |state| {
            state.mark_unknown(reconciliation)
        })
    }

    /// Enregistre un refus terminal pour la soumission active.
    pub fn observe_refusal(
        &mut self,
        actor: &AuthenticatedActor,
        conversation_id: &ConversationId,
        message_id: &MessageId,
        reason: Refusal,
    ) -> Result<SubmissionState, JournalError> {
        self.abandon_open_transaction();
        self.transit(actor, conversation_id, message_id, true, |state| {
            state.refuse(reason)
        })
    }

    /// Adopte explicitement une génération plus récente pour ce compte.
    ///
    /// La validation de toutes les générations précède toute mutation. Les clés,
    /// les corps, les états et les tokens sont conservés. Les autres comptes ne
    /// sont pas modifiés. Une génération en recul est refusée.
    pub fn adopt_generation(&mut self, actor: &AuthenticatedActor) -> Result<bool, JournalError> {
        self.abandon_open_transaction();
        if self.projection.iter().any(|record| {
            record.account_id == actor.account_id && record.generation > actor.generation
        }) {
            return Err(JournalError::StaleAccount);
        }
        let replacements: Vec<_> = self
            .projection
            .iter()
            .filter(|record| {
                record.account_id == actor.account_id && record.generation < actor.generation
            })
            .cloned()
            .collect();
        let changed = !replacements.is_empty();
        for mut record in replacements {
            record.generation = actor.generation;
            self.stage(record);
            self.commit()?;
        }
        Ok(changed)
    }

    /// Enregistre un statut accepté pour la réconciliation courante.
    ///
    /// Le reçu doit appartenir au compte de l'acteur et viser la clé incertaine.
    /// Une réponse de génération ou de réconciliation périmée est refusée.
    pub fn observe_reconciliation_found(
        &mut self,
        actor: &AuthenticatedActor,
        reconciliation: &ReconciliationId,
        receipt: DeliveryReceipt,
    ) -> Result<SubmissionState, JournalError> {
        self.abandon_open_transaction();
        if receipt.account_id != actor.account_id {
            return Err(JournalError::IllegalTransition(
                "le reçu vise un autre compte que l'acteur".into(),
            ));
        }
        let conversation_id = receipt.conversation_id.clone();
        let message_id = receipt.message_id.clone();
        self.transit(actor, &conversation_id, &message_id, true, |state| {
            state.reconcile_found(reconciliation, receipt)
        })
    }

    /// Réouvre la file après un statut `not-found` de la réconciliation courante.
    ///
    /// Le même identifiant et le même corps sont conservés. Aucune acceptation
    /// ni décision terminale ne peut être annulée par cette réponse.
    pub fn observe_reconciliation_not_found(
        &mut self,
        actor: &AuthenticatedActor,
        conversation_id: &ConversationId,
        message_id: &MessageId,
        reconciliation: &ReconciliationId,
    ) -> Result<SubmissionState, JournalError> {
        self.abandon_open_transaction();
        self.transit(actor, conversation_id, message_id, true, |state| {
            state.reconcile_not_found(reconciliation)
        })
    }

    /// Avant de rejouer un envoi, consulter le statut. Ne crée pas un second message.
    ///
    /// `Prepared` et `Queued` autorisent un renvoi du même
    /// identifiant. `AcceptedAwaitingEcho`, `Echoed` et `Refused` sont déjà
    /// durables. `Dispatching` et `OutcomeUnknown` exigent une réconciliation.
    pub fn reconcile_before_retry(
        &mut self,
        actor: &AuthenticatedActor,
        conversation_id: &ConversationId,
        message_id: &MessageId,
    ) -> Result<RetryDecision, JournalError> {
        let current = self
            .lookup(&actor.account_id, conversation_id, message_id)
            .ok_or(JournalError::NotFound)?;
        self.ensure_continuation(actor, current.generation)?;
        let state = current.state.clone();
        let decision = match &state {
            SubmissionState::Prepared | SubmissionState::Queued => RetryDecision::ResendSameId,
            SubmissionState::AcceptedAwaitingEcho { .. }
            | SubmissionState::Echoed { .. }
            | SubmissionState::Refused { .. } => RetryDecision::AlreadyDurable { state },
            SubmissionState::Dispatching { .. } | SubmissionState::OutcomeUnknown { .. } => {
                RetryDecision::NeedsReconciliation { state }
            }
        };
        Ok(decision)
    }

    /// État consultable de la clé. Après un crash, seuls les enregistrements validés comptent.
    pub fn get(
        &self,
        account: &AccountId,
        conversation: &ConversationId,
        message: &MessageId,
    ) -> Option<SubmissionState> {
        self.lookup(account, conversation, message)
            .map(|record| record.state.clone())
    }

    /// Abandonne la zone non validée et recharge la projection depuis le journal validé.
    pub fn crash_and_recover(&mut self) {
        self.uncommitted.clear();
        self.projection = Self::replay(&self.committed);
    }

    /// Dernier état validé de chaque clé. La zone non validée est exclue.
    pub fn export_committed(&self) -> Vec<CommittedSubmission> {
        Self::replay(&self.committed)
            .into_iter()
            .map(|record| CommittedSubmission {
                account_id: record.account_id,
                generation: record.generation,
                conversation_id: record.conversation_id,
                message_id: record.message_id,
                body: record.body,
                state: record.state,
            })
            .collect()
    }

    /// Reconstruit un journal depuis des états déjà validés.
    ///
    /// Deux images de la même clé se résolvent comme le journal : la dernière
    /// remplace la précédente. Aucune transaction n'est ouverte.
    pub fn from_committed(items: Vec<CommittedSubmission>) -> Self {
        let committed = items
            .into_iter()
            .map(|item| Record {
                account_id: item.account_id,
                conversation_id: item.conversation_id,
                message_id: item.message_id,
                body: item.body,
                generation: item.generation,
                state: item.state,
            })
            .collect::<Vec<_>>();
        let projection = Self::replay(&committed);
        Self {
            committed,
            uncommitted: Vec::new(),
            projection,
        }
    }

    /// Applique l'acceptation dans la zone non validée, puis simule une coupure.
    ///
    /// L'acceptation non commitée disparaît. L'état renvoyé est celui rechargé
    /// depuis les enregistrements validés : après un `Dispatching` commité, il
    /// reste `Dispatching`. Le chemin [`Journal::observe_accept`] commit, lui,
    /// immédiatement.
    pub fn stage_accept_then_crash(
        &mut self,
        actor: &AuthenticatedActor,
        receipt: DeliveryReceipt,
    ) -> Result<SubmissionState, JournalError> {
        self.abandon_open_transaction();
        let account_id = actor.account_id.clone();
        let conversation_id = receipt.conversation_id.clone();
        let message_id = receipt.message_id.clone();

        let staged = self.accept_receipt(actor, receipt, false)?;
        if !matches!(staged, SubmissionState::AcceptedAwaitingEcho { .. }) {
            self.crash_and_recover();
            return Err(JournalError::IllegalTransition(
                "l'acceptation non validée n'a pas atteint AcceptedAwaitingEcho".into(),
            ));
        }

        let visible = self.get(&account_id, &conversation_id, &message_id);
        if visible.as_ref() != Some(&staged) {
            self.crash_and_recover();
            return Err(JournalError::IllegalTransition(
                "l'acceptation non validée n'était pas consultable avant la coupure".into(),
            ));
        }

        self.crash_and_recover();
        self.get(&account_id, &conversation_id, &message_id)
            .ok_or(JournalError::NotFound)
    }

    fn accept_receipt(
        &mut self,
        actor: &AuthenticatedActor,
        receipt: DeliveryReceipt,
        commit: bool,
    ) -> Result<SubmissionState, JournalError> {
        if receipt.account_id != actor.account_id {
            return Err(JournalError::IllegalTransition(
                "le reçu vise un autre compte que l'acteur".into(),
            ));
        }
        let conversation_id = receipt.conversation_id.clone();
        let message_id = receipt.message_id.clone();
        self.transit(actor, &conversation_id, &message_id, commit, |state| {
            state.accept(receipt)
        })
    }

    fn transit<E, F>(
        &mut self,
        actor: &AuthenticatedActor,
        conversation_id: &ConversationId,
        message_id: &MessageId,
        commit: bool,
        change: F,
    ) -> Result<SubmissionState, JournalError>
    where
        E: fmt::Display,
        F: FnOnce(SubmissionState) -> Result<SubmissionState, E>,
    {
        let current = self
            .lookup(&actor.account_id, conversation_id, message_id)
            .cloned()
            .ok_or(JournalError::NotFound)?;
        self.ensure_continuation(actor, current.generation)?;

        let next = change(current.state.clone()).map_err(illegal)?;
        if next == current.state {
            return Ok(next);
        }

        let mut updated = current;
        updated.state = next.clone();
        self.stage(updated);
        if commit {
            self.commit()?;
        }
        Ok(next)
    }

    fn ensure_continuation(
        &self,
        actor: &AuthenticatedActor,
        generation: AccountGeneration,
    ) -> Result<(), JournalError> {
        if actor.accepts_continuation(generation) {
            Ok(())
        } else {
            Err(JournalError::StaleAccount)
        }
    }

    /// Une transaction laissée ouverte est une coupure : on recharge le validé.
    fn abandon_open_transaction(&mut self) {
        if !self.uncommitted.is_empty() {
            self.crash_and_recover();
        }
    }

    fn stage(&mut self, record: Record) {
        upsert(&mut self.projection, record.clone());
        self.uncommitted.push(record);
    }

    fn commit(&mut self) -> Result<(), JournalError> {
        let Some(record) = self.uncommitted.last().cloned() else {
            return Err(JournalError::IllegalTransition(
                "aucune transaction à valider".into(),
            ));
        };
        self.committed.push(record);
        self.uncommitted.clear();
        self.projection = Self::replay(&self.committed);
        Ok(())
    }

    fn lookup(
        &self,
        account: &AccountId,
        conversation: &ConversationId,
        message: &MessageId,
    ) -> Option<&Record> {
        self.projection
            .iter()
            .rev()
            .find(|record| record.matches(account, conversation, message))
    }

    fn replay(committed: &[Record]) -> Vec<Record> {
        let mut projection = Vec::new();
        for record in committed {
            upsert(&mut projection, record.clone());
        }
        projection
    }
}

fn upsert(records: &mut Vec<Record>, record: Record) {
    if let Some(slot) = records.iter_mut().find(|item| item.same_key(&record)) {
        *slot = record;
    } else {
        records.push(record);
    }
}

fn illegal(err: impl fmt::Display) -> JournalError {
    JournalError::IllegalTransition(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use grokbot_domain::Refusal;

    fn must<T, E: fmt::Display>(result: Result<T, E>) -> T {
        match result {
            Ok(value) => value,
            Err(err) => panic!("identifiant rejeté : {err}"),
        }
    }

    fn actor(generation: u64) -> AuthenticatedActor {
        AuthenticatedActor {
            account_id: must(AccountId::new("acct-1")),
            team_id: None,
            generation: AccountGeneration(generation),
        }
    }

    fn conversation() -> ConversationId {
        must(ConversationId::new("conv-1"))
    }

    fn message() -> MessageId {
        must(MessageId::new("msg-1"))
    }

    fn prepared(actor: &AuthenticatedActor, body: &str) -> PreparedMessage {
        PreparedMessage {
            actor: actor.clone(),
            conversation_id: conversation(),
            message_id: message(),
            body: body.to_owned(),
        }
    }

    fn receipt_for(actor: &AuthenticatedActor) -> DeliveryReceipt {
        DeliveryReceipt {
            account_id: actor.account_id.clone(),
            conversation_id: conversation(),
            message_id: message(),
        }
    }

    fn records_for(journal: &Journal) -> usize {
        let account = must(AccountId::new("acct-1"));
        let conversation = conversation();
        let message = message();
        journal
            .committed
            .iter()
            .filter(|record| record.matches(&account, &conversation, &message))
            .count()
    }

    fn state_of(journal: &Journal) -> Option<SubmissionState> {
        journal.get(&must(AccountId::new("acct-1")), &conversation(), &message())
    }

    fn uncertain(owner: &AuthenticatedActor, token: &ReconciliationId) -> Journal {
        let mut journal = Journal::new();
        journal.prepare(prepared(owner, "bonjour")).unwrap();
        journal
            .dispatch(
                owner,
                &conversation(),
                &message(),
                must(AttemptId::new("attempt-1")),
            )
            .unwrap();
        journal
            .mark_outcome_unknown(owner, &conversation(), &message(), token.clone())
            .unwrap();
        journal
    }

    #[test]
    fn nouvelle_reconciliation_invalide_un_ancien_not_found() {
        let owner = actor(1);
        let old = must(ReconciliationId::new("old"));
        let current = must(ReconciliationId::new("current"));
        let mut journal = uncertain(&owner, &old);
        journal
            .mark_outcome_unknown(&owner, &conversation(), &message(), current.clone())
            .unwrap();
        assert!(journal
            .observe_reconciliation_not_found(&owner, &conversation(), &message(), &old)
            .is_err());
        assert_eq!(
            state_of(&journal),
            Some(SubmissionState::OutcomeUnknown {
                reconciliation: current
            })
        );
    }

    #[test]
    fn adoption_generation_valide_tout_le_compte_avant_modification() {
        let mut journal = Journal::new();
        journal.prepare(prepared(&actor(1), "bonjour")).unwrap();
        let mut newer = prepared(&actor(3), "newer body");
        newer.message_id = must(MessageId::new("newer-message"));
        journal.prepare(newer).unwrap();
        let before = journal.export_committed();
        assert_eq!(
            journal.adopt_generation(&actor(2)),
            Err(JournalError::StaleAccount)
        );
        assert_eq!(journal.export_committed(), before);
        journal.crash_and_recover();
        assert_eq!(journal.export_committed(), before);
    }

    #[test]
    fn refus_survit_au_replay_et_ne_permet_pas_de_renvoi() {
        let owner = actor(1);
        let token = must(ReconciliationId::new("current"));
        let mut journal = uncertain(&owner, &token);
        journal
            .observe_refusal(&owner, &conversation(), &message(), Refusal::Policy)
            .unwrap();
        journal.crash_and_recover();
        assert_eq!(
            state_of(&journal),
            Some(SubmissionState::Refused {
                reason: Refusal::Policy
            })
        );
        assert!(journal
            .dispatch(
                &owner,
                &conversation(),
                &message(),
                must(AttemptId::new("new-send"))
            )
            .is_err());
        assert!(journal
            .observe_reconciliation_not_found(&owner, &conversation(), &message(), &token)
            .is_err());
    }

    #[test]
    fn repetition_du_meme_corps_ne_cree_qu_un_enregistrement() {
        let mut journal = Journal::new();
        let actor = actor(1);
        let first = journal.prepare(prepared(&actor, "bonjour")).unwrap();
        let second = journal.prepare(prepared(&actor, "bonjour")).unwrap();

        assert_eq!(first, SubmissionState::Queued);
        assert_eq!(second, first);
        assert_eq!(records_for(&journal), 1);
        assert!(journal.uncommitted.is_empty());

        journal.crash_and_recover();
        assert_eq!(records_for(&journal), 1);
        assert_eq!(state_of(&journal), Some(SubmissionState::Queued));
    }

    #[test]
    fn meme_identifiant_corps_different_est_un_conflit() {
        let mut journal = Journal::new();
        let actor = actor(1);
        let queued = journal.prepare(prepared(&actor, "bonjour")).unwrap();

        let err = journal
            .prepare(prepared(&actor, "autre corps"))
            .unwrap_err();

        assert_eq!(err, JournalError::Conflict);
        assert_eq!(records_for(&journal), 1);
        assert_eq!(state_of(&journal), Some(queued));
    }

    #[test]
    fn incertitude_puis_echo_depuis_outcome_unknown() {
        let mut journal = Journal::new();
        let actor = actor(1);
        journal.prepare(prepared(&actor, "bonjour")).unwrap();
        let attempt = must(AttemptId::new("attempt-1"));
        let dispatching = journal
            .dispatch(&actor, &conversation(), &message(), attempt)
            .unwrap();
        assert!(matches!(dispatching, SubmissionState::Dispatching { .. }));
        assert_eq!(
            journal
                .reconcile_before_retry(&actor, &conversation(), &message())
                .unwrap(),
            RetryDecision::NeedsReconciliation { state: dispatching }
        );

        let reconciliation = must(ReconciliationId::new("recon-1"));
        let unknown = journal
            .mark_outcome_unknown(&actor, &conversation(), &message(), reconciliation.clone())
            .unwrap();
        assert_eq!(unknown, SubmissionState::OutcomeUnknown { reconciliation });
        assert_eq!(
            journal
                .reconcile_before_retry(&actor, &conversation(), &message())
                .unwrap(),
            RetryDecision::NeedsReconciliation {
                state: unknown.clone(),
            }
        );

        let entry = must(TranscriptEntryId::new("entry-1"));
        let echoed = journal
            .observe_echo(&actor, &conversation(), &message(), entry.clone())
            .unwrap();
        assert_eq!(echoed, SubmissionState::Echoed { entry });
        assert_eq!(
            journal
                .reconcile_before_retry(&actor, &conversation(), &message())
                .unwrap(),
            RetryDecision::AlreadyDurable { state: echoed }
        );
    }

    #[test]
    fn accept_commite_survit_au_crash_et_bloque_le_renvoi() {
        let mut journal = Journal::new();
        let actor = actor(1);
        journal.prepare(prepared(&actor, "bonjour")).unwrap();
        journal
            .dispatch(
                &actor,
                &conversation(),
                &message(),
                must(AttemptId::new("attempt-1")),
            )
            .unwrap();

        let accepted = journal.observe_accept(&actor, receipt_for(&actor)).unwrap();
        assert!(matches!(
            accepted,
            SubmissionState::AcceptedAwaitingEcho { .. }
        ));
        let records_after_accept = records_for(&journal);

        journal.crash_and_recover();

        assert_eq!(state_of(&journal), Some(accepted.clone()));
        assert_eq!(records_for(&journal), records_after_accept);
        assert_eq!(
            journal
                .reconcile_before_retry(&actor, &conversation(), &message())
                .unwrap(),
            RetryDecision::AlreadyDurable { state: accepted }
        );
    }

    #[test]
    fn accept_non_commite_disparait_au_crash() {
        let mut journal = Journal::new();
        let actor = actor(1);
        journal.prepare(prepared(&actor, "bonjour")).unwrap();
        let attempt = must(AttemptId::new("attempt-1"));
        journal
            .dispatch(&actor, &conversation(), &message(), attempt.clone())
            .unwrap();
        let records_before = records_for(&journal);

        let recovered = journal
            .stage_accept_then_crash(&actor, receipt_for(&actor))
            .unwrap();

        let expected = SubmissionState::Dispatching { attempt };
        assert_eq!(recovered, expected);
        assert_eq!(state_of(&journal), Some(expected));
        assert_eq!(records_for(&journal), records_before);
        assert!(journal.uncommitted.is_empty());
        assert!(journal.committed.iter().all(|record| {
            !matches!(record.state, SubmissionState::AcceptedAwaitingEcho { .. })
        }));
    }

    #[test]
    fn generation_perimee_n_accepte_pas_et_la_generation_origine_reste_valide() {
        let mut journal = Journal::new();
        let original = actor(1);
        let revoked = actor(2);
        journal.prepare(prepared(&original, "bonjour")).unwrap();
        let attempt = must(AttemptId::new("attempt-1"));
        journal
            .dispatch(&original, &conversation(), &message(), attempt.clone())
            .unwrap();
        let dispatching = SubmissionState::Dispatching { attempt };

        let stale_accept = journal
            .observe_accept(&revoked, receipt_for(&revoked))
            .unwrap_err();
        assert_eq!(stale_accept, JournalError::StaleAccount);
        assert_eq!(state_of(&journal), Some(dispatching.clone()));

        let stale_unknown = journal
            .mark_outcome_unknown(
                &revoked,
                &conversation(),
                &message(),
                must(ReconciliationId::new("recon-1")),
            )
            .unwrap_err();
        assert_eq!(stale_unknown, JournalError::StaleAccount);
        assert_eq!(state_of(&journal), Some(dispatching.clone()));

        let accepted = journal
            .observe_accept(&original, receipt_for(&original))
            .unwrap();
        assert!(matches!(
            accepted,
            SubmissionState::AcceptedAwaitingEcho { .. }
        ));
        assert_eq!(state_of(&journal), Some(accepted));
    }

    #[test]
    fn une_acceptation_commise_ne_redevient_pas_incertaine() {
        let mut journal = Journal::new();
        let actor = actor(1);
        journal.prepare(prepared(&actor, "bonjour")).unwrap();
        journal
            .dispatch(
                &actor,
                &conversation(),
                &message(),
                must(AttemptId::new("attempt-1")),
            )
            .unwrap();
        let accepted = journal.observe_accept(&actor, receipt_for(&actor)).unwrap();

        let reconciliation = must(ReconciliationId::new("recon-accept"));
        let err = journal
            .mark_outcome_unknown(&actor, &conversation(), &message(), reconciliation.clone())
            .unwrap_err();

        assert!(matches!(err, JournalError::IllegalTransition(_)));
        assert_eq!(state_of(&journal), Some(accepted.clone()));
        assert_eq!(
            journal
                .reconcile_before_retry(&actor, &conversation(), &message())
                .unwrap(),
            RetryDecision::AlreadyDurable { state: accepted }
        );
    }

    #[test]
    fn repetition_d_acceptation_ne_duplique_pas_le_recu() {
        let mut journal = Journal::new();
        let actor = actor(1);
        journal.prepare(prepared(&actor, "bonjour")).unwrap();
        journal
            .dispatch(
                &actor,
                &conversation(),
                &message(),
                must(AttemptId::new("attempt-1")),
            )
            .unwrap();

        let first = journal.observe_accept(&actor, receipt_for(&actor)).unwrap();
        let records = records_for(&journal);
        let second = journal.observe_accept(&actor, receipt_for(&actor)).unwrap();

        assert_eq!(first, second);
        assert_eq!(records_for(&journal), records);
        assert!(matches!(
            second,
            SubmissionState::AcceptedAwaitingEcho { .. }
        ));
    }

    #[test]
    fn refus_et_prepare_guident_le_rejeu_sans_creer_de_message() {
        let mut journal = Journal::new();
        let owner = actor(1);
        journal.committed.push(Record {
            account_id: owner.account_id.clone(),
            conversation_id: conversation(),
            message_id: message(),
            body: "bonjour".into(),
            generation: owner.generation,
            state: SubmissionState::Refused {
                reason: Refusal::Conflict,
            },
        });
        journal.crash_and_recover();

        let decision = journal
            .reconcile_before_retry(&owner, &conversation(), &message())
            .unwrap();
        assert_eq!(
            decision,
            RetryDecision::AlreadyDurable {
                state: SubmissionState::Refused {
                    reason: Refusal::Conflict,
                },
            }
        );
        assert_eq!(records_for(&journal), 1);

        journal.committed.push(Record {
            account_id: owner.account_id.clone(),
            conversation_id: conversation(),
            message_id: must(MessageId::new("msg-prepare")),
            body: "brouillon".into(),
            generation: AccountGeneration(1),
            state: SubmissionState::Prepared,
        });
        journal.crash_and_recover();
        let prepared_decision = journal
            .reconcile_before_retry(
                &owner,
                &conversation(),
                &must(MessageId::new("msg-prepare")),
            )
            .unwrap();
        assert_eq!(prepared_decision, RetryDecision::ResendSameId);
    }

    #[test]
    fn export_puis_rechargement_conserve_l_etat_valide() {
        let owner = actor(1);
        let mut journal = Journal::new();
        journal.prepare(prepared(&owner, "bonjour")).unwrap();
        journal
            .dispatch(
                &owner,
                &conversation(),
                &message(),
                must(AttemptId::new("attempt-1")),
            )
            .unwrap();
        let mut loaded = Journal::from_committed(journal.export_committed());
        assert_eq!(state_of(&loaded), state_of(&journal));
        assert!(loaded.uncommitted.is_empty());
        let again = loaded.prepare(prepared(&owner, "bonjour")).unwrap();
        assert_eq!(again, state_of(&journal).unwrap());
        assert!(matches!(
            loaded
                .reconcile_before_retry(&owner, &conversation(), &message())
                .unwrap(),
            RetryDecision::NeedsReconciliation {
                state: SubmissionState::Dispatching { .. }
            }
        ));
    }

    #[test]
    fn statut_found_recharge_conserve_le_recu_et_bloque_le_renvoi() {
        let owner = actor(1);
        let token = must(ReconciliationId::new("recon-1"));
        let mut loaded = Journal::from_committed(uncertain(&owner, &token).export_committed());
        let receipt = receipt_for(&owner);
        let accepted = loaded
            .observe_reconciliation_found(&owner, &token, receipt.clone())
            .unwrap();
        assert_eq!(accepted, SubmissionState::AcceptedAwaitingEcho { receipt });

        let mut restarted = Journal::from_committed(loaded.export_committed());
        assert_eq!(
            restarted
                .reconcile_before_retry(&owner, &conversation(), &message())
                .unwrap(),
            RetryDecision::AlreadyDurable { state: accepted }
        );
        assert!(restarted
            .observe_reconciliation_not_found(&owner, &conversation(), &message(), &token)
            .is_err());
        assert_eq!(restarted.export_committed().len(), 1);
    }

    #[test]
    fn statut_not_found_recharge_reutilise_le_meme_nonce_et_corps() {
        let owner = actor(1);
        let token = must(ReconciliationId::new("recon-1"));
        let mut loaded = Journal::from_committed(uncertain(&owner, &token).export_committed());
        assert_eq!(
            loaded
                .observe_reconciliation_not_found(&owner, &conversation(), &message(), &token)
                .unwrap(),
            SubmissionState::Queued
        );

        let mut restarted = Journal::from_committed(loaded.export_committed());
        assert_eq!(
            restarted
                .reconcile_before_retry(&owner, &conversation(), &message())
                .unwrap(),
            RetryDecision::ResendSameId
        );
        let items = restarted.export_committed();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].message_id.as_str(), "msg-1");
        assert_eq!(items[0].body, "bonjour");
        assert_eq!(
            restarted.prepare(prepared(&owner, "autre texte")),
            Err(JournalError::Conflict)
        );
        restarted
            .dispatch(
                &owner,
                &conversation(),
                &message(),
                must(AttemptId::new("attempt-2")),
            )
            .unwrap();
        assert!(matches!(
            restarted
                .reconcile_before_retry(&owner, &conversation(), &message())
                .unwrap(),
            RetryDecision::NeedsReconciliation { .. }
        ));
    }

    #[test]
    fn une_reponse_de_reconciliation_ancienne_ne_modifie_pas_la_nouvelle_tentative() {
        let owner = actor(1);
        let old = must(ReconciliationId::new("recon-1"));
        let current = must(ReconciliationId::new("recon-2"));
        let mut journal = uncertain(&owner, &old);
        journal
            .observe_reconciliation_not_found(&owner, &conversation(), &message(), &old)
            .unwrap();
        journal
            .dispatch(
                &owner,
                &conversation(),
                &message(),
                must(AttemptId::new("attempt-2")),
            )
            .unwrap();
        journal
            .mark_outcome_unknown(&owner, &conversation(), &message(), current.clone())
            .unwrap();
        let before = journal.export_committed();
        assert!(journal
            .observe_reconciliation_found(&owner, &old, receipt_for(&owner))
            .is_err());
        assert!(journal
            .observe_reconciliation_not_found(&owner, &conversation(), &message(), &old)
            .is_err());
        assert_eq!(journal.export_committed(), before);
        journal
            .observe_reconciliation_found(&owner, &current, receipt_for(&owner))
            .unwrap();
    }

    #[test]
    fn statut_de_reconciliation_refuse_mauvais_compte_generation_et_recu() {
        let owner = actor(1);
        let revoked = actor(2);
        let mut other = owner.clone();
        other.account_id = must(AccountId::new("acct-2"));
        let token = must(ReconciliationId::new("recon-1"));
        let mut journal = uncertain(&owner, &token);
        let before = journal.export_committed();
        for actor in [&revoked, &other] {
            assert!(journal
                .observe_reconciliation_found(actor, &token, receipt_for(&owner))
                .is_err());
            assert!(journal
                .observe_reconciliation_not_found(actor, &conversation(), &message(), &token)
                .is_err());
        }
        let mut wrong_receipt = receipt_for(&owner);
        wrong_receipt.message_id = must(MessageId::new("msg-autre"));
        assert_eq!(
            journal.observe_reconciliation_found(&owner, &token, wrong_receipt),
            Err(JournalError::NotFound)
        );
        assert!(journal
            .observe_reconciliation_found(&owner, &token, receipt_for(&other))
            .is_err());
        assert_eq!(journal.export_committed(), before);
    }

    #[test]
    fn un_refus_recharge_ne_peut_pas_etre_rejoue_par_un_statut_tardif() {
        let owner = actor(1);
        let token = must(ReconciliationId::new("recon-1"));
        let mut items = uncertain(&owner, &token).export_committed();
        items[0].state = SubmissionState::Refused {
            reason: Refusal::Policy,
        };
        let mut loaded = Journal::from_committed(items.clone());
        assert!(loaded
            .observe_reconciliation_found(&owner, &token, receipt_for(&owner))
            .is_err());
        assert!(loaded
            .observe_reconciliation_not_found(&owner, &conversation(), &message(), &token)
            .is_err());
        assert!(matches!(
            loaded
                .reconcile_before_retry(&owner, &conversation(), &message())
                .unwrap(),
            RetryDecision::AlreadyDurable {
                state: SubmissionState::Refused { .. }
            }
        ));
        assert_eq!(loaded.export_committed(), items);
    }

    #[test]
    fn une_preparation_perimee_ne_reutilise_pas_la_cle_existante() {
        let owner = actor(1);
        let token = must(ReconciliationId::new("recon-1"));
        let mut journal = uncertain(&owner, &token);
        let before = journal.export_committed();
        assert_eq!(
            journal.prepare(prepared(&actor(2), "bonjour")),
            Err(JournalError::StaleAccount)
        );
        assert_eq!(journal.export_committed(), before);
    }
}
