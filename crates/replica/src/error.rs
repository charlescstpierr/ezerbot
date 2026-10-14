//! Erreurs de réplication. Aucune ne laisse une application à moitié faite,
//! sauf [`ReplicaError::ReorderOverflow`] qui vide le buffer sans toucher
//! au journal déjà appliqué.

use std::fmt;

/// Échec d'une opération sur la réplique.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReplicaError {
    /// Les séquences du snapshot ne sont pas exactement `1..=n`.
    MalformedSnapshot,
    /// Un snapshot de la même epoch n'avance pas la pointe du journal.
    StaleSnapshot,
    /// Le snapshot ou le delta vise une autre conversation.
    WrongConversation,
    /// Le delta ou l'hydratation n'a pas l'epoch installée. Rien n'est appliqué.
    ForeignEpoch,
    /// La première séquence est au-delà de la fenêtre de réordonnancement.
    ///
    /// Rien n'est appliqué. Le buffer déjà retenu est conservé.
    Gap {
        /// Prochaine séquence attendue, soit la pointe du journal plus un.
        expected: u64,
        /// Première séquence du delta refusé.
        got: u64,
    },
    /// Séquence entièrement antérieure, trop vieille pour être réordonnée,
    /// et sans chevauchement avec la prochaine séquence attendue.
    CursorTooOld,
    /// Une séquence déjà connue (journal ou buffer) porte un autre contenu,
    /// ou le delta n'est pas une plage contiguë.
    Conflict,
    /// Accepter le delta ferait dépasser 512 mises à jour en attente.
    ///
    /// Le buffer est vidé. Le journal appliqué reste en place.
    ReorderOverflow,
    /// `fill_body` vise une séquence absente ou une autre entrée.
    NotFound,
    /// Le corps de remplacement bloque encore la couverture. L'état ne change pas.
    StillPending,
    /// La version indiquée ne portait pas un corps en attente.
    NotPending,
    /// L'empreinte annoncée du blob diffère de celle attendue.
    DigestMismatch,
}

impl fmt::Display for ReplicaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedSnapshot => f.write_str(
                "snapshot malformé : les séquences doivent être 1..=n sans trou ni doublon",
            ),
            Self::StaleSnapshot => f.write_str("snapshot périmé : la pointe n'avance pas"),
            Self::WrongConversation => f.write_str(
                "la conversation du snapshot ou du delta ne correspond pas à la réplique",
            ),
            Self::ForeignEpoch => {
                f.write_str("l'epoch ne correspond pas à l'epoch installée")
            }
            Self::Gap { expected, got } => write!(
                f,
                "trou de séquence : {got} reçu, {expected} attendu, au-delà de la fenêtre de réordonnancement"
            ),
            Self::CursorTooOld => {
                f.write_str("séquence trop ancienne pour le réordonnancement")
            }
            Self::Conflict => {
                f.write_str("conflit : séquence déjà vue avec un autre contenu, ou delta non contigu")
            }
            Self::ReorderOverflow => {
                f.write_str("le buffer de réordonnancement dépasserait 512 mises à jour")
            }
            Self::NotFound => f.write_str("entrée inconnue"),
            Self::StillPending => {
                f.write_str("le corps de remplacement bloque encore la couverture")
            }
            Self::NotPending => f.write_str("cette version n'attend pas de corps"),
            Self::DigestMismatch => f.write_str("l'empreinte du blob ne correspond pas à celle attendue"),
        }
    }
}

impl std::error::Error for ReplicaError {}
