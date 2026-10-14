//! Réplique en mémoire d'une conversation : snapshots, deltas, corps manquants
//! et curseur de reprise.
//!
//! La pointe du journal et le curseur de couverture sont distincts. La pointe
//! avance à chaque mise à jour contiguë appliquée. `covered_through` s'arrête
//! avant le premier `Upsert` dont le corps bloque la couverture.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod error;
mod recovery;

use std::collections::{BTreeMap, HashMap};

use grokbot_domain::{
    ContentDigest, ConversationId, ReplicaEpoch, ResumeCursor, TranscriptBody, TranscriptEntryId,
    TranscriptUpdate, UpdateKind, UpdateSequence,
};

pub use error::ReplicaError;
pub use recovery::{RecoveryError, RecoveryGate, RecoveryRequest};

/// Nombre maximal de mises à jour retenues en attente de réordonnancement.
pub const REORDER_LIMIT: usize = 512;

/// Image complète qui remplace l'état de la réplique.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// Conversation portée par le snapshot.
    pub conversation_id: ConversationId,
    /// Epoch installée par ce snapshot.
    pub epoch: ReplicaEpoch,
    /// Mises à jour. L'ensemble des séquences doit être exactement `1..=n`.
    pub updates: Vec<TranscriptUpdate>,
}

/// Lot de mises à jour à appliquer sur l'epoch déjà installée.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Delta {
    /// Conversation portée par le delta.
    pub conversation_id: ConversationId,
    /// Epoch à laquelle le delta appartient.
    pub epoch: ReplicaEpoch,
    /// Mises à jour. Une plage contiguë est exigée.
    pub updates: Vec<TranscriptUpdate>,
}

/// Image persistable d'une réplique : epoch, journal appliqué et buffer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaImage {
    /// Conversation de cette image.
    pub conversation_id: ConversationId,
    /// Epoch installée. `None` tant qu'aucun snapshot n'a été appliqué.
    pub epoch: Option<ReplicaEpoch>,
    /// Journal appliqué, séquences `1..=n` dans l'ordre.
    pub log: Vec<TranscriptUpdate>,
    /// Mises à jour en avance sur la pointe, non encore contiguës.
    pub buffer: Vec<TranscriptUpdate>,
    /// Corps retrouvés, liés à une version précise du journal original.
    pub hydrations: Vec<BodyHydration>,
}

/// Corps retrouvé pour une mise à jour `Pending` de l'epoch de l'image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BodyHydration {
    /// Séquence originale qui attendait ce corps.
    pub sequence: UpdateSequence,
    /// Entrée portée par cette mise à jour.
    pub entry_id: TranscriptEntryId,
    /// Empreinte annoncée par la mise à jour originale.
    pub expected_digest: ContentDigest,
    /// Corps retrouvé. Le journal original reste inchangé.
    pub body: TranscriptBody,
}

/// Réplique en mémoire d'une seule conversation.
pub struct Replica {
    conversation_id: ConversationId,
    epoch: Option<ReplicaEpoch>,
    /// Journal appliqué. L'indice `0` est la séquence `1`.
    log: Vec<TranscriptUpdate>,
    entries: HashMap<TranscriptEntryId, TranscriptBody>,
    /// Mises à jour dont la séquence est en avance sur la pointe, dans la fenêtre.
    buffer: BTreeMap<u64, TranscriptUpdate>,
    hydrations: BTreeMap<u64, BodyHydration>,
}

impl Replica {
    /// Réplique vide, sans epoch ni curseur.
    pub fn new(conversation_id: ConversationId) -> Self {
        Self {
            conversation_id,
            epoch: None,
            log: Vec::new(),
            entries: HashMap::new(),
            buffer: BTreeMap::new(),
            hydrations: BTreeMap::new(),
        }
    }

