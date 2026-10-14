//! Enveloppe versionnée de [`grokbot_domain::ResumeCursor`].

use grokbot_domain::{BootId, ReplicaEpoch, ResumeCursor, ServerGeneration, UpdateSequence};
use serde::{Deserialize, Serialize};

use crate::{parse_id, require_schema_version, SCHEMA_VERSION};

/// Message enveloppe d'un curseur de reprise.
///
/// Le curseur n'est pas un enum : le JSON porte `schema_version` et les champs
/// du curseur, sans tag `type`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResumeCursorDto {
    /// Version de schéma du message. Seule la valeur 1 est convertible.
    pub schema_version: u32,
    /// Époque à laquelle le curseur se rapporte.
    pub epoch: ReplicaEpochDto,
    /// Dernière séquence couverte.
    pub covered_through: u64,
    /// Indique si la couverture est complète.
    pub complete: bool,
}

/// Époque de réplica, sans version de schéma propre.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicaEpochDto {
    /// Identifiant de démarrage.
    pub boot_id: String,
    /// Génération serveur.
    pub server_generation: u64,
    /// Compteur de recovery.
    pub recovery: u32,
}

/// Projette un curseur de domaine vers son DTO, en version de schéma 1.
pub fn cursor_to_dto(cursor: &ResumeCursor) -> ResumeCursorDto {
    ResumeCursorDto {
        schema_version: SCHEMA_VERSION,
        epoch: ReplicaEpochDto {
            boot_id: cursor.epoch.boot_id.as_str().to_owned(),
            server_generation: cursor.epoch.server_generation.0,
            recovery: cursor.epoch.recovery,
        },
        covered_through: cursor.covered_through.0,
        complete: cursor.complete,
    }
}

/// Reconstruit un curseur de domaine.
///
/// Une version autre que 1 produit [`crate::WireError::UnsupportedVersion`].
pub fn cursor_from_dto(dto: ResumeCursorDto) -> Result<ResumeCursor, crate::WireError> {
    require_schema_version(dto.schema_version)?;
    Ok(ResumeCursor {
        epoch: ReplicaEpoch {
            boot_id: parse_id(dto.epoch.boot_id, BootId::new)?,
            server_generation: ServerGeneration(dto.epoch.server_generation),
            recovery: dto.epoch.recovery,
        },
        covered_through: UpdateSequence(dto.covered_through),
        complete: dto.complete,
    })
}
