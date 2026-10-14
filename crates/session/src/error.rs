use std::fmt;

use grokbot_store::StoreError;
use grokbot_submission_journal::JournalError;

/// Commande refusée ou échec de sa sauvegarde durable.
#[derive(Debug)]
pub enum SessionError {
    Store(StoreError),
    Journal(JournalError),
    Revoked,
    StaleRequest,
    InvalidReceipt,
    CounterExhausted,
    ReloadRequired,
    ReloadFailed {
        save: Box<StoreError>,
        reload: Box<StoreError>,
    },
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => write!(formatter, "stockage : {error}"),
            Self::Journal(error) => write!(formatter, "journal : {error}"),
            Self::Revoked => formatter.write_str("session révoquée"),
            Self::StaleRequest => formatter.write_str("continuation périmée ou étrangère"),
            Self::InvalidReceipt => formatter.write_str("reçu incompatible avec la requête"),
            Self::CounterExhausted => formatter.write_str("compteur de tentatives épuisé"),
            Self::ReloadRequired => formatter.write_str("journal durable à recharger"),
            Self::ReloadFailed { save, reload } => {
                write!(
                    formatter,
                    "sauvegarde refusée : {save}; rechargement refusé : {reload}"
                )
            }
        }
    }
}

impl std::error::Error for SessionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Store(error) => Some(error),
            Self::Journal(error) => Some(error),
            Self::ReloadFailed { save, .. } => Some(save.as_ref()),
            _ => None,
        }
    }
}

impl From<StoreError> for SessionError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

impl From<JournalError> for SessionError {
    fn from(error: JournalError) -> Self {
        Self::Journal(error)
    }
}