    /// Installe une image complète du journal.
    ///
    /// Une nouvelle epoch remplace l'état pour resynchroniser. Dans la même
    /// epoch, le snapshot doit avancer, conserver le préfixe connu et ne pas
    /// contredire le buffer. Les hydratations et deltas plus récents survivent.
    /// Un snapshot vide n'est accepté qu'en installant une nouvelle epoch.
    pub fn apply_snapshot(&mut self, snapshot: Snapshot) -> Result<ResumeCursor, ReplicaError> {
        if snapshot.conversation_id != self.conversation_id {
            return Err(ReplicaError::WrongConversation);
        }
        let ordered = order_snapshot(snapshot.updates)?;
        if self.epoch.as_ref() == Some(&snapshot.epoch) {
            if ordered.len() <= self.log.len() {
                return Err(ReplicaError::StaleSnapshot);
            }
            for update in &ordered {
                let known = self
                    .log_at(sequence_value(update))
                    .or_else(|| self.buffer.get(&sequence_value(update)));
                if let Some(known) = known {
                    if !same_payload(known, update) {
                        return Err(ReplicaError::Conflict);
                    }
                }
            }
            for update in ordered.into_iter().skip(self.log.len()) {
                self.buffer.remove(&sequence_value(&update));
                self.push_applied(update);
            }
            self.drain_buffer();
            return Ok(self.installed_cursor());
        }
        self.epoch = Some(snapshot.epoch);
        self.log.clear();
        self.entries.clear();
        self.buffer.clear();
        self.hydrations.clear();
        for update in ordered {
            self.push_applied(update);
        }
        Ok(self.installed_cursor())
    }

    /// Applique un delta, ou le retient s'il est en avance dans la fenêtre.
    ///
    /// Un début strictement supérieur à la prochaine séquence est un trou.
    /// Il est bufferisé seulement si `last <= expected + REORDER_LIMIT` et si
    /// l'union des séquences du delta et du buffer tient dans `REORDER_LIMIT`.
    /// Au-delà de la fenêtre, c'est
    /// un [`ReplicaError::Gap`]. Au-delà de 512 mises à jour en attente, c'est
    /// un [`ReplicaError::ReorderOverflow`] : le buffer est vidé, le journal non.
    pub fn apply_delta(&mut self, delta: Delta) -> Result<ResumeCursor, ReplicaError> {
        if delta.conversation_id != self.conversation_id {
            return Err(ReplicaError::WrongConversation);
        }
        match &self.epoch {
            Some(epoch) if epoch == &delta.epoch => {}
            _ => return Err(ReplicaError::ForeignEpoch),
        }
        if delta.updates.is_empty() {
            return Ok(self.installed_cursor());
        }

        let indexed = index_contiguous(delta.updates)?;
        let Some((&first, _)) = indexed.first_key_value() else {
            return Ok(self.installed_cursor());
        };
        let Some((&last, _)) = indexed.last_key_value() else {
            return Ok(self.installed_cursor());
        };
        let expected = self.expected_sequence();

        if first > expected {
            self.buffer_ahead(expected, first, indexed)
        } else if last < expected {
            self.replay_historical(first, expected, &indexed)
        } else {
            self.apply_overlapping(expected, indexed)
        }
    }

    /// Retrouve le corps d'une version précise sans modifier son événement original.
    ///
    /// L'epoch, la séquence et l'entrée doivent correspondre au `Pending` original.
    /// Un `Blob` doit annoncer son empreinte attendue. Aucun hash de contenu en
    /// ligne n'est calculé ici : sa vérification reste à la frontière de fetch.
    pub fn fill_body(
        &mut self,
        epoch: &ReplicaEpoch,
        sequence: UpdateSequence,
        entry_id: &TranscriptEntryId,
        body: TranscriptBody,
    ) -> Result<ResumeCursor, ReplicaError> {
        if self.epoch.as_ref() != Some(epoch) {
            return Err(ReplicaError::ForeignEpoch);
        }
        let expected_digest = self.pending_digest(sequence, entry_id)?;
        if body.blocks_coverage() {
            return Err(ReplicaError::StillPending);
        }
        if let TranscriptBody::Blob { digest, .. } = &body {
            if digest != &expected_digest {
                return Err(ReplicaError::DigestMismatch);
            }
        }
        if let Some(existing) = self.hydrations.get(&sequence.0) {
            return if existing.body == body {
                Ok(self.installed_cursor())
            } else {
                Err(ReplicaError::Conflict)
            };
        }
        self.hydrations.insert(
            sequence.0,
            BodyHydration {
                sequence,
                entry_id: entry_id.clone(),
                expected_digest,
                body,
            },
        );
        self.rebuild_entries();
        Ok(self.installed_cursor())
    }

