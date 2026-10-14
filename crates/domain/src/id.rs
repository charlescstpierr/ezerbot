//! Identifiants opaques et compteurs copiables.

use std::fmt;

/// Erreur produite lorsqu'un identifiant est une chaîne vide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmptyId;

impl fmt::Display for EmptyId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("identifiant vide")
    }
}

impl std::error::Error for EmptyId {}

pub(crate) fn require_non_empty(value: impl Into<String>) -> Result<String, EmptyId> {
    let value = value.into();
    if value.is_empty() {
        Err(EmptyId)
    } else {
        Ok(value)
    }
}

macro_rules! opaque_id {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// Construit l'identifiant à partir d'une chaîne non vide.
            pub fn new(value: impl Into<String>) -> Result<Self, EmptyId> {
                Ok(Self(require_non_empty(value)?))
            }

            /// Retourne la valeur textuelle de l'identifiant.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

opaque_id!(
    /// Identifiant opaque d'un compte.
    AccountId
);
opaque_id!(
    /// Identifiant opaque d'une équipe.
    TeamId
);
opaque_id!(
    /// Identifiant opaque d'un bot.
    BotId
);
opaque_id!(
    /// Identifiant opaque d'une conversation.
    ConversationId
);
opaque_id!(
    /// Identifiant opaque d'une session.
    SessionId
);
opaque_id!(
    /// Identifiant opaque d'une exécution.
    RunId
);
opaque_id!(
    /// Identifiant opaque d'un message.
    MessageId
);
opaque_id!(
    /// Identifiant opaque d'un appel d'outil.
    ToolCallId
);
opaque_id!(
    /// Identifiant opaque d'une demande d'approbation.
    ApprovalId
);
opaque_id!(
    /// Identifiant opaque d'un ordinateur piloté.
    ComputerId
);
opaque_id!(
    /// Identifiant opaque d'un bail d'écran.
    ScreenLeaseId
);
opaque_id!(
    /// Identifiant opaque d'un démarrage de réplica.
    BootId
);
opaque_id!(
    /// Identifiant opaque d'une tentative d'envoi.
    AttemptId
);
opaque_id!(
    /// Identifiant opaque d'une entrée de transcription.
    TranscriptEntryId
);
opaque_id!(
    /// Identifiant opaque d'une réconciliation.
    ReconciliationId
);
opaque_id!(
    /// Identifiant opaque d'un point de contrôle.
    CheckpointId
);
opaque_id!(
    /// Identifiant opaque d'une demande de saisie humaine.
    HumanInputId
);
opaque_id!(
    /// Identifiant opaque d'une autorisation d'action.
    ActionGrantId
);

/// Génération monotone du serveur de réplica.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ServerGeneration(
    /// Valeur du compteur.
    pub u64,
);

/// Compteur d'entrées.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntrySequence(
    /// Valeur du compteur.
    pub u64,
);

/// Compteur de mises à jour de transcription.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UpdateSequence(
    /// Valeur du compteur.
    pub u64,
);

/// Compteur de réplica.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ReplicaSequence(
    /// Valeur du compteur.
    pub u64,
);
