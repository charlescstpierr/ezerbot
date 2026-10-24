//! Stockage SQLite du journal validé et de la réplique.
//!
//! Chaque enregistrement reprend les DTO versionnés de `grokbot-wire`.
//! La zone non validée du journal n'est pas écrite : elle ne survit pas à une coupure.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use grokbot_domain::{AccountGeneration, AccountId, ConversationId, MessageId};
use grokbot_replica::{BodyHydration, Replica, ReplicaImage};
use grokbot_submission_journal::{CommittedSubmission, Journal};
use grokbot_wire::{
    cursor_from_dto, cursor_to_dto, submission_from_dto, submission_to_dto, update_from_dto,
    update_to_dto, ReplicaEpochDto, ResumeCursorDto, SubmissionStateDto, TranscriptUpdateDto,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};

const SCHEMA_VERSION: i64 = 2;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS submissions (
    account_id TEXT NOT NULL,
    generation INTEGER NOT NULL,
    conversation_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    body TEXT NOT NULL,
    state_json TEXT NOT NULL,
    PRIMARY KEY (account_id, conversation_id, message_id)
);
CREATE TABLE IF NOT EXISTS replicas (
    account_id TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    epoch_json TEXT,
    log_json TEXT NOT NULL,
    buffer_json TEXT NOT NULL,
    hydrations_json TEXT NOT NULL DEFAULT '[]',
    revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0),
    PRIMARY KEY (account_id, conversation_id)
);
CREATE TABLE IF NOT EXISTS journal_revisions (
    account_id TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK (revision >= 0)
);
";

/// Base locale du bureau pour une session d'essai.
pub struct SessionStore {
    connection: Connection,
    account_id: AccountId,
    database_path: PathBuf,
}

type SubmissionKey = (ConversationId, MessageId);

/// Journal chargé avec la révision et l'origine de cette image.
/// Les opérations du journal sont accessibles via `Deref` et `DerefMut`.
pub struct StoredJournal {
    journal: Journal,
    revision: i64,
    /// Lignes du compte telles qu'elles sont sur disque à `revision`.
    saved: BTreeMap<SubmissionKey, CommittedSubmission>,
    account_id: AccountId,
    database_path: PathBuf,
}

impl std::ops::Deref for StoredJournal {
    type Target = Journal;

    fn deref(&self) -> &Self::Target {
        &self.journal
    }
}

impl std::ops::DerefMut for StoredJournal {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.journal
    }
}

/// Réplique chargée avec la révision et l'origine de cette image.
/// Une conversation absente est représentée par une réplique vide, révision zéro.
pub struct StoredReplica {
    replica: Replica,
    revision: i64,
    account_id: AccountId,
    conversation_id: ConversationId,
    database_path: PathBuf,
}

impl std::ops::Deref for StoredReplica {
    type Target = Replica;

    fn deref(&self) -> &Self::Target {
        &self.replica
    }
}

impl std::ops::DerefMut for StoredReplica {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.replica
    }
}