    /// Curseur courant. `None` tant qu'aucun snapshot n'a installé d'epoch.
    ///
    /// `covered_through` vaut `0` quand aucune séquence n'est couverte.
    /// `complete` est faux si un corps bloque la couverture ou si le buffer
    /// n'est pas vide.
    pub fn cursor(&self) -> Option<ResumeCursor> {
        let epoch = self.epoch.clone()?;
        let (covered_through, complete) = self.coverage();
        Some(ResumeCursor {
            epoch,
            covered_through,
            complete,
        })
    }

    /// Corps matérialisé de l'entrée, après application du journal.
    ///
    /// Une entrée seulement bufferisée n'est pas visible. Un `Pending` appliqué
    /// est conservé. Un `Delete` retire l'entrée.
    pub fn entry(&self, id: &TranscriptEntryId) -> Option<&TranscriptBody> {
        self.entries.get(id)
    }

    /// Copie l'état appliqué et le buffer. Les entrées se reconstruisent au chargement.
    pub fn export(&self) -> ReplicaImage {
        ReplicaImage {
            conversation_id: self.conversation_id.clone(),
            epoch: self.epoch.clone(),
            log: self.log.clone(),
            buffer: self.buffer.values().cloned().collect(),
            hydrations: self.hydrations.values().cloned().collect(),
        }
    }

    /// Reconstruit une réplique depuis une image. Le journal doit être `1..=n`.
    pub fn restore(image: ReplicaImage) -> Result<Self, ReplicaError> {
        let mut replica = Self::new(image.conversation_id);
        if image.epoch.is_none() {
            if image.log.is_empty() && image.buffer.is_empty() && image.hydrations.is_empty() {
                return Ok(replica);
            }
            return Err(ReplicaError::MalformedSnapshot);
        }
        replica.epoch = image.epoch;
        for (index, update) in image.log.into_iter().enumerate() {
            if sequence_value(&update) != (index as u64) + 1 {
                return Err(ReplicaError::MalformedSnapshot);
            }
            replica.push_applied(update);
        }
        if image.buffer.len() > REORDER_LIMIT {
            return Err(ReplicaError::MalformedSnapshot);
        }
        for update in image.buffer {
            let sequence = sequence_value(&update);
            if sequence <= replica.expected_sequence()
                || !within_reorder_window(sequence, replica.expected_sequence())
                || replica.buffer.contains_key(&sequence)
            {
                return Err(ReplicaError::MalformedSnapshot);
            }
            replica.buffer.insert(sequence, update);
        }
        for hydration in image.hydrations {
            if replica.hydrations.contains_key(&hydration.sequence.0)
                || replica.pending_digest(hydration.sequence, &hydration.entry_id)?
                    != hydration.expected_digest
            {
                return Err(ReplicaError::MalformedSnapshot);
            }
            let epoch = replica.epoch.clone().expect("epoch contrôlée plus haut");
            replica.fill_body(
                &epoch,
                hydration.sequence,
                &hydration.entry_id,
                hydration.body,
            )?;
        }
        Ok(replica)
    }

    fn expected_sequence(&self) -> u64 {
        (self.log.len() as u64).saturating_add(1)
    }

    fn coverage(&self) -> (UpdateSequence, bool) {
        for (index, update) in self.log.iter().enumerate() {
            if let UpdateKind::Upsert { body } = &update.kind {
                if body.blocks_coverage() && !self.hydrations.contains_key(&update.sequence.0) {
                    return (UpdateSequence(index as u64), false);
                }
            }
        }
        (
            UpdateSequence(self.log.len() as u64),
            self.buffer.is_empty(),
        )
    }

