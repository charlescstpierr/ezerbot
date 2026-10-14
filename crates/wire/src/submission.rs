//! Enveloppe versionnée de [`grokbot_domain::SubmissionState`].

use grokbot_domain::{
    AccountId, AttemptId, ConversationId, DeliveryReceipt, MessageId, ReconciliationId, Refusal,
    SubmissionState, TranscriptEntryId,
};
use serde::{Deserialize, Serialize};

use crate::{parse_id, require_schema_version, SCHEMA_VERSION};

/// Message enveloppe d'un état de soumission.
///
/// JSON : `schema_version` au même niveau qu'un tag interne `type`.
/// Exemple : `{"schema_version":1,"type":"queued"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmissionStateDto {
    /// Version de schéma du message. Seule la valeur 1 est convertible.
    pub schema_version: u32,
    /// Variante, aplatie dans le même objet JSON que `schema_version`.
    #[serde(flatten)]
    pub state: SubmissionStateBodyDto,
}

/// Corps d'un [`SubmissionStateDto`], tagué par `type`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SubmissionStateBodyDto {
    /// Soumission construite.
    Prepared,
    /// Soumission en file.
    Queued,
    /// Envoi en cours.
    Dispatching {
        /// Identifiant de tentative.
        attempt: String,
    },
    /// Acceptée, en attente de l'écho.
    AcceptedAwaitingEcho {
        /// Accusé de remise.
        receipt: DeliveryReceiptDto,
    },
    /// Écho reçu.
    Echoed {
        /// Identifiant de l'entrée de transcription.
        entry: String,
    },
    /// Refusée.
    Refused {
        /// Motif de refus.
        reason: RefusalDto,
    },
    /// Issue inconnue.
    OutcomeUnknown {
        /// Identifiant de réconciliation.
        reconciliation: String,
    },
    /// Tag `type` qui ne correspond à aucune action connue.
    ///
    /// [`submission_from_dto`] refuse cette variante : une action inconnue
    /// n'est pas devinée.
    #[serde(other)]
    Unrecognized,
}

/// Accusé de remise, identifiants en chaînes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryReceiptDto {
    /// Compte.
    pub account_id: String,
    /// Conversation.
    pub conversation_id: String,
    /// Message.
    pub message_id: String,
}

/// Motif de refus, tag interne `type`.
///
/// `Other` conserve la chaîne libre dans `message`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RefusalDto {
    /// Conflit.
    Conflict,
    /// Conversation inconnue.
    UnknownConversation,
    /// Génération de compte périmée.
    StaleAccountGeneration,
    /// Politique.
    Policy,
    /// Motif libre.
    Other {
        /// Texte du motif, repris tel quel.
        message: String,
    },
}

/// Projette un état de domaine vers son DTO, en version de schéma 1.
pub fn submission_to_dto(state: &SubmissionState) -> SubmissionStateDto {
    let body = match state {
        SubmissionState::Prepared => SubmissionStateBodyDto::Prepared,
        SubmissionState::Queued => SubmissionStateBodyDto::Queued,
        SubmissionState::Dispatching { attempt } => SubmissionStateBodyDto::Dispatching {
            attempt: attempt.as_str().to_owned(),
        },
        SubmissionState::AcceptedAwaitingEcho { receipt } => {
            SubmissionStateBodyDto::AcceptedAwaitingEcho {
                receipt: DeliveryReceiptDto {
                    account_id: receipt.account_id.as_str().to_owned(),
                    conversation_id: receipt.conversation_id.as_str().to_owned(),
                    message_id: receipt.message_id.as_str().to_owned(),
                },
            }
        }
        SubmissionState::Echoed { entry } => SubmissionStateBodyDto::Echoed {
            entry: entry.as_str().to_owned(),
        },
        SubmissionState::Refused { reason } => SubmissionStateBodyDto::Refused {
            reason: refusal_to_dto(reason),
        },
        SubmissionState::OutcomeUnknown { reconciliation } => {
            SubmissionStateBodyDto::OutcomeUnknown {
                reconciliation: reconciliation.as_str().to_owned(),
            }
        }
    };
    SubmissionStateDto {
        schema_version: SCHEMA_VERSION,
        state: body,
    }
}

/// Reconstruit un état de domaine.
///
/// Une version autre que 1 produit [`crate::WireError::UnsupportedVersion`].
/// Un tag de soumission inconnu produit [`crate::WireError::UnknownSubmissionState`].
pub fn submission_from_dto(dto: SubmissionStateDto) -> Result<SubmissionState, crate::WireError> {
    require_schema_version(dto.schema_version)?;
    match dto.state {
        SubmissionStateBodyDto::Prepared => Ok(SubmissionState::Prepared),
        SubmissionStateBodyDto::Queued => Ok(SubmissionState::Queued),
        SubmissionStateBodyDto::Dispatching { attempt } => Ok(SubmissionState::Dispatching {
            attempt: parse_id(attempt, AttemptId::new)?,
        }),
        SubmissionStateBodyDto::AcceptedAwaitingEcho { receipt } => {
            Ok(SubmissionState::AcceptedAwaitingEcho {
                receipt: receipt_from_dto(receipt)?,
            })
        }
        SubmissionStateBodyDto::Echoed { entry } => Ok(SubmissionState::Echoed {
            entry: parse_id(entry, TranscriptEntryId::new)?,
        }),
        SubmissionStateBodyDto::Refused { reason } => Ok(SubmissionState::Refused {
            reason: refusal_from_dto(reason),
        }),
        SubmissionStateBodyDto::OutcomeUnknown { reconciliation } => {
            Ok(SubmissionState::OutcomeUnknown {
                reconciliation: parse_id(reconciliation, ReconciliationId::new)?,
            })
        }
        SubmissionStateBodyDto::Unrecognized => Err(crate::WireError::UnknownSubmissionState),
    }
}

fn receipt_from_dto(dto: DeliveryReceiptDto) -> Result<DeliveryReceipt, crate::WireError> {
    Ok(DeliveryReceipt {
        account_id: parse_id(dto.account_id, AccountId::new)?,
        conversation_id: parse_id(dto.conversation_id, ConversationId::new)?,
        message_id: parse_id(dto.message_id, MessageId::new)?,
    })
}

fn refusal_to_dto(reason: &Refusal) -> RefusalDto {
    match reason {
        Refusal::Conflict => RefusalDto::Conflict,
        Refusal::UnknownConversation => RefusalDto::UnknownConversation,
        Refusal::StaleAccountGeneration => RefusalDto::StaleAccountGeneration,
        Refusal::Policy => RefusalDto::Policy,
        Refusal::Other(message) => RefusalDto::Other {
            message: message.clone(),
        },
    }
}

fn refusal_from_dto(reason: RefusalDto) -> Refusal {
    match reason {
        RefusalDto::Conflict => Refusal::Conflict,
        RefusalDto::UnknownConversation => Refusal::UnknownConversation,
        RefusalDto::StaleAccountGeneration => Refusal::StaleAccountGeneration,
        RefusalDto::Policy => Refusal::Policy,
        RefusalDto::Other { message } => Refusal::Other(message),
    }
}