impl SessionStore {
    /// Ouvre ou crée la base du compte. Le dossier parent est créé s'il manque.
    /// Un schéma futur et les répliques anciennes sans propriétaire sont refusés.
    pub fn open(path: &Path, account_id: &AccountId) -> Result<Self, StoreError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        let mut connection = Connection::open(path)?;
        // Align with the wrapper/provider's explicit three-second wait policy
        // (rusqlite otherwise defaults to five seconds). Read-then-write callers
        // must also use IMMEDIATE transactions to avoid WAL snapshot upgrades.
        connection.busy_timeout(std::time::Duration::from_secs(3))?;
        migrate(&mut connection)?;
        Ok(Self {
            connection,
            account_id: account_id.clone(),
            database_path: fs::canonicalize(path)?,
        })
    }

    /// Remplace seulement le journal du compte par l'image courante.
    /// Une écriture concurrente impose de recharger le journal avant de réessayer.
    /// Le jeton de cette image avance seulement si le commit réussit.
    /// Seules les lignes qui diffèrent de `journal.saved` sont écrites. C'est un
    /// remplacement complet tant que cette méthode est le seul écrivain des
    /// soumissions : la révision vérifiée prouve alors que le disque égale `saved`.
    /// La révision avance même sans ligne modifiée.
    pub fn save_journal(&mut self, journal: &mut StoredJournal) -> Result<(), StoreError> {
        self.check_origin(&journal.account_id, &journal.database_path)?;
        if journal
            .committed()
            .any(|item| item.account_id != self.account_id)
        {
            return Err(StoreError::AccountMismatch);
        }
        for item in journal.committed() {
            validate_submission(item)?;
        }
        let (deleted, changed) = diff_saved(&journal.saved, journal.committed());
        let expected = journal.revision;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let actual = journal_revision(&tx, &self.account_id)?;
        if actual != expected {
            return Err(StoreError::StaleJournal { expected, actual });
        }
        let next = actual
            .checked_add(1)
            .ok_or_else(|| StoreError::Invalid("révision du journal épuisée".into()))?;
        {
            let mut delete = tx.prepare_cached(
                "DELETE FROM submissions
                 WHERE account_id = ?1 AND conversation_id = ?2 AND message_id = ?3",
            )?;
            for key in &deleted {
                delete.execute(params![
                    self.account_id.as_str(),
                    key.0.as_str(),
                    key.1.as_str(),
                ])?;
            }
            let mut upsert = tx.prepare_cached(
                "INSERT INTO submissions (
                    account_id, generation, conversation_id, message_id, body, state_json
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(account_id, conversation_id, message_id) DO UPDATE SET
                    generation = excluded.generation,
                    body = excluded.body,
                    state_json = excluded.state_json",
            )?;
            for item in &changed {
                let state_json = serde_json::to_string(&submission_to_dto(&item.state))?;
                upsert.execute(params![
                    item.account_id.as_str(),
                    item.generation.0,
                    item.conversation_id.as_str(),
                    item.message_id.as_str(),
                    item.body,
                    state_json,
                ])?;
            }
        }
        tx.execute(
            "INSERT INTO journal_revisions (account_id, revision) VALUES (?1, ?2)
             ON CONFLICT(account_id) DO UPDATE SET revision = excluded.revision",
            params![self.account_id.as_str(), next],
        )?;
        tx.commit()?;
        journal.revision = next;
        for key in &deleted {
            journal.saved.remove(key);
        }
        for item in changed {
            journal.saved.insert(
                (item.conversation_id.clone(), item.message_id.clone()),
                item,
            );
        }
        Ok(())
    }

    /// Recharge le journal validé du compte et sa révision dans la même lecture.
    /// Une base vide donne un journal vide.
    pub fn load_journal(&self) -> Result<StoredJournal, StoreError> {
        let (items, revision) = self.read_journal()?;
        let saved = items
            .iter()
            .map(|item| {
                (
                    (item.conversation_id.clone(), item.message_id.clone()),
                    item.clone(),
                )
            })
            .collect();
        Ok(StoredJournal {
            journal: Journal::from_committed(items),
            revision,
            saved,
            account_id: self.account_id.clone(),
            database_path: self.database_path.clone(),
        })
    }

    fn read_journal(&self) -> Result<(Vec<CommittedSubmission>, i64), StoreError> {
        let tx = self.connection.unchecked_transaction()?;
        let revision = journal_revision(&tx, &self.account_id)?;
        let mut statement = tx.prepare(
            "SELECT account_id, generation, conversation_id, message_id, body, state_json
             FROM submissions WHERE account_id = ?1
             ORDER BY conversation_id, message_id",
        )?;
        let rows = statement.query_map(params![self.account_id.as_str()], submission_columns)?;
        let mut items = Vec::new();
        for row in rows {
            items.push(submission_from_columns(row?)?);
        }
        drop(statement);
        tx.commit()?;
        Ok((items, revision))
    }

    /// Remplace l'image de cette conversation si sa révision n'a pas changé.
    /// Le jeton de cette image avance seulement si le commit réussit.
    pub fn save_replica(&mut self, replica: &mut StoredReplica) -> Result<(), StoreError> {
        self.check_origin(&replica.account_id, &replica.database_path)?;
        let image = replica.export();
        if image.conversation_id != replica.conversation_id {
            return Err(StoreError::Invalid(
                "la réplique a changé de conversation".into(),
            ));
        }
        let epoch_json = match &image.epoch {
            Some(epoch) => {
                let cursor = grokbot_domain::ResumeCursor {
                    epoch: epoch.clone(),
                    covered_through: grokbot_domain::UpdateSequence(0),
                    complete: false,
                };
                Some(serde_json::to_string(&cursor_to_dto(&cursor).epoch)?)
            }
            None => None,
        };
        let log_json = encode_updates(&image.log)?;
        let buffer_json = encode_updates(&image.buffer)?;
        let hydrations_json = encode_hydrations(&image.hydrations)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let actual: i64 = tx
            .query_row(
                "SELECT revision FROM replicas WHERE account_id = ?1 AND conversation_id = ?2",
                params![self.account_id.as_str(), image.conversation_id.as_str()],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(0);
        if actual != replica.revision {
            return Err(StoreError::StaleReplica {
                expected: replica.revision,
                actual,
            });
        }
        let next = actual
            .checked_add(1)
            .ok_or_else(|| StoreError::Invalid("révision de réplique épuisée".into()))?;
        tx.execute(
            "INSERT INTO replicas (
                account_id, conversation_id, epoch_json, log_json, buffer_json, hydrations_json, revision
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(account_id, conversation_id) DO UPDATE SET
                epoch_json = excluded.epoch_json,
                log_json = excluded.log_json,
                buffer_json = excluded.buffer_json,
                hydrations_json = excluded.hydrations_json,
                revision = excluded.revision",
            params![
                self.account_id.as_str(),
                image.conversation_id.as_str(),
                epoch_json,
                log_json,
                buffer_json,
                hydrations_json,
                next,
            ],
        )?;
        tx.commit()?;
        replica.revision = next;
        Ok(())
    }

    /// Recharge la conversation du compte, ou une image vide si elle est absente.
    pub fn load_replica(
        &self,
        conversation_id: &ConversationId,
    ) -> Result<StoredReplica, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT epoch_json, log_json, buffer_json, hydrations_json, revision FROM replicas
             WHERE account_id = ?1 AND conversation_id = ?2",
        )?;
        let mut rows =
            statement.query(params![self.account_id.as_str(), conversation_id.as_str()])?;
        let Some(row) = rows.next()? else {
            return Ok(StoredReplica {
                replica: Replica::new(conversation_id.clone()),
                revision: 0,
                account_id: self.account_id.clone(),
                conversation_id: conversation_id.clone(),
                database_path: self.database_path.clone(),
            });
        };
        let epoch_json: Option<String> = row.get(0)?;
        let log_json: String = row.get(1)?;
        let buffer_json: String = row.get(2)?;
        let hydrations_json: String = row.get(3)?;
        let revision: i64 = row.get(4)?;
        if revision < 0 {
            return Err(StoreError::Invalid("révision de réplique négative".into()));
        }
        let epoch = match epoch_json {
            Some(raw) => {
                let dto: ReplicaEpochDto = serde_json::from_str(&raw)?;
                let cursor = cursor_from_dto(ResumeCursorDto {
                    schema_version: grokbot_wire::SCHEMA_VERSION,
                    epoch: dto,
                    covered_through: 0,
                    complete: false,
                })?;
                Some(cursor.epoch)
            }
            None => None,
        };
        let image = ReplicaImage {
            conversation_id: conversation_id.clone(),
            epoch,
            log: decode_updates(&log_json)?,
            buffer: decode_updates(&buffer_json)?,
            hydrations: decode_hydrations(&hydrations_json)?,
        };
        Ok(StoredReplica {
            replica: Replica::restore(image)?,
            revision,
            account_id: self.account_id.clone(),
            conversation_id: conversation_id.clone(),
            database_path: self.database_path.clone(),
        })
    }

    /// Soumissions validées, pour l'affichage. Ne rafraîchit pas la révision d'écriture.
    pub fn list_submissions(&self) -> Result<Vec<CommittedSubmission>, StoreError> {
        Ok(self.read_journal()?.0)
    }

    /// Soumissions validées d'une seule conversation, dans l'ordre des messages.
    pub fn list_conversation_submissions(
        &self,
        conversation: &ConversationId,
    ) -> Result<Vec<CommittedSubmission>, StoreError> {
        let mut statement = self.connection.prepare(
            "SELECT account_id, generation, conversation_id, message_id, body, state_json
             FROM submissions WHERE account_id = ?1 AND conversation_id = ?2
             ORDER BY message_id",
        )?;
        let rows = statement.query_map(
            params![self.account_id.as_str(), conversation.as_str()],
            submission_columns,
        )?;
        let mut items = Vec::new();
        for row in rows {
            items.push(submission_from_columns(row?)?);
        }
        Ok(items)
    }

    fn check_origin(&self, account: &AccountId, path: &Path) -> Result<(), StoreError> {
        if account != &self.account_id {
            return Err(StoreError::AccountMismatch);
        }
        if path != self.database_path {
            return Err(StoreError::DatabaseMismatch);
        }
        Ok(())
    }
}

type SubmissionColumns = (String, i64, String, String, String, String);

fn submission_columns(row: &rusqlite::Row<'_>) -> rusqlite::Result<SubmissionColumns> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
    ))
}

fn submission_from_columns(
    (account_id, generation, conversation_id, message_id, body, state_json): SubmissionColumns,
) -> Result<CommittedSubmission, StoreError> {
    if generation < 0 {
        return Err(StoreError::Invalid("génération négative".into()));
    }
    let dto: SubmissionStateDto = serde_json::from_str(&state_json)?;
    let item = CommittedSubmission {
        account_id: AccountId::new(account_id).map_err(|_| StoreError::empty_id())?,
        generation: AccountGeneration(generation as u64),
        conversation_id: ConversationId::new(conversation_id)
            .map_err(|_| StoreError::empty_id())?,
        message_id: MessageId::new(message_id).map_err(|_| StoreError::empty_id())?,
        body,
        state: submission_from_dto(dto)?,
    };
    validate_submission(&item)?;
    Ok(item)
}