    fn installed_cursor(&self) -> ResumeCursor {
        self.cursor()
            .expect("l'epoch est installée avant de produire un curseur")
    }

    fn log_at(&self, sequence: u64) -> Option<&TranscriptUpdate> {
        let index = sequence.checked_sub(1)?;
        self.log.get(index as usize)
    }

    fn pending_digest(
        &self,
        sequence: UpdateSequence,
        entry_id: &TranscriptEntryId,
    ) -> Result<ContentDigest, ReplicaError> {
        let update = self.log_at(sequence.0).ok_or(ReplicaError::NotFound)?;
        if &update.entry_id != entry_id {
            return Err(ReplicaError::NotFound);
        }
        match &update.kind {
            UpdateKind::Upsert {
                body: TranscriptBody::Pending { expected_digest },
            } => Ok(*expected_digest),
            _ => Err(ReplicaError::NotPending),
        }
    }

    fn push_applied(&mut self, update: TranscriptUpdate) {
        debug_assert_eq!(sequence_value(&update), self.expected_sequence());
        self.project(&update);
        self.log.push(update);
    }

    fn project(&mut self, update: &TranscriptUpdate) {
        match &update.kind {
            UpdateKind::Upsert { body } => {
                self.entries.insert(update.entry_id.clone(), body.clone());
            }
            UpdateKind::Delete => {
                self.entries.remove(&update.entry_id);
            }
        }
    }

    fn rebuild_entries(&mut self) {
        let mut entries = HashMap::new();
        for update in &self.log {
            match &update.kind {
                UpdateKind::Upsert { body } => {
                    let resolved = self
                        .hydrations
                        .get(&update.sequence.0)
                        .map_or(body, |hydration| &hydration.body);
                    entries.insert(update.entry_id.clone(), resolved.clone());
                }
                UpdateKind::Delete => {
                    entries.remove(&update.entry_id);
                }
            }
        }
        self.entries = entries;
    }

    fn replay_historical(
        &self,
        first: u64,
        expected: u64,
        indexed: &BTreeMap<u64, TranscriptUpdate>,
    ) -> Result<ResumeCursor, ReplicaError> {
        let exact = indexed.iter().all(|(sequence, update)| {
            matches!(
                self.log_at(*sequence),
                Some(stored) if same_payload(stored, update)
            )
        });
        if exact {
            return Ok(self.installed_cursor());
        }
        if is_cursor_too_old(first, expected) {
            return Err(ReplicaError::CursorTooOld);
        }
        Err(ReplicaError::Conflict)
    }

    fn apply_overlapping(
        &mut self,
        expected: u64,
        indexed: BTreeMap<u64, TranscriptUpdate>,
    ) -> Result<ResumeCursor, ReplicaError> {
        for (sequence, update) in &indexed {
            if *sequence < expected {
                match self.log_at(*sequence) {
                    Some(stored) if same_payload(stored, update) => {}
                    _ => return Err(ReplicaError::Conflict),
                }
            } else if let Some(buffered) = self.buffer.get(sequence) {
                if !same_payload(buffered, update) {
                    return Err(ReplicaError::Conflict);
                }
            }
        }

        let fresh: Vec<TranscriptUpdate> = indexed
            .into_iter()
            .filter(|(sequence, _)| *sequence >= expected)
            .map(|(_, update)| update)
            .collect();
        for update in fresh {
            let sequence = sequence_value(&update);
            self.buffer.remove(&sequence);
            self.push_applied(update);
        }
        self.drain_buffer();
        Ok(self.installed_cursor())
    }

