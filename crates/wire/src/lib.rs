//! DTO versionnés et conversions vers le domaine.
//!
//! Chaque message enveloppe porte `schema_version`. Les enums utilisent un tag
//! interne `type`. Ce crate valide la forme : il ne tient ni journal ni réplique.

#![forbid(unsafe_code)]

mod cursor;
mod digest;
mod error;
mod submission;
mod transcript;

pub use cursor::{cursor_from_dto, cursor_to_dto, ReplicaEpochDto, ResumeCursorDto};
pub use error::WireError;
pub use submission::{
    submission_from_dto, submission_to_dto, DeliveryReceiptDto, RefusalDto, SubmissionStateBodyDto,
    SubmissionStateDto,
};
pub use transcript::{
    update_from_dto, update_to_dto, BodyUnavailableDto, EntryBodyDto, TranscriptBodyDto,
    TranscriptUpdateDto, UpdateKindDto,
};

/// Seule version de schéma acceptée par les conversions.
pub const SCHEMA_VERSION: u32 = 1;

pub(crate) fn require_schema_version(version: u32) -> Result<(), WireError> {
    if version == SCHEMA_VERSION {
        Ok(())
    } else {
        Err(WireError::UnsupportedVersion { version })
    }
}

pub(crate) fn parse_id<T>(
    raw: String,
    ctor: impl FnOnce(String) -> Result<T, grokbot_domain::EmptyId>,
) -> Result<T, WireError> {
    ctor(raw).map_err(|_| WireError::EmptyId)
}

#[cfg(test)]
mod tests;
