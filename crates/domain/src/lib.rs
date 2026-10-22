//! Identités, états et règles pures, sans entrée/sortie.
//!
//! Les identifiants sont des chaînes opaques non vides. Ce crate ne décide
//! aucune transition en parlant au réseau, au disque ou à une horloge.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod actor;
mod id;
mod run;
mod submission;
mod transcript;

pub use actor::{AccountGeneration, AuthenticatedActor};
pub use id::{
    AccountId, ActionGrantId, ApprovalId, AttemptId, BootId, BotId, CheckpointId, ComputerId,
    ConversationId, EmptyId, EntrySequence, HumanInputId, MessageId, ReconciliationId,
    ReplicaSequence, RunId, ScreenLeaseId, ServerGeneration, SessionId, TeamId, ToolCallId,
    TranscriptEntryId, UpdateSequence,
};
pub use run::{ApprovalDecision, EffectDisposition, InterruptionReason, RunFailure, RunState};
pub use submission::{
    DeliveryReceipt, Refusal, ReplicaEpoch, ResumeCursor, SubmissionState, TransitionError,
};
pub use transcript::{
    BlobRef, BodyUnavailable, ContentDigest, EntryBody, TranscriptBody, TranscriptUpdate,
    UpdateKind,
};

/// Renvoie 42 pour sonder le fonctionnement de l'usine.
pub fn usine_sonde() -> u32 {
    42
}

#[cfg(test)]
mod tests;

#[cfg(test)]
#[test]
fn usine_sonde_renvoie_42() {
    assert_eq!(usine_sonde(), 42);
}