    /// Un delta dont le premier numéro est `> expected` est un trou.
    /// On ne le bufferise que dans la fenêtre, et seulement si le volume tient.
    fn buffer_ahead(
        &mut self,
        expected: u64,
        first: u64,
        indexed: BTreeMap<u64, TranscriptUpdate>,
    ) -> Result<ResumeCursor, ReplicaError> {
        if !within_reorder_window(first, expected) {
            return Err(ReplicaError::Gap {
                expected,
                got: first,
            });
        }
        for (sequence, update) in &indexed {
            if let Some(existing) = self.buffer.get(sequence) {
                if !same_payload(existing, update) {
                    return Err(ReplicaError::Conflict);
                }
            }
        }
        let novel = indexed
            .keys()
            .filter(|sequence| !self.buffer.contains_key(sequence))
            .count();
        if novel == 0 {
            return Ok(self.installed_cursor());
        }
        if self.buffer.len().saturating_add(novel) > REORDER_LIMIT {
            self.buffer.clear();
            return Err(ReplicaError::ReorderOverflow);
        }
        if let Some((&last, _)) = indexed.last_key_value() {
            if !within_reorder_window(last, expected) {
                return Err(ReplicaError::Gap {
                    expected,
                    got: last,
                });
            }
        }
        for (sequence, update) in indexed {
            self.buffer.insert(sequence, update);
        }
        Ok(self.installed_cursor())
    }

    fn drain_buffer(&mut self) {
        loop {
            let next = self.expected_sequence();
            let Some(update) = self.buffer.remove(&next) else {
                break;
            };
            self.push_applied(update);
        }
    }
}

fn sequence_value(update: &TranscriptUpdate) -> u64 {
    update.sequence.0
}

fn same_payload(left: &TranscriptUpdate, right: &TranscriptUpdate) -> bool {
    left.entry_id == right.entry_id && left.kind == right.kind
}

const fn within_reorder_window(first: u64, expected: u64) -> bool {
    match expected.checked_add(REORDER_LIMIT as u64) {
        Some(limit) => first <= limit,
        None => true,
    }
}

const fn is_cursor_too_old(got: u64, expected: u64) -> bool {
    match got.checked_add(REORDER_LIMIT as u64) {
        Some(furthest) => furthest < expected,
        None => false,
    }
}

/// L'ensemble des séquences doit être exactement `1..=n`, dans n'importe quel ordre.
fn order_snapshot(updates: Vec<TranscriptUpdate>) -> Result<Vec<TranscriptUpdate>, ReplicaError> {
    let mut slots = vec![None; updates.len()];
    let limit = updates.len() as u64;
    for update in updates {
        let sequence = sequence_value(&update);
        if sequence == 0 || sequence > limit {
            return Err(ReplicaError::MalformedSnapshot);
        }
        let index = (sequence - 1) as usize;
        if slots[index].is_some() {
            return Err(ReplicaError::MalformedSnapshot);
        }
        slots[index] = Some(update);
    }
    let mut ordered = Vec::with_capacity(slots.len());
    for slot in slots {
        match slot {
            Some(update) => ordered.push(update),
            None => return Err(ReplicaError::MalformedSnapshot),
        }
    }
    Ok(ordered)
}

/// Indexe une plage contiguë. Deux fois la même séquence avec le même contenu
/// compte pour une. Un contenu différent ou un trou interne est un conflit.
fn index_contiguous(
    updates: Vec<TranscriptUpdate>,
) -> Result<BTreeMap<u64, TranscriptUpdate>, ReplicaError> {
    let mut indexed = BTreeMap::new();
    for update in updates {
        let sequence = sequence_value(&update);
        if let Some(existing) = indexed.get(&sequence) {
            if !same_payload(existing, &update) {
                return Err(ReplicaError::Conflict);
            }
            continue;
        }
        indexed.insert(sequence, update);
    }
    let Some((&first, _)) = indexed.first_key_value() else {
        return Ok(indexed);
    };
    let Some((&last, _)) = indexed.last_key_value() else {
        return Ok(indexed);
    };
    let span = last
        .checked_sub(first)
        .and_then(|distance| distance.checked_add(1));
    if span != Some(indexed.len() as u64) {
        return Err(ReplicaError::Conflict);
    }
    Ok(indexed)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod recovery_tests;
