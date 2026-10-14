//! Soumission, époque de réplica et transitions associées.

use std::fmt;

use crate::id::{
    AccountId, AttemptId, BootId, ConversationId, MessageId, ReconciliationId, ServerGeneration,
    TranscriptEntryId, UpdateSequence,
};

/// Époque d'un réplica.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaEpoch {
    /// Démarrage qui a ouvert cette époque.
    pub boot_id: BootId,
    /// Génération du serveur associée à l'époque.
    pub server_generation: ServerGeneration,
    /// Compteur de recovery de l'époque.
    pub recovery: u32,
}

/// Curseur de reprise d'un flux de mises à jour.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResumeCursor {
    /// Époque à laquelle le curseur se rapporte.
    pub epoch: ReplicaEpoch,
    /// Dernière séquence couverte.
    pub covered_through: UpdateSequence,
    /// Indique si le curseur est complet.
    pub complete: bool,
}

/// Accusé de remise d'un message dans une conversation, pour un compte.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryReceipt {
    /// Compte destinataire de l'accusé.
    pub account_id: AccountId,
    /// Conversation dans laquelle le message a été remis.
    pub conversation_id: ConversationId,
    /// Message remis.
    pub message_id: MessageId,
}

/// Motif de refus d'une soumission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Conflit avec un état déjà retenu.
    Conflict,
    /// La conversation est inconnue.
    UnknownConversation,
    /// La génération du compte ne correspond plus.
    StaleAccountGeneration,
    /// Refus de politique.
    Policy,
    /// Autre motif, porté par le message.
    Other(String),
}

/// Cycle de vie d'une soumission.
///
/// [`SubmissionState::Echoed`] et [`SubmissionState::Refused`] sont terminaux :
/// toute transition ultérieure échoue.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SubmissionState {
    /// Soumission construite, pas encore en file.
    Prepared,
    /// En attente d'envoi.
    Queued,
    /// Envoi en cours pour une tentative.
    Dispatching {
        /// Tentative d'envoi en cours.
        attempt: AttemptId,
    },
    /// Acceptée, en attente de l'écho dans la transcription.
    AcceptedAwaitingEcho {
        /// Accusé qui a accepté la soumission.
        receipt: DeliveryReceipt,
    },
    /// Écho reçu dans la transcription.
    Echoed {
        /// Entrée de transcription qui porte l'écho.
        entry: TranscriptEntryId,
    },
    /// Refusée.
    Refused {
        /// Motif du refus.
        reason: Refusal,
    },
    /// Issue inconnue, en cours de réconciliation.
    OutcomeUnknown {
        /// Réconciliation ouverte pour retrouver l'issue.
        reconciliation: ReconciliationId,
    },
}

/// Transition refusée. Le message nomme l'état courant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TransitionError {
    operation: &'static str,
    state: &'static str,
}

impl TransitionError {
    pub(crate) fn new(operation: &'static str, state: &'static str) -> Self {
        Self { operation, state }
    }
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "transition « {} » refusée depuis l'état {}",
            self.operation, self.state
        )
    }
}

impl std::error::Error for TransitionError {}

impl SubmissionState {
    /// `Prepared` → `Queued`.
    pub fn queue(self) -> Result<Self, TransitionError> {
        match self {
            Self::Prepared => Ok(Self::Queued),
            other => Err(other.reject("queue")),
        }
    }

    /// `Queued` → `Dispatching`.
    pub fn begin_dispatch(self, attempt: AttemptId) -> Result<Self, TransitionError> {
        match self {
            Self::Queued => Ok(Self::Dispatching { attempt }),
            other => Err(other.reject("begin_dispatch")),
        }
    }

    /// `Dispatching` → `AcceptedAwaitingEcho`.
    ///
    /// Si l'état est déjà `AcceptedAwaitingEcho` avec le même reçu, rend `Ok(self)`.
    /// Un reçu différent est refusé.
    pub fn accept(self, receipt: DeliveryReceipt) -> Result<Self, TransitionError> {
        if let Self::AcceptedAwaitingEcho { receipt: current } = &self {
            if current == &receipt {
                return Ok(self);
            }
        }
        match self {
            Self::Dispatching { .. } => Ok(Self::AcceptedAwaitingEcho { receipt }),
            other => Err(other.reject("accept")),
        }
    }

    /// `AcceptedAwaitingEcho` ou `OutcomeUnknown` → `Echoed`.
    pub fn echo(self, entry: TranscriptEntryId) -> Result<Self, TransitionError> {
        match self {
            Self::AcceptedAwaitingEcho { .. } | Self::OutcomeUnknown { .. } => {
                Ok(Self::Echoed { entry })
            }
            other => Err(other.reject("echo")),
        }
    }

    /// `Dispatching` ou `OutcomeUnknown` → `Refused`.
    pub fn refuse(self, reason: Refusal) -> Result<Self, TransitionError> {
        match self {
            Self::Dispatching { .. } | Self::OutcomeUnknown { .. } => Ok(Self::Refused { reason }),
            other => Err(other.reject("refuse")),
        }
    }

    /// `Dispatching` ou `OutcomeUnknown` → `OutcomeUnknown` avec un nouveau token.
    /// Une acceptation acquise n'est pas effacée par l'absence d'écho.
    pub fn mark_unknown(self, reconciliation: ReconciliationId) -> Result<Self, TransitionError> {
        match self {
            Self::Dispatching { .. } | Self::OutcomeUnknown { .. } => {
                Ok(Self::OutcomeUnknown { reconciliation })
            }
            other => Err(other.reject("mark_unknown")),
        }
    }

    /// `OutcomeUnknown` → `AcceptedAwaitingEcho`, avec le token courant.
    pub fn reconcile_found(
        self,
        reconciliation: &ReconciliationId,
        receipt: DeliveryReceipt,
    ) -> Result<Self, TransitionError> {
        match self {
            Self::OutcomeUnknown {
                reconciliation: current,
            } if &current == reconciliation => Ok(Self::AcceptedAwaitingEcho { receipt }),
            other => Err(other.reject("reconcile_found")),
        }
    }

    /// `OutcomeUnknown` → `Queued` après un statut `not-found` du token courant.
    pub fn reconcile_not_found(
        self,
        reconciliation: &ReconciliationId,
    ) -> Result<Self, TransitionError> {
        match self {
            Self::OutcomeUnknown {
                reconciliation: current,
            } if &current == reconciliation => Ok(Self::Queued),
            other => Err(other.reject("reconcile_not_found")),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Prepared => "SubmissionState::Prepared",
            Self::Queued => "SubmissionState::Queued",
            Self::Dispatching { .. } => "SubmissionState::Dispatching",
            Self::AcceptedAwaitingEcho { .. } => "SubmissionState::AcceptedAwaitingEcho",
            Self::Echoed { .. } => "SubmissionState::Echoed",
            Self::Refused { .. } => "SubmissionState::Refused",
            Self::OutcomeUnknown { .. } => "SubmissionState::OutcomeUnknown",
        }
    }

    fn reject(&self, operation: &'static str) -> TransitionError {
        TransitionError::new(operation, self.name())
    }
}
