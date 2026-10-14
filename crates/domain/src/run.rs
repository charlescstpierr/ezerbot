//! Exécution, interruption et décision d'approbation.

use crate::id::{ActionGrantId, ApprovalId, CheckpointId, HumanInputId};
use crate::submission::TransitionError;

/// Raison pour laquelle une exécution est en cours d'annulation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InterruptionReason {
    /// Interruption demandée par l'utilisateur.
    User,
    /// Exécution remplacée par une autre.
    Superseded,
    /// Compte révoqué.
    AccountRevoked,
}

/// Cause d'échec d'une exécution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunFailure {
    /// Échec du modèle.
    Model,
    /// Échec d'un outil.
    Tool,
    /// Échec de politique.
    Policy,
    /// Échec interne.
    Internal,
}

/// Sort réservé à un résultat d'outil, sans modifier l'exécution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectDisposition {
    /// Le résultat d'outil s'applique à l'exécution.
    Applied,
    /// Le résultat arrive après interruption ou terminaison, et il est ignoré.
    IgnoredAfterInterruption,
}

/// Cycle de vie d'une exécution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RunState {
    /// En attente de démarrage.
    Queued,
    /// En cours, au point de contrôle indiqué.
    Running {
        /// Point de contrôle courant.
        checkpoint: CheckpointId,
    },
    /// En attente d'une approbation.
    AwaitingApproval {
        /// Demande d'approbation ouverte.
        approval: ApprovalId,
    },
    /// En attente d'une saisie humaine.
    AwaitingHumanInput {
        /// Demande de saisie ouverte.
        request: HumanInputId,
    },
    /// Annulation demandée, pas encore terminée.
    Cancelling {
        /// Raison de l'annulation.
        reason: InterruptionReason,
    },
    /// Terminée avec succès.
    Succeeded,
    /// Terminée en échec.
    Failed {
        /// Cause de l'échec.
        error: RunFailure,
    },
    /// Annulée au point de contrôle indiqué.
    Cancelled {
        /// Point de contrôle retenu pour l'annulation.
        checkpoint: CheckpointId,
    },
}

impl RunState {
    /// Indique si un résultat d'outil arrivé maintenant serait pris en compte.
    ///
    /// `Applied` pour `Queued`, `Running`, `AwaitingApproval` et `AwaitingHumanInput`.
    /// `IgnoredAfterInterruption` pour `Cancelling`, `Succeeded`, `Failed` et `Cancelled`.
    /// L'état n'est pas modifié : une réponse tardive ne fait pas réussir une exécution annulée.
    pub fn dispose_tool_result(&self) -> EffectDisposition {
        match self {
            Self::Queued
            | Self::Running { .. }
            | Self::AwaitingApproval { .. }
            | Self::AwaitingHumanInput { .. } => EffectDisposition::Applied,
            Self::Cancelling { .. }
            | Self::Succeeded
            | Self::Failed { .. }
            | Self::Cancelled { .. } => EffectDisposition::IgnoredAfterInterruption,
        }
    }

    /// `Running`, `AwaitingApproval` ou `AwaitingHumanInput` → `Cancelling`.
    ///
    /// Si l'état est déjà `Cancelling` pour la même raison, rend `Ok(self)`.
    pub fn begin_cancel(self, reason: InterruptionReason) -> Result<Self, TransitionError> {
        if let Self::Cancelling { reason: current } = &self {
            if current == &reason {
                return Ok(self);
            }
        }
        match self {
            Self::Running { .. }
            | Self::AwaitingApproval { .. }
            | Self::AwaitingHumanInput { .. } => Ok(Self::Cancelling { reason }),
            other => Err(other.reject("begin_cancel")),
        }
    }

    /// `Cancelling` → `Cancelled`.
    pub fn finish_cancel(self, checkpoint: CheckpointId) -> Result<Self, TransitionError> {
        match self {
            Self::Cancelling { .. } => Ok(Self::Cancelled { checkpoint }),
            other => Err(other.reject("finish_cancel")),
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Self::Queued => "RunState::Queued",
            Self::Running { .. } => "RunState::Running",
            Self::AwaitingApproval { .. } => "RunState::AwaitingApproval",
            Self::AwaitingHumanInput { .. } => "RunState::AwaitingHumanInput",
            Self::Cancelling { .. } => "RunState::Cancelling",
            Self::Succeeded => "RunState::Succeeded",
            Self::Failed { .. } => "RunState::Failed",
            Self::Cancelled { .. } => "RunState::Cancelled",
        }
    }

    fn reject(&self, operation: &'static str) -> TransitionError {
        TransitionError::new(operation, self.name())
    }
}

/// Décision d'approbation d'une action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    /// Action autorisée, liée à un grant.
    Allow {
        /// Autorisation accordée.
        grant: ActionGrantId,
    },
    /// Décision différée, en attente d'une approbation.
    Ask {
        /// Demande d'approbation à présenter.
        approval: ApprovalId,
    },
    /// Action refusée.
    Deny {
        /// Motif du refus.
        reason: String,
    },
}
