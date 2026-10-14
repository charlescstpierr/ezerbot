//! Tentatives de récupération liées à une conversation et à leur propriétaire.

use std::{fmt, sync::Arc};

use grokbot_domain::{ConversationId, ReplicaEpoch, ResumeCursor, UpdateSequence};

use crate::{Replica, ReplicaError, Snapshot};

/// Échec d'une tentative de récupération. La réplique reste inchangée.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecoveryError {
    /// Le contrôleur a été révoqué définitivement.
    Revoked,
    /// La requête appartient à un autre contrôleur.
    ForeignRequest,
    /// La tentative a été remplacée ou déjà acquittée.
    StaleRequest,
    /// La conversation ne correspond pas à celle du contrôleur.
    WrongConversation,
    /// L'epoch de la réplique a changé depuis le début de la recherche.
    SourceChanged,
    /// La couverture déclarée ne correspond pas au nombre de mises à jour.
    CoverageMismatch {
        /// Dernière séquence annoncée par le producteur du snapshot.
        through_sequence: UpdateSequence,
        /// Nombre de mises à jour contenu dans le snapshot.
        update_count: UpdateSequence,
    },
    /// Un snapshot de même epoch reculerait sous une pointe déjà observée.
    SnapshotBehind {
        /// Pointe minimale exigée, y compris les deltas arrivés pendant la recherche.
        minimum_sequence: UpdateSequence,
        /// Pointe annoncée par le snapshot reçu.
        through_sequence: UpdateSequence,
    },
    /// Aucun numéro de tentative supplémentaire ne peut être alloué.
    AttemptExhausted,
    /// Le contenu du snapshot est invalide pour la réplique.
    Replica(ReplicaError),
}

impl fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Revoked => f.write_str("le contrôleur de récupération est révoqué"),
            Self::ForeignRequest => f.write_str("la requête appartient à un autre contrôleur"),
            Self::StaleRequest => f.write_str("la tentative de récupération est périmée"),
            Self::WrongConversation => {
                f.write_str("la conversation ne correspond pas au contrôleur")
            }
            Self::SourceChanged => {
                f.write_str("l'epoch source a changé depuis le début de la recherche")
            }
            Self::CoverageMismatch {
                through_sequence,
                update_count,
            } => write!(
                f,
                "couverture {} annoncée pour {} mises à jour",
                through_sequence.0, update_count.0,
            ),
            Self::SnapshotBehind {
                minimum_sequence,
                through_sequence,
            } => write!(
                f,
                "snapshot en retard : pointe {}, minimum {}",
                through_sequence.0, minimum_sequence.0,
            ),
            Self::AttemptExhausted => {
                f.write_str("les numéros de tentative de récupération sont épuisés")
            }
            Self::Replica(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for RecoveryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Replica(error) => Some(error),
            _ => None,
        }
    }
}

impl From<ReplicaError> for RecoveryError {
    fn from(error: ReplicaError) -> Self {
        Self::Replica(error)
    }
}

/// Requête opaque de récupération, valable pour une seule tentative.
///
/// Une copie conserve le même propriétaire et la même tentative. Elle ne peut
/// plus être appliquée après une réussite, une nouvelle recherche ou révocation.
#[derive(Clone, Debug)]
pub struct RecoveryRequest {
    owner: Arc<()>,
    attempt: u64,
    conversation_id: ConversationId,
    source_epoch: Option<ReplicaEpoch>,
    minimum_sequence: UpdateSequence,
}

impl RecoveryRequest {
    /// Conversation dont le snapshot est recherché.
    pub fn conversation_id(&self) -> &ConversationId {
        &self.conversation_id
    }

    /// Numéro monotone de cette tentative dans son contrôleur.
    pub fn attempt(&self) -> u64 {
        self.attempt
    }

    /// Epoch installée au début de la recherche, ou aucune pour une réplique vide.
    pub fn source_epoch(&self) -> Option<&ReplicaEpoch> {
        self.source_epoch.as_ref()
    }

    /// Pointe du journal appliqué observée au début de la recherche.
    ///
    /// Cette pointe peut dépasser la couverture quand un corps est en attente.
    pub fn minimum_sequence(&self) -> UpdateSequence {
        self.minimum_sequence
    }
}

/// Contrôleur local des réponses de récupération d'une seule conversation.
pub struct RecoveryGate {
    conversation_id: ConversationId,
    owner: Arc<()>,
    next_attempt: Option<u64>,
    state: GateState,
}

enum GateState {
    Ready,
    Searching(u64),
    Revoked,
}

impl RecoveryGate {
    /// Crée un contrôleur sans tentative active.
    pub fn new(conversation_id: ConversationId) -> Self {
        Self {
            conversation_id,
            owner: Arc::new(()),
            next_attempt: Some(1),
            state: GateState::Ready,
        }
    }

