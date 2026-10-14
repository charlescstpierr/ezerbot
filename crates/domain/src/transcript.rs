//! Corps de transcription et mises à jour ordonnées.

use crate::id::{require_non_empty, EmptyId, TranscriptEntryId, UpdateSequence};

/// Empreinte de 32 octets associée à un corps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ContentDigest(
    /// Octets de l'empreinte.
    pub [u8; 32],
);

/// Référence non vide vers un contenu externe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlobRef(String);

impl BlobRef {
    /// Construit la référence. Une chaîne vide est refusée.
    pub fn new(value: impl Into<String>) -> Result<Self, EmptyId> {
        Ok(Self(require_non_empty(value)?))
    }

    /// Retourne la référence textuelle.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Raison pour laquelle un corps n'est plus disponible.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BodyUnavailable {
    /// Contenu expiré.
    Expired,
    /// Contenu purgé.
    Purged,
    /// Accès refusé.
    Acl,
}

/// Corps d'entrée connu du domaine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryBody {
    /// Texte intégral.
    Text(String),
    /// Corps dont le type n'est pas interprété ici.
    Unknown {
        /// Nom du type non interprété.
        kind: String,
    },
}

/// Localisation du corps d'une entrée de transcription.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TranscriptBody {
    /// Corps porté directement par la mise à jour.
    Inline {
        /// Contenu en ligne.
        content: EntryBody,
    },
    /// Corps adressé par une référence et une empreinte.
    Blob {
        /// Référence du blob.
        reference: BlobRef,
        /// Empreinte du contenu adressé.
        digest: ContentDigest,
    },
    /// Corps pas encore disponible ; l'empreinte attendue est connue.
    Pending {
        /// Empreinte attendue lorsque le corps arrivera.
        expected_digest: ContentDigest,
    },
    /// Corps définitivement inaccessible.
    Unavailable {
        /// Raison de l'indisponibilité.
        reason: BodyUnavailable,
    },
}

impl TranscriptBody {
    /// Vrai seulement pour [`TranscriptBody::Pending`].
    ///
    /// Un corps en attente bloque la couverture. Les autres localisations ne la bloquent pas.
    pub fn blocks_coverage(&self) -> bool {
        match self {
            Self::Pending { .. } => true,
            Self::Inline { .. } | Self::Blob { .. } | Self::Unavailable { .. } => false,
        }
    }
}

/// Nature d'une mise à jour de transcription.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateKind {
    /// Insertion ou remplacement du corps.
    Upsert {
        /// Corps inséré ou remplacé.
        body: TranscriptBody,
    },
    /// Suppression de l'entrée.
    Delete,
}

/// Mise à jour ordonnée d'une entrée de transcription.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscriptUpdate {
    /// Position de la mise à jour dans l'ordre du flux.
    pub sequence: UpdateSequence,
    /// Entrée visée.
    pub entry_id: TranscriptEntryId,
    /// Nature de la mise à jour.
    pub kind: UpdateKind,
}