fn migrate(connection: &mut Connection) -> Result<(), StoreError> {
    let tx = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: i64 = tx.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if !(0..=SCHEMA_VERSION).contains(&version) {
        return Err(StoreError::UnsupportedSchema {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    if version == 0 {
        let replicas_exist: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'replicas')",
            [],
            |row| row.get(0),
        )?;
        if replicas_exist {
            let replicas_count: i64 =
                tx.query_row("SELECT COUNT(*) FROM replicas", [], |row| row.get(0))?;
            if replicas_count != 0 {
                return Err(StoreError::LegacyReplicaOwnership);
            }
            tx.execute("DROP TABLE replicas", [])?;
        }
        tx.execute_batch(SCHEMA)?;
        // Une ancienne table incompatible doit annuler aussi les opérations DDL précédentes.
        tx.prepare("SELECT account_id, generation, conversation_id, message_id, body, state_json FROM submissions LIMIT 0")?;
        tx.prepare("SELECT account_id, conversation_id, epoch_json, log_json, buffer_json, hydrations_json FROM replicas LIMIT 0")?;
        tx.prepare("SELECT account_id, revision FROM journal_revisions LIMIT 0")?;
    } else if version == 1 {
        tx.execute("ALTER TABLE replicas ADD COLUMN revision INTEGER NOT NULL DEFAULT 0 CHECK (revision >= 0)", [])?;
    }
    tx.prepare("SELECT account_id, conversation_id, epoch_json, log_json, buffer_json, hydrations_json, revision FROM replicas LIMIT 0")?;
    if version < SCHEMA_VERSION {
        tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    }
    tx.commit()?;
    Ok(())
}

fn journal_revision(connection: &Connection, account: &AccountId) -> Result<i64, StoreError> {
    let revision: Option<i64> = connection
        .query_row(
            "SELECT revision FROM journal_revisions WHERE account_id = ?1",
            params![account.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    match revision {
        Some(value) if value < 0 => Err(StoreError::Invalid("révision négative".into())),
        Some(value) => Ok(value),
        None => Ok(0),
    }
}

/// `journal` doit être ordonné par clé, comme `saved`.
fn diff_saved<'a>(
    saved: &BTreeMap<SubmissionKey, CommittedSubmission>,
    journal: impl Iterator<Item = &'a CommittedSubmission>,
) -> (Vec<SubmissionKey>, Vec<CommittedSubmission>) {
    let mut deleted = Vec::new();
    let mut changed = Vec::new();
    let mut rows = saved.iter().peekable();
    for item in journal {
        let key = (&item.conversation_id, &item.message_id);
        while let Some((saved_key, _)) = rows.next_if(|(k, _)| (&k.0, &k.1) < key) {
            deleted.push(saved_key.clone());
        }
        match rows.next_if(|(k, _)| (&k.0, &k.1) == key) {
            Some((_, row)) if row == item => {}
            _ => changed.push(item.clone()),
        }
    }
    deleted.extend(rows.map(|(key, _)| key.clone()));
    (deleted, changed)
}

fn validate_submission(item: &CommittedSubmission) -> Result<(), StoreError> {
    if let grokbot_domain::SubmissionState::AcceptedAwaitingEcho { receipt } = &item.state {
        if receipt.account_id != item.account_id
            || receipt.conversation_id != item.conversation_id
            || receipt.message_id != item.message_id
        {
            return Err(StoreError::Invalid(
                "le reçu ne correspond pas à la clé de soumission".into(),
            ));
        }
    }
    Ok(())
}

fn encode_updates(updates: &[grokbot_domain::TranscriptUpdate]) -> Result<String, StoreError> {
    let dtos: Vec<TranscriptUpdateDto> = updates.iter().map(update_to_dto).collect();
    Ok(serde_json::to_string(&dtos)?)
}

fn decode_updates(raw: &str) -> Result<Vec<grokbot_domain::TranscriptUpdate>, StoreError> {
    let dtos: Vec<TranscriptUpdateDto> = serde_json::from_str(raw)?;
    dtos.into_iter()
        .map(|dto| update_from_dto(dto).map_err(StoreError::from))
        .collect()
}

#[derive(serde::Serialize, serde::Deserialize)]
struct HydrationDto {
    expected_digest: [u8; 32],
    update: TranscriptUpdateDto,
}

fn encode_hydrations(hydrations: &[BodyHydration]) -> Result<String, StoreError> {
    let dtos: Vec<HydrationDto> = hydrations
        .iter()
        .map(|hydration| HydrationDto {
            expected_digest: hydration.expected_digest.0,
            update: update_to_dto(&grokbot_domain::TranscriptUpdate {
                sequence: hydration.sequence,
                entry_id: hydration.entry_id.clone(),
                kind: grokbot_domain::UpdateKind::Upsert {
                    body: hydration.body.clone(),
                },
            }),
        })
        .collect();
    Ok(serde_json::to_string(&dtos)?)
}

fn decode_hydrations(raw: &str) -> Result<Vec<BodyHydration>, StoreError> {
    let dtos: Vec<HydrationDto> = serde_json::from_str(raw)?;
    dtos.into_iter()
        .map(|dto| {
            let update = update_from_dto(dto.update)?;
            let grokbot_domain::UpdateKind::Upsert { body } = update.kind else {
                return Err(StoreError::Invalid("hydratation sans corps".into()));
            };
            Ok(BodyHydration {
                sequence: update.sequence,
                entry_id: update.entry_id,
                expected_digest: grokbot_domain::ContentDigest(dto.expected_digest),
                body,
            })
        })
        .collect()
}

/// Échec de lecture ou d'écriture de la base.
#[derive(Debug)]
pub enum StoreError {
    /// SQLite a refusé l'opération.
    Sqlite(rusqlite::Error),
    /// Le JSON stocké n'est pas lisible.
    Json(serde_json::Error),
    /// Un DTO versionné a été refusé.
    Wire(grokbot_wire::WireError),
    /// L'image de réplique est incohérente.
    Replica(grokbot_replica::ReplicaError),
    /// Le dossier de la base n'a pas pu être créé.
    Io(std::io::Error),
    /// Une valeur stockée sort du contrat.
    Invalid(String),
    /// Le logiciel ne connaît pas cette version du schéma SQLite.
    UnsupportedSchema {
        /// Version déclarée par la base.
        found: i64,
        /// Dernière version acceptée.
        supported: i64,
    },
    /// Les répliques anciennes ne déclarent pas leur compte propriétaire.
    LegacyReplicaOwnership,
    /// Le journal fourni contient une soumission d'un autre compte.
    AccountMismatch,
    /// L'image a été chargée depuis une autre base SQLite.
    DatabaseMismatch,
    /// Un autre écrivain a validé une nouvelle image depuis la dernière lecture.
    StaleJournal {
        /// Révision capturée par ce stockage.
        expected: i64,
        /// Révision actuellement validée dans la base.
        actual: i64,
    },
    /// Un autre écrivain a validé une nouvelle réplique depuis la lecture de l'image.
    StaleReplica {
        /// Révision portée par l'image proposée.
        expected: i64,
        /// Révision actuellement validée dans la base.
        actual: i64,
    },
}

impl StoreError {
    fn empty_id() -> Self {
        Self::Invalid("identifiant vide".into())
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sqlite(error) => write!(formatter, "sqlite : {error}"),
            Self::Json(error) => write!(formatter, "json : {error}"),
            Self::Wire(error) => write!(formatter, "contrat : {error}"),
            Self::Replica(error) => write!(formatter, "réplique : {error}"),
            Self::Io(error) => write!(formatter, "fichier : {error}"),
            Self::Invalid(detail) => write!(formatter, "donnée invalide : {detail}"),
            Self::UnsupportedSchema { found, supported } => write!(
                formatter,
                "schéma SQLite {found} non pris en charge (maximum {supported})"
            ),
            Self::LegacyReplicaOwnership => formatter.write_str(
                "migration refusée : les répliques anciennes ne déclarent pas leur compte ; conserver cette base et utiliser une nouvelle base"
            ),
            Self::AccountMismatch => formatter.write_str(
                "journal refusé : une soumission appartient à un autre compte"
            ),
            Self::DatabaseMismatch => formatter.write_str(
                "image refusée : elle appartient à une autre base SQLite"
            ),
            Self::StaleJournal { expected, actual } => write!(
                formatter,
                "journal périmé : révision {expected}, révision actuelle {actual} ; recharger avant de sauvegarder"
            ),
            Self::StaleReplica { expected, actual } => write!(
                formatter,
                "réplique périmée : révision {expected}, révision actuelle {actual} ; recharger avant de sauvegarder"
            ),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(value: rusqlite::Error) -> Self {
        Self::Sqlite(value)
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<grokbot_wire::WireError> for StoreError {
    fn from(value: grokbot_wire::WireError) -> Self {
        Self::Wire(value)
    }
}

impl From<grokbot_replica::ReplicaError> for StoreError {
    fn from(value: grokbot_replica::ReplicaError) -> Self {
        Self::Replica(value)
    }
}

impl From<std::io::Error> for StoreError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use grokbot_domain::{
        AccountId, AttemptId, AuthenticatedActor, BootId, ConversationId, DeliveryReceipt,
        EntryBody, MessageId, ReplicaEpoch, ServerGeneration, TranscriptBody, TranscriptEntryId,
        TranscriptUpdate, UpdateKind, UpdateSequence,
    };
    use grokbot_replica::{Delta, Replica, Snapshot};
    use grokbot_submission_journal::{Journal, PreparedMessage};

    fn must_id<T>(result: Result<T, grokbot_domain::EmptyId>) -> T {
        result.expect("identifiant")
    }

    struct TestDatabase(std::path::PathBuf);

    impl TestDatabase {
        fn new(name: &str) -> Self {
            static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let serial = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "grokbot-store-{name}-{}-{serial}.sqlite",
                std::process::id()
            ));
            let _ = fs::remove_file(&path);
            Self(path)
        }
    }

    impl Drop for TestDatabase {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    fn open_store(path: &Path, account: &AccountId) -> Result<SessionStore, StoreError> {
        SessionStore::open(path, account)
    }

    fn save_fixture_journal(store: &mut SessionStore, journal: &Journal) -> Result<(), StoreError> {
        let mut image = store.load_journal()?;
        image.journal = Journal::from_committed(journal.export_committed());
        store.save_journal(&mut image)
    }

    fn save_fixture_replica(store: &mut SessionStore, replica: &Replica) -> Result<(), StoreError> {
        let original = replica.export();
        let mut image = store.load_replica(&original.conversation_id)?;
        image.replica = Replica::restore(original)?;
        store.save_replica(&mut image)
    }

    fn journal_for(account: &AccountId, message: &str, body: &str) -> Journal {
        let mut journal = Journal::new();
        journal
            .prepare(PreparedMessage {
                actor: AuthenticatedActor {
                    account_id: account.clone(),
                    generation: AccountGeneration(1),
                    team_id: None,
                },
                conversation_id: must_id(ConversationId::new("shared-conversation")),
                message_id: must_id(MessageId::new(message)),
                body: body.into(),
            })
            .unwrap();
        journal
    }

    fn replica_with_text(text: &str) -> Replica {
        let conversation = must_id(ConversationId::new("shared-conversation"));
        let mut replica = Replica::new(conversation.clone());
        replica
            .apply_snapshot(Snapshot {
                conversation_id: conversation,
                epoch: ReplicaEpoch {
                    boot_id: must_id(BootId::new("boot")),
                    server_generation: ServerGeneration(1),
                    recovery: 0,
                },
                updates: vec![TranscriptUpdate {
                    sequence: UpdateSequence(1),
                    entry_id: must_id(TranscriptEntryId::new("shared-entry")),
                    kind: UpdateKind::Upsert {
                        body: TranscriptBody::Inline {
                            content: EntryBody::Text(text.into()),
                        },
                    },
                }],
            })
            .unwrap();
        replica
    }

    #[test]
    fn sauvegarder_un_compte_preserve_le_journal_des_autres() {
        let db = TestDatabase::new("journal-isolation");
        let alice = must_id(AccountId::new("alice"));
        let bob = must_id(AccountId::new("bob"));
        let mut store_alice = open_store(&db.0, &alice).unwrap();
        let mut store_bob = open_store(&db.0, &bob).unwrap();
        save_fixture_journal(
            &mut store_alice,
            &journal_for(&alice, "same-message", "alice-body"),
        )
        .unwrap();
        save_fixture_journal(
            &mut store_bob,
            &journal_for(&bob, "same-message", "bob-body"),
        )
        .unwrap();
        assert_eq!(store_alice.list_submissions().unwrap().len(), 1);
        assert_eq!(
            store_alice.list_submissions().unwrap()[0].body,
            "alice-body"
        );
        assert_eq!(store_bob.list_submissions().unwrap().len(), 1);
        assert_eq!(store_bob.list_submissions().unwrap()[0].body, "bob-body");
        save_fixture_journal(&mut store_alice, &Journal::new()).unwrap();
        assert!(store_alice.list_submissions().unwrap().is_empty());
        assert_eq!(store_bob.list_submissions().unwrap()[0].body, "bob-body");
    }

    #[test]
    fn lister_une_conversation_ne_rend_que_ses_soumissions() {
        let db = TestDatabase::new("liste-par-conversation");
        let account = must_id(AccountId::new("alice"));
        let mut store = open_store(&db.0, &account).unwrap();
        let mut journal = Journal::new();
        for (conversation, message, body) in [
            ("conversation-a", "m1", "a-premier"),
            ("conversation-b", "m1", "b-premier"),
            ("conversation-a", "m2", "a-second"),
        ] {
            journal
                .prepare(PreparedMessage {
                    actor: AuthenticatedActor {
                        account_id: account.clone(),
                        generation: AccountGeneration(1),
                        team_id: None,
                    },
                    conversation_id: must_id(ConversationId::new(conversation)),
                    message_id: must_id(MessageId::new(message)),
                    body: body.into(),
                })
                .unwrap();
        }
        save_fixture_journal(&mut store, &journal).unwrap();
        let listed = store
            .list_conversation_submissions(&must_id(ConversationId::new("conversation-a")))
            .unwrap();
        let bodies: Vec<_> = listed.iter().map(|item| item.body.as_str()).collect();
        assert_eq!(bodies, ["a-premier", "a-second"]);
        assert!(store
            .list_conversation_submissions(&must_id(ConversationId::new("inconnue")))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn meme_conversation_ne_partage_pas_la_replique_entre_comptes() {
        let db = TestDatabase::new("replica-isolation");
        let alice = must_id(AccountId::new("alice"));
        let bob = must_id(AccountId::new("bob"));
        let conversation = must_id(ConversationId::new("shared-conversation"));
        let entry = must_id(TranscriptEntryId::new("shared-entry"));
        let mut store_alice = open_store(&db.0, &alice).unwrap();
        let mut store_bob = open_store(&db.0, &bob).unwrap();
        save_fixture_replica(&mut store_alice, &replica_with_text("alice")).unwrap();
        assert!(store_bob
            .load_replica(&conversation)
            .unwrap()
            .cursor()
            .is_none());
        save_fixture_replica(&mut store_bob, &replica_with_text("bob")).unwrap();
        let loaded = store_alice.load_replica(&conversation).unwrap();
        assert_eq!(
            loaded.entry(&entry),
            Some(&TranscriptBody::Inline {
                content: EntryBody::Text("alice".into()),
            })
        );
        drop(store_bob);
        let reopened = open_store(&db.0, &bob).unwrap();
        assert_eq!(
            reopened.load_replica(&conversation).unwrap().entry(&entry),
            Some(&TranscriptBody::Inline {
                content: EntryBody::Text("bob".into()),
            })
        );
    }

    #[test]
    fn un_journal_melange_est_refuse_sans_effacer_le_compte() {
        let db = TestDatabase::new("mixed-journal");
        let alice = must_id(AccountId::new("alice"));
        let bob = must_id(AccountId::new("bob"));
        let mut store = open_store(&db.0, &alice).unwrap();
        save_fixture_journal(&mut store, &journal_for(&alice, "old", "old-body")).unwrap();
        let mut items = journal_for(&alice, "new", "new-body").export_committed();
        items.extend(journal_for(&bob, "other", "bob-body").export_committed());
        assert!(save_fixture_journal(&mut store, &Journal::from_committed(items)).is_err());
        let loaded = store.list_submissions().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].body, "old-body");
    }

    #[test]
    fn echec_sqlite_annule_toute_la_sauvegarde_et_permet_la_reprise() {
        let db = TestDatabase::new("rollback");
        let account = must_id(AccountId::new("alice"));
        let mut store = open_store(&db.0, &account).unwrap();
        save_fixture_journal(&mut store, &journal_for(&account, "old", "old-body")).unwrap();
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER reject_message BEFORE INSERT ON submissions
             WHEN NEW.message_id = 'blocked'
             BEGIN SELECT RAISE(ABORT, 'forced insertion failure'); END;",
            )
            .unwrap();
        let mut items = journal_for(&account, "allowed", "new-body").export_committed();
        items.extend(journal_for(&account, "blocked", "blocked-body").export_committed());
        let mut journal = store.load_journal().unwrap();
        journal.journal = Journal::from_committed(items);
        assert!(store.save_journal(&mut journal).is_err());
        let loaded = store.list_submissions().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].body, "old-body");
        store
            .connection
            .execute("DROP TRIGGER reject_message", [])
            .unwrap();
        store.save_journal(&mut journal).unwrap();
        assert_eq!(store.list_submissions().unwrap().len(), 2);
    }

    fn actor_for(account: &AccountId) -> AuthenticatedActor {
        AuthenticatedActor {
            account_id: account.clone(),
            generation: AccountGeneration(1),
            team_id: None,
        }
    }

    fn prepare_in(journal: &mut StoredJournal, account: &AccountId, message: &str, body: &str) {
        journal
            .prepare(PreparedMessage {
                actor: actor_for(account),
                conversation_id: must_id(ConversationId::new("conversation")),
                message_id: must_id(MessageId::new(message)),
                body: body.into(),
            })
            .unwrap();
    }

    #[test]
    fn un_message_retire_disparait_du_disque_apres_la_sauvegarde() {
        let db = TestDatabase::new("discard-on-disk");
        let account = must_id(AccountId::new("alice"));
        let mut store = open_store(&db.0, &account).unwrap();
        let mut image = store.load_journal().unwrap();
        prepare_in(&mut image, &account, "one", "premier");
        prepare_in(&mut image, &account, "two", "second");
        store.save_journal(&mut image).unwrap();
        assert_eq!(store.list_submissions().unwrap().len(), 2);

        image
            .discard_undelivered(
                &actor_for(&account),
                &must_id(ConversationId::new("conversation")),
                &must_id(MessageId::new("one")),
            )
            .unwrap();
        store.save_journal(&mut image).unwrap();

        let bodies: Vec<_> = store
            .list_submissions()
            .unwrap()
            .into_iter()
            .map(|item| item.body)
            .collect();
        assert_eq!(bodies, ["second"]);
    }

    #[test]
    fn recharger_apres_plusieurs_sauvegardes_rend_exactement_l_image_en_memoire() {
        let db = TestDatabase::new("save-round-trip");
        let account = must_id(AccountId::new("alice"));
        let conversation = must_id(ConversationId::new("conversation"));
        let mut store = open_store(&db.0, &account).unwrap();
        let mut image = store.load_journal().unwrap();
        let assert_round_trip = |store: &SessionStore, image: &StoredJournal| {
            assert_eq!(
                store.load_journal().unwrap().export_committed(),
                image.export_committed()
            );
        };

        store.save_journal(&mut image).unwrap();
        assert_round_trip(&store, &image);
        prepare_in(&mut image, &account, "a", "alpha");
        prepare_in(&mut image, &account, "b", "beta");
        store.save_journal(&mut image).unwrap();
        assert_round_trip(&store, &image);
        image
            .dispatch(
                &actor_for(&account),
                &conversation,
                &must_id(MessageId::new("a")),
                must_id(AttemptId::new("send-1")),
            )
            .unwrap();
        prepare_in(&mut image, &account, "c", "gamma");
        store.save_journal(&mut image).unwrap();
        assert_round_trip(&store, &image);
        image
            .discard_undelivered(
                &actor_for(&account),
                &conversation,
                &must_id(MessageId::new("b")),
            )
            .unwrap();
        store.save_journal(&mut image).unwrap();
        assert_round_trip(&store, &image);
        store.save_journal(&mut image).unwrap();
        assert_round_trip(&store, &image);
        assert_eq!(store.list_submissions().unwrap().len(), 2);
    }

    #[test]
    fn un_ecrivain_perime_ne_remplace_pas_un_journal_plus_recent() {
        let db = TestDatabase::new("stale-writer");
        let account = must_id(AccountId::new("alice"));
        let mut first = open_store(&db.0, &account).unwrap();
        let mut second = open_store(&db.0, &account).unwrap();
        let mut stale_journal = second.load_journal().unwrap();
        save_fixture_journal(&mut first, &journal_for(&account, "one", "latest-body")).unwrap();
        assert!(second.save_journal(&mut stale_journal).is_err());
        assert_eq!(first.list_submissions().unwrap()[0].body, "latest-body");
        let mut refreshed = second.load_journal().unwrap();
        second.save_journal(&mut refreshed).unwrap();
        assert_eq!(second.list_submissions().unwrap()[0].body, "latest-body");
    }

    #[test]
    fn un_schema_futur_est_refuse_sans_mutation() {
        let db = TestDatabase::new("future-schema");
        let connection = Connection::open(&db.0).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE future_marker (body TEXT NOT NULL);
             INSERT INTO future_marker VALUES ('preserved');
             PRAGMA user_version = 999;",
            )
            .unwrap();
        drop(connection);
        let before = fs::read(&db.0).unwrap();
        assert!(open_store(&db.0, &must_id(AccountId::new("alice"))).is_err());
        assert_eq!(fs::read(&db.0).unwrap(), before);
    }

    #[test]
    fn un_schema_negatif_est_refuse_sans_mutation() {
        let db = TestDatabase::new("negative-schema");
        let account = must_id(AccountId::new("alice"));
        let store = open_store(&db.0, &account).unwrap();
        store
            .connection
            .pragma_update(None, "user_version", -1)
            .unwrap();
        drop(store);
        let before = fs::read(&db.0).unwrap();
        assert!(matches!(
            open_store(&db.0, &account),
            Err(StoreError::UnsupportedSchema { found: -1, .. })
        ));
        assert_eq!(fs::read(&db.0).unwrap(), before);
    }

    fn create_legacy_database(path: &Path) -> Connection {
        let connection = Connection::open(path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE submissions (
                account_id TEXT NOT NULL, generation INTEGER NOT NULL,
                conversation_id TEXT NOT NULL, message_id TEXT NOT NULL,
                body TEXT NOT NULL, state_json TEXT NOT NULL,
                PRIMARY KEY (account_id, conversation_id, message_id)
             );
             CREATE TABLE replicas (
                conversation_id TEXT PRIMARY KEY, epoch_json TEXT,
                log_json TEXT NOT NULL, buffer_json TEXT NOT NULL
             );",
            )
            .unwrap();
        connection
    }

    #[test]
    fn une_replique_legacy_sans_proprietaire_est_refusee_sans_mutation() {
        let db = TestDatabase::new("legacy-replica");
        let connection = create_legacy_database(&db.0);
        connection
            .execute(
                "INSERT INTO replicas VALUES ('conversation', NULL, '[]', '[]')",
                [],
            )
            .unwrap();
        drop(connection);
        let before = fs::read(&db.0).unwrap();
        assert!(open_store(&db.0, &must_id(AccountId::new("alice"))).is_err());
        assert_eq!(fs::read(&db.0).unwrap(), before);
    }

    #[test]
    fn migration_legacy_preserve_les_soumissions_de_chaque_compte() {
        let db = TestDatabase::new("legacy-submissions");
        let alice = must_id(AccountId::new("alice"));
        let bob = must_id(AccountId::new("bob"));
        let connection = create_legacy_database(&db.0);
        for account in [&alice, &bob] {
            let item = journal_for(account, "same-message", account.as_str())
                .export_committed()
                .pop()
                .unwrap();
            connection
                .execute(
                    "INSERT INTO submissions VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![
                        item.account_id.as_str(),
                        item.generation.0,
                        item.conversation_id.as_str(),
                        item.message_id.as_str(),
                        item.body,
                        serde_json::to_string(&submission_to_dto(&item.state)).unwrap()
                    ],
                )
                .unwrap();
        }
        drop(connection);
        let mut store_alice = open_store(&db.0, &alice).unwrap();
        let store_bob = open_store(&db.0, &bob).unwrap();
        assert_eq!(store_alice.list_submissions().unwrap().len(), 1);
        assert_eq!(store_alice.list_submissions().unwrap()[0].body, "alice");
        assert_eq!(store_bob.list_submissions().unwrap().len(), 1);
        assert_eq!(store_bob.list_submissions().unwrap()[0].body, "bob");
        save_fixture_replica(&mut store_alice, &replica_with_text("alice")).unwrap();
        assert!(store_bob
            .load_replica(&must_id(ConversationId::new("shared-conversation")))
            .unwrap()
            .cursor()
            .is_none());
        let version: i64 = store_alice
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert!(version > 0);
    }

    #[test]
    fn migration_incompatible_annule_les_modifications_du_schema() {
        let db = TestDatabase::new("migration-rollback");
        let connection = Connection::open(&db.0).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE submissions (body TEXT);
             CREATE TABLE replicas (
                 conversation_id TEXT PRIMARY KEY, epoch_json TEXT,
                 log_json TEXT NOT NULL, buffer_json TEXT NOT NULL
             );",
            )
            .unwrap();
        drop(connection);
        let before = fs::read(&db.0).unwrap();
        assert!(open_store(&db.0, &must_id(AccountId::new("alice"))).is_err());
        assert_eq!(fs::read(&db.0).unwrap(), before);
    }

    #[test]
    fn afficher_les_soumissions_ne_revalide_pas_un_ecrivain_perime() {
        let db = TestDatabase::new("listing-stale-writer");
        let account = must_id(AccountId::new("alice"));
        let mut first = open_store(&db.0, &account).unwrap();
        let mut second = open_store(&db.0, &account).unwrap();
        let mut stale = second.load_journal().unwrap();
        save_fixture_journal(&mut first, &journal_for(&account, "one", "latest-body")).unwrap();
        assert_eq!(second.list_submissions().unwrap()[0].body, "latest-body");
        assert!(second.save_journal(&mut stale).is_err());
    }

    #[test]
    fn une_image_ancienne_reste_perimee_apres_reouverture() {
        let db = TestDatabase::new("reopen-stale-writer");
        let account = must_id(AccountId::new("alice"));
        let mut first = open_store(&db.0, &account).unwrap();
        let mut stale = first.load_journal().unwrap();
        save_fixture_journal(&mut first, &journal_for(&account, "one", "latest-body")).unwrap();
        drop(first);
        let mut reopened = open_store(&db.0, &account).unwrap();
        assert!(reopened.save_journal(&mut stale).is_err());
        let mut refreshed = reopened.load_journal().unwrap();
        reopened.save_journal(&mut refreshed).unwrap();
        assert_eq!(reopened.list_submissions().unwrap()[0].body, "latest-body");
    }

    #[test]
    fn recharger_ne_revalide_pas_une_ancienne_image_du_journal() {
        let db = TestDatabase::new("image-stale-after-reload");
        let account = must_id(AccountId::new("alice"));
        let mut store = open_store(&db.0, &account).unwrap();
        let mut stale = store.load_journal().unwrap();
        save_fixture_journal(&mut store, &journal_for(&account, "one", "latest-body")).unwrap();
        let _fresh = store.load_journal().unwrap();
        assert!(store.save_journal(&mut stale).is_err());
        assert_eq!(store.list_submissions().unwrap()[0].body, "latest-body");
    }

    #[test]
    fn une_image_de_journal_ne_peut_pas_etre_ecrite_dans_une_autre_base() {
        let first_db = TestDatabase::new("journal-database-origin");
        let second_db = TestDatabase::new("journal-database-target");
        let account = must_id(AccountId::new("alice"));
        let first = open_store(&first_db.0, &account).unwrap();
        let mut second = open_store(&second_db.0, &account).unwrap();
        let mut image = first.load_journal().unwrap();
        assert!(second.save_journal(&mut image).is_err());
    }

    #[test]
    fn une_image_vide_du_journal_ne_peut_pas_changer_de_compte() {
        let db = TestDatabase::new("empty-journal-account-origin");
        let alice = must_id(AccountId::new("alice"));
        let bob = must_id(AccountId::new("bob"));
        let first = open_store(&db.0, &alice).unwrap();
        let mut second = open_store(&db.0, &bob).unwrap();
        let mut image = first.load_journal().unwrap();
        assert!(second.save_journal(&mut image).is_err());
    }

    #[test]
    fn une_replique_perimee_ne_remplace_pas_une_image_plus_complete() {
        let db = TestDatabase::new("replica-stale-writer");
        let account = must_id(AccountId::new("alice"));
        let conversation = must_id(ConversationId::new("shared-conversation"));
        let mut first = open_store(&db.0, &account).unwrap();
        let mut second = open_store(&db.0, &account).unwrap();
        let old = replica_with_text("old-body");
        save_fixture_replica(&mut first, &old).unwrap();
        let mut stale = second.load_replica(&conversation).unwrap();
        let mut fresh = first.load_replica(&conversation).unwrap();
        let epoch = fresh.cursor().unwrap().epoch;
        fresh
            .apply_delta(Delta {
                conversation_id: conversation.clone(),
                epoch,
                updates: vec![TranscriptUpdate {
                    sequence: UpdateSequence(2),
                    entry_id: must_id(TranscriptEntryId::new("shared-entry")),
                    kind: UpdateKind::Upsert {
                        body: TranscriptBody::Inline {
                            content: EntryBody::Text("latest-body".into()),
                        },
                    },
                }],
            })
            .unwrap();
        first.save_replica(&mut fresh).unwrap();
        assert!(second.save_replica(&mut stale).is_err());
        assert_eq!(
            first
                .load_replica(&conversation)
                .unwrap()
                .cursor()
                .unwrap()
                .covered_through,
            UpdateSequence(2)
        );
    }

    #[test]
    fn recharger_ne_revalide_pas_une_ancienne_image_de_replique() {
        let db = TestDatabase::new("replica-image-stale-after-reload");
        let account = must_id(AccountId::new("alice"));
        let conversation = must_id(ConversationId::new("shared-conversation"));
        let mut store = open_store(&db.0, &account).unwrap();
        let mut stale = store.load_replica(&conversation).unwrap();
        save_fixture_replica(&mut store, &replica_with_text("latest-body")).unwrap();
        let _fresh = store.load_replica(&conversation).unwrap();
        assert!(store.save_replica(&mut stale).is_err());
        drop(store);
        let mut reopened = open_store(&db.0, &account).unwrap();
        assert!(reopened.save_replica(&mut stale).is_err());
        assert_eq!(
            reopened
                .load_replica(&conversation)
                .unwrap()
                .cursor()
                .unwrap()
                .covered_through,
            UpdateSequence(1)
        );
    }

    #[test]
    fn une_replique_ne_peut_pas_changer_de_base_ou_de_compte() {
        let db = TestDatabase::new("replica-origin");
        let other_db = TestDatabase::new("replica-target");
        let alice = must_id(AccountId::new("alice"));
        let bob = must_id(AccountId::new("bob"));
        let conversation = must_id(ConversationId::new("conversation"));
        let source = open_store(&db.0, &alice).unwrap();
        let mut other_account = open_store(&db.0, &bob).unwrap();
        let mut other_database = open_store(&other_db.0, &alice).unwrap();
        let mut image = source.load_replica(&conversation).unwrap();
        assert!(matches!(
            other_account.save_replica(&mut image),
            Err(StoreError::AccountMismatch)
        ));
        assert!(matches!(
            other_database.save_replica(&mut image),
            Err(StoreError::DatabaseMismatch)
        ));
    }

    #[test]
    fn une_replique_ne_peut_pas_detourner_le_jeton_d_une_autre_conversation() {
        let db = TestDatabase::new("replica-conversation-origin");
        let account = must_id(AccountId::new("alice"));
        let conversation = must_id(ConversationId::new("conversation"));
        let other_conversation = must_id(ConversationId::new("other-conversation"));
        let mut store = open_store(&db.0, &account).unwrap();
        let mut image = store.load_replica(&conversation).unwrap();
        *image = Replica::new(other_conversation.clone());
        assert!(store.save_replica(&mut image).is_err());
        assert!(store
            .load_replica(&other_conversation)
            .unwrap()
            .cursor()
            .is_none());
    }

    #[test]
    fn echec_ecriture_replique_preserve_le_jeton_et_permet_la_reprise() {
        let db = TestDatabase::new("replica-rollback");
        let account = must_id(AccountId::new("alice"));
        let conversation = must_id(ConversationId::new("shared-conversation"));
        let entry = must_id(TranscriptEntryId::new("shared-entry"));
        let mut store = open_store(&db.0, &account).unwrap();
        save_fixture_replica(&mut store, &replica_with_text("old-body")).unwrap();
        let mut image = store.load_replica(&conversation).unwrap();
        let epoch = image.cursor().unwrap().epoch;
        image
            .apply_delta(Delta {
                conversation_id: conversation.clone(),
                epoch,
                updates: vec![TranscriptUpdate {
                    sequence: UpdateSequence(2),
                    entry_id: entry.clone(),
                    kind: UpdateKind::Upsert {
                        body: TranscriptBody::Inline {
                            content: EntryBody::Text("latest-body".into()),
                        },
                    },
                }],
            })
            .unwrap();
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER reject_replica BEFORE UPDATE ON replicas
             BEGIN SELECT RAISE(ABORT, 'forced replica failure'); END;",
            )
            .unwrap();
        assert!(store.save_replica(&mut image).is_err());
        let previous = store.load_replica(&conversation).unwrap();
        assert_eq!(
            previous.cursor().unwrap().covered_through,
            UpdateSequence(1)
        );
        assert_eq!(
            previous.entry(&entry),
            Some(&TranscriptBody::Inline {
                content: EntryBody::Text("old-body".into())
            })
        );
        store
            .connection
            .execute("DROP TRIGGER reject_replica", [])
            .unwrap();
        store.save_replica(&mut image).unwrap();
        let loaded = store.load_replica(&conversation).unwrap();
        assert_eq!(loaded.cursor().unwrap().covered_through, UpdateSequence(2));
        assert_eq!(
            loaded.entry(&entry),
            Some(&TranscriptBody::Inline {
                content: EntryBody::Text("latest-body".into())
            })
        );
    }

    #[test]
    fn migration_version_un_preserve_la_replique_et_ajoute_ses_revisions() {
        let db = TestDatabase::new("migration-v1");
        let account = must_id(AccountId::new("alice"));
        let conversation = must_id(ConversationId::new("shared-conversation"));
        let connection = Connection::open(&db.0).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE submissions (
                account_id TEXT NOT NULL, generation INTEGER NOT NULL,
                conversation_id TEXT NOT NULL, message_id TEXT NOT NULL,
                body TEXT NOT NULL, state_json TEXT NOT NULL,
                PRIMARY KEY (account_id, conversation_id, message_id)
             );
             CREATE TABLE replicas (
                account_id TEXT NOT NULL, conversation_id TEXT NOT NULL,
                epoch_json TEXT, log_json TEXT NOT NULL, buffer_json TEXT NOT NULL,
                hydrations_json TEXT NOT NULL DEFAULT '[]',
                PRIMARY KEY (account_id, conversation_id)
             );
             CREATE TABLE journal_revisions (
                account_id TEXT PRIMARY KEY, revision INTEGER NOT NULL CHECK (revision >= 0)
             );
             PRAGMA user_version = 1;",
            )
            .unwrap();
        let replica = replica_with_text("version-one-body");
        let image = replica.export();
        connection.execute(
            "INSERT INTO replicas (account_id, conversation_id, epoch_json, log_json, buffer_json)
             VALUES (?1, ?2, ?3, ?4, '[]')",
            params![account.as_str(), conversation.as_str(),
                serde_json::to_string(&cursor_to_dto(&replica.cursor().unwrap()).epoch).unwrap(),
                encode_updates(&image.log).unwrap()],
        ).unwrap();
        drop(connection);
        let mut store = open_store(&db.0, &account).unwrap();
        let mut loaded = store.load_replica(&conversation).unwrap();
        assert_eq!(
            loaded.entry(&must_id(TranscriptEntryId::new("shared-entry"))),
            Some(&TranscriptBody::Inline {
                content: EntryBody::Text("version-one-body".into())
            })
        );
        store.save_replica(&mut loaded).unwrap();
        store.save_replica(&mut loaded).unwrap();
        drop(store);
        let reopened = open_store(&db.0, &account).unwrap();
        let version: i64 = reopened
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 2);
        assert_eq!(
            reopened
                .load_replica(&conversation)
                .unwrap()
                .cursor()
                .unwrap()
                .covered_through,
            UpdateSequence(1)
        );
    }

    fn inconsistent_receipts(account: &AccountId) -> Vec<DeliveryReceipt> {
        vec![
            DeliveryReceipt {
                account_id: must_id(AccountId::new("other-account")),
                conversation_id: must_id(ConversationId::new("shared-conversation")),
                message_id: must_id(MessageId::new("one")),
            },
            DeliveryReceipt {
                account_id: account.clone(),
                conversation_id: must_id(ConversationId::new("other-conversation")),
                message_id: must_id(MessageId::new("one")),
            },
            DeliveryReceipt {
                account_id: account.clone(),
                conversation_id: must_id(ConversationId::new("shared-conversation")),
                message_id: must_id(MessageId::new("other-message")),
            },
        ]
    }

    #[test]
    fn sauvegarder_refuse_un_recu_incoherent_avec_la_cle_durable() {
        let db = TestDatabase::new("receipt-save");
        let account = must_id(AccountId::new("alice"));
        let mut store = open_store(&db.0, &account).unwrap();
        save_fixture_journal(&mut store, &journal_for(&account, "one", "preserved-body")).unwrap();
        for receipt in inconsistent_receipts(&account) {
            let mut items = journal_for(&account, "one", "invalid-body").export_committed();
            items[0].state = grokbot_domain::SubmissionState::AcceptedAwaitingEcho { receipt };
            assert!(save_fixture_journal(&mut store, &Journal::from_committed(items)).is_err());
            assert_eq!(store.list_submissions().unwrap()[0].body, "preserved-body");
        }
    }

    #[test]
    fn charger_refuse_un_recu_incoherent_avec_la_cle_durable() {
        let db = TestDatabase::new("receipt-load");
        let account = must_id(AccountId::new("alice"));
        let mut store = open_store(&db.0, &account).unwrap();
        save_fixture_journal(&mut store, &journal_for(&account, "one", "preserved-body")).unwrap();
        for receipt in inconsistent_receipts(&account) {
            let state = grokbot_domain::SubmissionState::AcceptedAwaitingEcho { receipt };
            store
                .connection
                .execute(
                    "UPDATE submissions SET state_json = ?1",
                    params![serde_json::to_string(&submission_to_dto(&state)).unwrap()],
                )
                .unwrap();
            assert!(store.load_journal().is_err());
        }
    }

    #[test]
    fn hydratation_survit_au_disque_sans_modifier_le_journal_original() {
        let db = TestDatabase::new("hydration");
        let account = must_id(AccountId::new("alice"));
        let conversation = must_id(ConversationId::new("conversation"));
        let entry = must_id(TranscriptEntryId::new("entry"));
        let epoch = ReplicaEpoch {
            boot_id: must_id(BootId::new("boot")),
            server_generation: ServerGeneration(1),
            recovery: 0,
        };
        let digest = grokbot_domain::ContentDigest([7; 32]);
        let pending = TranscriptUpdate {
            sequence: UpdateSequence(1),
            entry_id: entry.clone(),
            kind: UpdateKind::Upsert {
                body: TranscriptBody::Pending {
                    expected_digest: digest,
                },
            },
        };
        let resolved = TranscriptBody::Blob {
            reference: must_id(grokbot_domain::BlobRef::new("blob://retained")),
            digest,
        };
        let mut replica = Replica::new(conversation.clone());
        replica
            .apply_snapshot(Snapshot {
                conversation_id: conversation.clone(),
                epoch: epoch.clone(),
                updates: vec![pending.clone()],
            })
            .unwrap();
        replica
            .fill_body(&epoch, UpdateSequence(1), &entry, resolved.clone())
            .unwrap();
        let mut store = open_store(&db.0, &account).unwrap();
        save_fixture_replica(&mut store, &replica).unwrap();
        drop(store);
        let reopened = open_store(&db.0, &account).unwrap();
        let mut loaded = reopened.load_replica(&conversation).unwrap();
        assert_eq!(loaded.entry(&entry), Some(&resolved));
        assert_eq!(loaded.export().log, vec![pending.clone()]);
        assert_eq!(loaded.cursor().unwrap().covered_through, UpdateSequence(1));
        assert!(loaded.cursor().unwrap().complete);
        loaded
            .apply_delta(Delta {
                conversation_id: conversation,
                epoch,
                updates: vec![pending],
            })
            .unwrap();
        assert_eq!(loaded.entry(&entry), Some(&resolved));
    }

    #[test]
    fn hydratation_stockee_corrompue_est_refusee() {
        let db = TestDatabase::new("invalid-hydration");
        let account = must_id(AccountId::new("alice"));
        let conversation = must_id(ConversationId::new("shared-conversation"));
        let mut store = open_store(&db.0, &account).unwrap();
        save_fixture_replica(&mut store, &replica_with_text("body")).unwrap();
        store
            .connection
            .execute("UPDATE replicas SET hydrations_json = 'not-json'", [])
            .unwrap();
        assert!(store.load_replica(&conversation).is_err());
    }

    #[test]
    fn le_journal_survit_a_la_reouverture() {
        let path = std::env::temp_dir().join(format!(
            "grokbot-store-journal-{}-{}.sqlite",
            std::process::id(),
            "journal"
        ));
        let _ = fs::remove_file(&path);
        let actor = AuthenticatedActor {
            account_id: must_id(AccountId::new("compte")),
            team_id: None,
            generation: AccountGeneration(1),
        };
        let conversation = must_id(ConversationId::new("conv"));
        let message = must_id(MessageId::new("msg"));
        let mut journal = Journal::new();
        journal
            .prepare(PreparedMessage {
                actor: actor.clone(),
                conversation_id: conversation.clone(),
                message_id: message.clone(),
                body: "bonjour".into(),
            })
            .unwrap();
        journal
            .dispatch(
                &actor,
                &conversation,
                &message,
                must_id(AttemptId::new("tentative")),
            )
            .unwrap();
        journal
            .observe_accept(
                &actor,
                DeliveryReceipt {
                    account_id: actor.account_id.clone(),
                    conversation_id: conversation.clone(),
                    message_id: message.clone(),
                },
            )
            .unwrap();

        let mut store = SessionStore::open(&path, &must_id(AccountId::new("compte"))).unwrap();
        save_fixture_journal(&mut store, &journal).unwrap();
        drop(store);

        let store = SessionStore::open(&path, &must_id(AccountId::new("compte"))).unwrap();
        let loaded = store.load_journal().unwrap();
        assert_eq!(
            loaded.get(&actor.account_id, &conversation, &message),
            journal.get(&actor.account_id, &conversation, &message)
        );
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn la_replique_survit_a_la_reouverture() {
        let path = std::env::temp_dir().join(format!(
            "grokbot-store-replica-{}-{}.sqlite",
            std::process::id(),
            "replica"
        ));
        let _ = fs::remove_file(&path);
        let conversation = must_id(ConversationId::new("conv"));
        let epoch = ReplicaEpoch {
            boot_id: must_id(BootId::new("boot")),
            server_generation: ServerGeneration(1),
            recovery: 0,
        };
        let mut replica = Replica::new(conversation.clone());
        replica
            .apply_snapshot(Snapshot {
                conversation_id: conversation.clone(),
                epoch: epoch.clone(),
                updates: vec![TranscriptUpdate {
                    sequence: UpdateSequence(1),
                    entry_id: must_id(TranscriptEntryId::new("e1")),
                    kind: UpdateKind::Upsert {
                        body: TranscriptBody::Inline {
                            content: EntryBody::Text("un".into()),
                        },
                    },
                }],
            })
            .unwrap();
        replica
            .apply_delta(Delta {
                conversation_id: conversation.clone(),
                epoch,
                updates: vec![TranscriptUpdate {
                    sequence: UpdateSequence(3),
                    entry_id: must_id(TranscriptEntryId::new("e3")),
                    kind: UpdateKind::Upsert {
                        body: TranscriptBody::Inline {
                            content: EntryBody::Text("trois".into()),
                        },
                    },
                }],
            })
            .unwrap();

        let mut store = SessionStore::open(&path, &must_id(AccountId::new("compte"))).unwrap();
        save_fixture_replica(&mut store, &replica).unwrap();
        drop(store);

        let store = SessionStore::open(&path, &must_id(AccountId::new("compte"))).unwrap();
        let loaded = store.load_replica(&conversation).unwrap();
        assert_eq!(loaded.cursor(), replica.cursor());
        assert_eq!(loaded.export().buffer.len(), 1);
        let _ = fs::remove_file(&path);
    }
}
