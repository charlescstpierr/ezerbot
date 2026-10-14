//! Sauvegarde des projections après validation des tentatives de récupération.

use std::fmt;
use std::path::Path;

use grokbot_domain::{
    AuthenticatedActor, ConversationId, ReplicaEpoch, ResumeCursor, TranscriptBody,
    TranscriptEntryId, UpdateSequence,
};
use grokbot_replica::{
    Delta, RecoveryError, RecoveryGate, RecoveryRequest, Replica, ReplicaError, Snapshot,
};
use grokbot_store::{SessionStore, StoreError, StoredReplica};

/// Échec d'une projection locale ou de sa sauvegarde.
#[derive(Debug)]
pub enum ReplicationSessionError {
    /// La session a été révoquée, ou une restauration après erreur a échoué.
    Inactive,
    /// La garde a refusé le snapshot.
    Recovery(RecoveryError),
    /// Les événements ou le corps sont incompatibles avec la projection.
    Replica(ReplicaError),
    /// La sauvegarde a échoué; la dernière image durable est rechargée.
    Store(StoreError),
    /// La sauvegarde et le rechargement ont échoué. La session devient inactive.
    ReloadFailed {
        /// Première erreur rencontrée.
        original: String,
        /// Échec de rechargement de l'image durable.
        reload: StoreError,
    },
}

impl fmt::Display for ReplicationSessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inactive => f.write_str("session de réplication inactive"),
            Self::Recovery(error) => write!(f, "récupération : {error}"),
            Self::Replica(error) => write!(f, "réplique : {error}"),
            Self::Store(error) => write!(f, "stockage : {error}"),
            Self::ReloadFailed { original, reload } => {
                write!(f, "{original}; rechargement impossible : {reload}")
            }
        }
    }
}

impl std::error::Error for ReplicationSessionError {}

/// Projection d'une conversation d'un compte, avec snapshots gardés et commits SQLite.
pub struct SnapshotSession {
    actor: AuthenticatedActor,
    conversation: ConversationId,
    store: SessionStore,
    replica: StoredReplica,
    recovery: RecoveryGate,
    active: bool,
}

impl SnapshotSession {
    /// Ouvre la projection du compte, sans réutiliser une ancienne tentative.
    pub fn open(
        path: &Path,
        actor: AuthenticatedActor,
        conversation: ConversationId,
    ) -> Result<Self, ReplicationSessionError> {
        let store =
            SessionStore::open(path, &actor.account_id).map_err(ReplicationSessionError::Store)?;
        let replica = store
            .load_replica(&conversation)
            .map_err(ReplicationSessionError::Store)?;
        Ok(Self {
            actor,
            recovery: RecoveryGate::new(conversation.clone()),
            conversation,
            store,
            replica,
            active: true,
        })
    }

    /// Identité capturée lors de l'ouverture.
    pub fn actor(&self) -> &AuthenticatedActor {
        &self.actor
    }

    /// Projection persistée consultable. Une image impossible à recharger reste inaccessible.
    pub fn replica(&self) -> Result<&Replica, ReplicationSessionError> {
        self.ensure_active()?;
        Ok(&self.replica)
    }

    /// Curseur de l'image consultable.
    pub fn cursor(&self) -> Result<Option<ResumeCursor>, ReplicationSessionError> {
        Ok(self.replica()?.cursor())
    }

    /// Commence une nouvelle recherche et invalide la précédente.
    pub fn begin_recovery(&mut self) -> Result<RecoveryRequest, ReplicationSessionError> {
        self.ensure_active()?;
        self.recovery
            .begin(&self.replica)
            .map_err(ReplicationSessionError::Recovery)
    }

    /// Valide le snapshot actif, puis le sauvegarde avant d'annoncer son curseur.
    pub fn apply_snapshot(
        &mut self,
        request: &RecoveryRequest,
        snapshot: Snapshot,
        through: UpdateSequence,
    ) -> Result<ResumeCursor, ReplicationSessionError> {
        self.ensure_active()?;
        let cursor = self
            .recovery
            .apply(&mut self.replica, request, snapshot, through)
            .map_err(ReplicationSessionError::Recovery)?;
        self.persist()?;
        Ok(cursor)
    }

    /// Applique un delta et sauvegarde avant de rendre son curseur.
    pub fn apply_delta(&mut self, delta: Delta) -> Result<ResumeCursor, ReplicationSessionError> {
        self.ensure_active()?;
        let cursor = match self.replica.apply_delta(delta) {
            Ok(cursor) => cursor,
            Err(error) => return Err(self.restore(ReplicationSessionError::Replica(error))),
        };
        self.persist()?;
        Ok(cursor)
    }

    /// Retrouve une version précise d'un corps et sauvegarde sa résolution.
    pub fn fill_body(
        &mut self,
        epoch: &ReplicaEpoch,
        sequence: UpdateSequence,
        entry: &TranscriptEntryId,
        body: TranscriptBody,
    ) -> Result<ResumeCursor, ReplicationSessionError> {
        self.ensure_active()?;
        let cursor = self
            .replica
            .fill_body(epoch, sequence, entry, body)
            .map_err(ReplicationSessionError::Replica)?;
        self.persist()?;
        Ok(cursor)
    }

    /// Révoque les intentions et les réponses de cette instance.
    pub fn revoke(&mut self) {
        self.active = false;
        self.recovery.revoke();
    }

    fn ensure_active(&self) -> Result<(), ReplicationSessionError> {
        if self.active {
            Ok(())
        } else {
            Err(ReplicationSessionError::Inactive)
        }
    }

    fn persist(&mut self) -> Result<(), ReplicationSessionError> {
        if let Err(error) = self.store.save_replica(&mut self.replica) {
            return Err(self.restore(ReplicationSessionError::Store(error)));
        }
        Ok(())
    }

    fn restore(&mut self, original: ReplicationSessionError) -> ReplicationSessionError {
        match self.store.load_replica(&self.conversation) {
            Ok(replica) => {
                self.replica = replica;
                // A reloaded image may belong to a concurrent writer. Previously
                // issued requests cannot authorize a new replacement of that image.
                self.recovery = RecoveryGate::new(self.conversation.clone());
                original
            }
            Err(reload) => {
                self.revoke();
                ReplicationSessionError::ReloadFailed {
                    original: original.to_string(),
                    reload,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
