//! Enveloppe versionnée de [`grokbot_domain::TranscriptUpdate`].

use grokbot_domain::{
    BlobRef, BodyUnavailable, EntryBody, TranscriptBody, TranscriptEntryId, TranscriptUpdate,
    UpdateKind, UpdateSequence,
};
use serde::de::{self, DeserializeOwned, Deserializer};
use serde::{Deserialize, Serialize};

use crate::digest::{decode_digest, encode_digest};
use crate::{parse_id, require_schema_version, SCHEMA_VERSION};

/// Message enveloppe d'une mise à jour de transcription.
///
/// `schema_version`, `sequence` et `entry_id` accompagnent le tag interne
/// `type` de la mise à jour (`upsert` ou `delete`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptUpdateDto {
    /// Version de schéma du message. Seule la valeur 1 est convertible.
    pub schema_version: u32,
    /// Numéro de séquence.
    pub sequence: u64,
    /// Identifiant de l'entrée concernée.
    pub entry_id: String,
    /// Nature de la mise à jour, aplatie dans le même objet.
    #[serde(flatten)]
    pub kind: UpdateKindDto,
}

/// Nature d'une mise à jour, tag interne `type`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum UpdateKindDto {
    /// Insertion ou remplacement du corps.
    Upsert {
        /// Corps de l'entrée.
        body: TranscriptBodyDto,
    },
    /// Suppression.
    Delete,
}

/// Corps de transcription, tag interne `type`.
///
/// Un discriminant inconnu est relu comme un contenu d'affichage
/// [`EntryBodyDto::Unknown`] : il n'est pas rejeté.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TranscriptBodyDto {
    /// Corps porté dans le message.
    Inline {
        /// Contenu affichable.
        content: EntryBodyDto,
    },
    /// Corps stocké à part.
    Blob {
        /// Référence opaque.
        reference: String,
        /// Empreinte hexadécimale minuscule, 64 caractères.
        digest: String,
    },
    /// Corps pas encore disponible.
    Pending {
        /// Empreinte attendue, hexadécimale minuscule.
        expected_digest: String,
    },
    /// Corps définitivement indisponible.
    Unavailable {
        /// Motif d'indisponibilité.
        reason: BodyUnavailableDto,
    },
}

/// Contenu affichable, tag interne `type`.
///
/// `text` est le seul discriminant connu. Tout autre discriminant, y compris
/// un type de carte inconnu, devient [`EntryBodyDto::Unknown`] et conserve
/// cette chaîne dans `kind`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EntryBodyDto {
    /// Texte.
    Text {
        /// Texte de l'entrée.
        text: String,
    },
    /// Contenu dont le discriminant n'est pas connu de cette version.
    Unknown {
        /// Discriminant d'origine, ou `kind` explicite lorsque `type` vaut `unknown`.
        kind: String,
    },
}

/// Motif d'indisponibilité d'un corps, tag interne `type`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BodyUnavailableDto {
    /// Corps expiré.
    Expired,
    /// Corps purgé.
    Purged,
    /// Accès refusé.
    Acl,
}

/// Projette une mise à jour de domaine vers son DTO, en version de schéma 1.
pub fn update_to_dto(update: &TranscriptUpdate) -> TranscriptUpdateDto {
    let kind = match &update.kind {
        UpdateKind::Upsert { body } => UpdateKindDto::Upsert {
            body: body_to_dto(body),
        },
        UpdateKind::Delete => UpdateKindDto::Delete,
    };
    TranscriptUpdateDto {
        schema_version: SCHEMA_VERSION,
        sequence: update.sequence.0,
        entry_id: update.entry_id.as_str().to_owned(),
        kind,
    }
}

/// Reconstruit une mise à jour de domaine.
///
/// Un discriminant de corps inconnu devient [`EntryBody::Unknown`] et ne fait
/// pas échouer la conversion. Une empreinte mal formée produit
/// [`crate::WireError::InvalidDigest`].
pub fn update_from_dto(dto: TranscriptUpdateDto) -> Result<TranscriptUpdate, crate::WireError> {
    require_schema_version(dto.schema_version)?;
    let kind = match dto.kind {
        UpdateKindDto::Upsert { body } => UpdateKind::Upsert {
            body: body_from_dto(body)?,
        },
        UpdateKindDto::Delete => UpdateKind::Delete,
    };
    Ok(TranscriptUpdate {
        sequence: UpdateSequence(dto.sequence),
        entry_id: parse_id(dto.entry_id, TranscriptEntryId::new)?,
        kind,
    })
}

