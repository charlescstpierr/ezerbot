//! Acteur authentifié et génération de compte.

use crate::id::{AccountId, TeamId};

/// Génération d'un compte.
///
/// Une continuation n'est acceptée que si elle porte exactement cette valeur.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AccountGeneration(
    /// Valeur de la génération.
    pub u64,
);

/// Acteur authentifié, ancré sur un compte et une génération.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedActor {
    /// Compte auquel l'acteur appartient.
    pub account_id: AccountId,
    /// Équipe active, si l'acteur en a une.
    pub team_id: Option<TeamId>,
    /// Génération du compte au moment de l'authentification.
    pub generation: AccountGeneration,
}

impl AuthenticatedActor {
    /// Indique si une continuation peut être acceptée pour cet acteur.
    ///
    /// Vrai seulement lorsque `continuation` est exactement [`Self::generation`].
    /// Une génération différente signifie que le compte a changé.
    pub fn accepts_continuation(&self, continuation: AccountGeneration) -> bool {
        continuation == self.generation
    }
}
