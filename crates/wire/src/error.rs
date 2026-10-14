//! Erreurs de conversion entre DTO et domaine.

use std::fmt;

/// Échec de validation d'un DTO. Aucune de ces variantes ne devine une action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WireError {
    /// `schema_version` différente de [`crate::SCHEMA_VERSION`].
    UnsupportedVersion {
        /// Version reçue.
        version: u32,
    },
    /// Le tag `type` d'une soumission ne correspond à aucune variante connue.
    UnknownSubmissionState,
    /// Empreinte qui n'est pas exactement 64 chiffres hexadécimaux minuscules.
    InvalidDigest,
    /// Identifiant ou référence de blob vide.
    EmptyId,
}

impl fmt::Display for WireError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedVersion { version } => {
                write!(formatter, "version de schéma {version} non prise en charge")
            }
            Self::UnknownSubmissionState => formatter.write_str("état de soumission inconnu"),
            Self::InvalidDigest => formatter
                .write_str("empreinte hexadécimale invalide : 64 caractères minuscules attendus"),
            Self::EmptyId => formatter.write_str("identifiant vide"),
        }
    }
}

impl std::error::Error for WireError {}