fn body_to_dto(body: &TranscriptBody) -> TranscriptBodyDto {
    match body {
        TranscriptBody::Inline { content } => TranscriptBodyDto::Inline {
            content: entry_to_dto(content),
        },
        TranscriptBody::Blob { reference, digest } => TranscriptBodyDto::Blob {
            reference: reference.as_str().to_owned(),
            digest: encode_digest(digest),
        },
        TranscriptBody::Pending { expected_digest } => TranscriptBodyDto::Pending {
            expected_digest: encode_digest(expected_digest),
        },
        TranscriptBody::Unavailable { reason } => TranscriptBodyDto::Unavailable {
            reason: match reason {
                BodyUnavailable::Expired => BodyUnavailableDto::Expired,
                BodyUnavailable::Purged => BodyUnavailableDto::Purged,
                BodyUnavailable::Acl => BodyUnavailableDto::Acl,
            },
        },
    }
}

fn body_from_dto(body: TranscriptBodyDto) -> Result<TranscriptBody, crate::WireError> {
    match body {
        TranscriptBodyDto::Inline { content } => Ok(TranscriptBody::Inline {
            content: entry_from_dto(content),
        }),
        TranscriptBodyDto::Blob { reference, digest } => Ok(TranscriptBody::Blob {
            reference: parse_id(reference, BlobRef::new)?,
            digest: decode_digest(&digest)?,
        }),
        TranscriptBodyDto::Pending { expected_digest } => Ok(TranscriptBody::Pending {
            expected_digest: decode_digest(&expected_digest)?,
        }),
        TranscriptBodyDto::Unavailable { reason } => Ok(TranscriptBody::Unavailable {
            reason: match reason {
                BodyUnavailableDto::Expired => BodyUnavailable::Expired,
                BodyUnavailableDto::Purged => BodyUnavailable::Purged,
                BodyUnavailableDto::Acl => BodyUnavailable::Acl,
            },
        }),
    }
}

fn entry_to_dto(body: &EntryBody) -> EntryBodyDto {
    match body {
        EntryBody::Text(text) => EntryBodyDto::Text { text: text.clone() },
        EntryBody::Unknown { kind } => EntryBodyDto::Unknown { kind: kind.clone() },
    }
}

fn entry_from_dto(body: EntryBodyDto) -> EntryBody {
    match body {
        EntryBodyDto::Text { text } => EntryBody::Text(text),
        EntryBodyDto::Unknown { kind } => EntryBody::Unknown { kind },
    }
}

#[derive(Deserialize)]
struct RawEntryBody {
    #[serde(rename = "type")]
    type_name: String,
    #[serde(default)]
    text: Option<serde_json::Value>,
    #[serde(default)]
    kind: Option<serde_json::Value>,
}

impl<'de> Deserialize<'de> for EntryBodyDto {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawEntryBody::deserialize(deserializer)?;
        match raw.type_name.as_str() {
            "text" => {
                let text = deserialize_field(raw.text, "text")?;
                Ok(Self::Text { text })
            }
            "unknown" => {
                let kind = deserialize_field(raw.kind, "kind")?;
                Ok(Self::Unknown { kind })
            }
            kind => Ok(Self::Unknown {
                kind: kind.to_owned(),
            }),
        }
    }
}

#[derive(Deserialize)]
struct RawTranscriptBody {
    #[serde(rename = "type")]
    type_name: String,
    #[serde(default)]
    content: Option<serde_json::Value>,
    #[serde(default)]
    reference: Option<serde_json::Value>,
    #[serde(default)]
    digest: Option<serde_json::Value>,
    #[serde(default)]
    expected_digest: Option<serde_json::Value>,
    #[serde(default)]
    reason: Option<serde_json::Value>,
}

impl<'de> Deserialize<'de> for TranscriptBodyDto {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = RawTranscriptBody::deserialize(deserializer)?;
        match raw.type_name.as_str() {
            "inline" => {
                let content = deserialize_field(raw.content, "content")?;
                Ok(Self::Inline { content })
            }
            "blob" => {
                let reference = deserialize_field(raw.reference, "reference")?;
                let digest = deserialize_field(raw.digest, "digest")?;
                Ok(Self::Blob { reference, digest })
            }
            "pending" => {
                let expected_digest = deserialize_field(raw.expected_digest, "expected_digest")?;
                Ok(Self::Pending { expected_digest })
            }
            "unavailable" => {
                let reason = deserialize_field(raw.reason, "reason")?;
                Ok(Self::Unavailable { reason })
            }
            kind => Ok(Self::Inline {
                content: EntryBodyDto::Unknown {
                    kind: kind.to_owned(),
                },
            }),
        }
    }
}

// Decode only fields belonging to the selected variant. Unknown variants may
// reuse known field names with a different shape.
fn deserialize_field<T: DeserializeOwned, E: de::Error>(
    value: Option<serde_json::Value>,
    name: &'static str,
) -> Result<T, E> {
    let value = value.ok_or_else(|| E::missing_field(name))?;
    serde_json::from_value(value).map_err(E::custom)
}
