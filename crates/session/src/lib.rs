//! Orchestration locale du journal durable et des adaptations de transport.

#![forbid(unsafe_code)]

mod error;
mod replication;
mod submission;

pub use error::SessionError;
pub use replication::{ReplicationSessionError, SnapshotSession};
pub use submission::{
    LookupOutcome, ReconciliationRequest, SendOutcome, SendRequest, SubmissionSession,
    SubmissionTransport,
};

#[cfg(test)]
mod tests;