    /// Capture la source et la pointe actuelles et remplace la tentative active.
    ///
    /// Une révocation est irréversible. Le compteur ne revient jamais à zéro.
    pub fn begin(&mut self, replica: &Replica) -> Result<RecoveryRequest, RecoveryError> {
        if matches!(self.state, GateState::Revoked) {
            return Err(RecoveryError::Revoked);
        }
        let image = replica.export();
        if image.conversation_id != self.conversation_id {
            return Err(RecoveryError::WrongConversation);
        }
        let attempt = self.next_attempt.ok_or(RecoveryError::AttemptExhausted)?;
        self.next_attempt = attempt.checked_add(1);
        self.state = GateState::Searching(attempt);
        Ok(RecoveryRequest {
            owner: self.owner.clone(),
            attempt,
            conversation_id: self.conversation_id.clone(),
            source_epoch: image.epoch,
            minimum_sequence: UpdateSequence(image.log.len() as u64),
        })
    }

    /// Valide une réponse puis installe son snapshot et acquitte la tentative.
    ///
    /// Un snapshot identique de même epoch est acquitté sans modifier les corps
    /// hydratés ni les deltas bufferisés. Toute erreur garde la réplique intacte.
    pub fn apply(
        &mut self,
        replica: &mut Replica,
        request: &RecoveryRequest,
        snapshot: Snapshot,
        through_sequence: UpdateSequence,
    ) -> Result<ResumeCursor, RecoveryError> {
        if matches!(self.state, GateState::Revoked) {
            return Err(RecoveryError::Revoked);
        }
        if !Arc::ptr_eq(&self.owner, &request.owner) {
            return Err(RecoveryError::ForeignRequest);
        }
        if !matches!(self.state, GateState::Searching(attempt) if attempt == request.attempt) {
            return Err(RecoveryError::StaleRequest);
        }
        let image = replica.export();
        if request.conversation_id != self.conversation_id
            || image.conversation_id != self.conversation_id
            || snapshot.conversation_id != self.conversation_id
        {
            return Err(RecoveryError::WrongConversation);
        }
        if image.epoch != request.source_epoch {
            return Err(RecoveryError::SourceChanged);
        }
        let update_count = UpdateSequence(snapshot.updates.len() as u64);
        if through_sequence != update_count {
            return Err(RecoveryError::CoverageMismatch {
                through_sequence,
                update_count,
            });
        }
        let same_epoch = image.epoch.as_ref() == Some(&snapshot.epoch);
        if same_epoch {
            let minimum_sequence =
                UpdateSequence(request.minimum_sequence.0.max(image.log.len() as u64));
            if through_sequence.0 < minimum_sequence.0 {
                return Err(RecoveryError::SnapshotBehind {
                    minimum_sequence,
                    through_sequence,
                });
            }
        }

        // Cette réplique indépendante vérifie tout le lot avant de décider
        // d'un remplacement ou d'une réussite sans mutation.
        let mut candidate = Replica::new(self.conversation_id.clone());
        let mut cursor = candidate.apply_snapshot(snapshot.clone())?;
        if same_epoch {
            if candidate.log.len() == image.log.len() {
                if candidate.log != image.log {
                    return Err(ReplicaError::Conflict.into());
                }
                self.state = GateState::Ready;
                return Ok(replica.installed_cursor());
            }
            candidate = Replica::restore(image)?;
            cursor = candidate.apply_snapshot(snapshot)?;
        }
        *replica = candidate;
        self.state = GateState::Ready;
        Ok(cursor)
    }

    /// Invalide toutes les tentatives et interdit toute nouvelle recherche.
    pub fn revoke(&mut self) {
        self.state = GateState::Revoked;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compteur_ne_reutilise_pas_le_dernier_numero_et_garde_la_tentative_en_cours() {
        let conversation = ConversationId::new("c").unwrap();
        let mut replica = Replica::new(conversation.clone());
        let mut gate = RecoveryGate::new(conversation.clone());
        gate.next_attempt = Some(u64::MAX);
        let request = gate.begin(&replica).unwrap();
        assert_eq!(request.attempt(), u64::MAX);
        assert!(matches!(
            gate.begin(&replica),
            Err(RecoveryError::AttemptExhausted)
        ));
        let cursor = gate
            .apply(
                &mut replica,
                &request,
                Snapshot {
                    conversation_id: conversation,
                    epoch: ReplicaEpoch {
                        boot_id: grokbot_domain::BootId::new("boot").unwrap(),
                        server_generation: grokbot_domain::ServerGeneration(1),
                        recovery: 0,
                    },
                    updates: Vec::new(),
                },
                UpdateSequence(0),
            )
            .unwrap();
        assert!(cursor.complete);
        assert!(matches!(
            gate.begin(&replica),
            Err(RecoveryError::AttemptExhausted)
        ));
    }
}
