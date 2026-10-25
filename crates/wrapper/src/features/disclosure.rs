//! Privacy profile for persisted deliverable presentations. Only fingerprints survive
//! vault rotation; raw credentials are never copied into this ledger.
use crate::ApiError;
use grokbot_forms::vault::Vault;
use rusqlite::Connection;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    path::Path,
};
use unicode_normalization::UnicodeNormalization;

const MASK: &str = "[...]";
#[cfg(test)]
thread_local! { static VERIFIED_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
pub(super) fn refused() -> ApiError {
    ApiError::new(409, "Lecture indisponible.")
}

pub(super) fn migrate(conn: &Connection) -> Result<(), ApiError> {
    conn.execute_batch("CREATE TABLE IF NOT EXISTS wrapper_disclosure_patterns (length INTEGER NOT NULL, digest BLOB NOT NULL, PRIMARY KEY(length,digest)) WITHOUT ROWID")?;
    Ok(())
}

#[derive(Clone, Default, PartialEq, Eq)]
pub(super) struct Patterns(BTreeMap<usize, BTreeSet<Vec<u8>>>);
impl Patterns {
    fn current(conn: &Connection) -> Result<Self, ApiError> {
        let path = conn.path().ok_or_else(refused)?;
        let vault = Vault::open_app(&Path::new(path).with_extension("secrets.json"))
            .map_err(|_| refused())?;
        let mut result = Self::default();
        for value in vault.values_for_redaction().map_err(|_| refused())? {
            let encoded = serde_json::to_string(&value).map_err(|_| refused())?;
            for variant in [&value, value.trim(), &encoded[1..encoded.len() - 1]] {
                let normalized: String = variant.nfd().collect();
                if normalized.encode_utf16().count() >= 6 && normalized.len() > MASK.len() {
                    result
                        .0
                        .entry(normalized.len())
                        .or_default()
                        .insert(Sha256::digest(normalized.as_bytes()).to_vec());
                }
            }
        }
        Ok(result)
    }
    fn remember(&self, conn: &Connection) -> Result<Self, ApiError> {
        let mut all = Self::default();
        let mut query =
            conn.prepare_cached("SELECT length,digest FROM wrapper_disclosure_patterns")?;
        for pair in query.query_map([], |row| {
            Ok((row.get::<_, usize>(0)?, row.get::<_, Vec<u8>>(1)?))
        })? {
            let (length, digest) = pair?;
            all.0.entry(length).or_default().insert(digest);
        }
        drop(query);
        let missing: Vec<_> = self
            .0
            .iter()
            .flat_map(|(length, digests)| {
                digests.iter().filter_map(|digest| {
                    (!all
                        .0
                        .get(length)
                        .is_some_and(|known| known.contains(digest)))
                    .then_some((*length, digest))
                })
            })
            .collect();
        if !missing.is_empty() {
            let tx = conn.unchecked_transaction()?;
            for (length, digest) in missing {
                tx.execute("INSERT OR IGNORE INTO wrapper_disclosure_patterns(length,digest) VALUES (?1,?2)", rusqlite::params![length,digest])?;
                all.0.entry(length).or_default().insert(digest.clone());
            }
            tx.commit()?;
        }
        Ok(all)
    }
    pub(super) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub(super) fn ranges(&self, normalized_nfd: &str) -> Vec<Range<usize>> {
        let mut ranges: Vec<Range<usize>> = Vec::new();
        for (start, _) in normalized_nfd.char_indices() {
            for (length, digests) in &self.0 {
                let Some(end) = start.checked_add(*length).filter(|end| {
                    *end <= normalized_nfd.len() && normalized_nfd.is_char_boundary(*end)
                }) else {
                    continue;
                };
                #[cfg(test)]
                VERIFIED_BYTES.with(|count| count.set(count.get() + end - start));
                if digests
                    .contains(Sha256::digest(&normalized_nfd.as_bytes()[start..end]).as_slice())
                {
                    ranges.push(start..end);
                }
            }
        }
        ranges.sort_by_key(|range| range.start);
        let mut merged: Vec<Range<usize>> = Vec::new();
        for range in ranges {
            if let Some(last) = merged.last_mut().filter(|last| range.start <= last.end) {
                last.end = last.end.max(range.end);
            } else {
                merged.push(range);
            }
        }
        merged
    }
    pub(super) fn text(&self, text: &str) -> String {
        if self.is_empty() {
            return text.to_owned();
        }
        let mut normalized: String = text.nfd().collect();
        let mut changed = false;
        loop {
            let ranges = self.ranges(&normalized);
            if ranges.is_empty() {
                return if changed { normalized } else { text.to_owned() };
            }
            changed = true;
            let mut result = String::new();
            let mut at = 0;
            for range in ranges {
                result.push_str(&normalized[at..range.start]);
                result.push_str(MASK);
                at = range.end;
            }
            result.push_str(&normalized[at..]);
            normalized = result;
        }
    }
    pub(super) fn identity(&self, value: &Value) -> Result<(), ApiError> {
        if self.value(value)? == *value {
            Ok(())
        } else {
            Err(refused())
        }
    }
    pub(super) fn value(&self, value: &Value) -> Result<Value, ApiError> {
        Ok(match value {
            Value::String(text) => Value::String(self.text(text)),
            Value::Number(number) => {
                let original = number.to_string();
                let masked = self.text(&original);
                if masked == original {
                    value.clone()
                } else {
                    Value::String(masked)
                }
            }
            Value::Array(values) => Value::Array(
                values
                    .iter()
                    .map(|v| self.value(v))
                    .collect::<Result<_, _>>()?,
            ),
            Value::Object(values) => {
                let mut result = serde_json::Map::new();
                for (key, value) in values {
                    if self.text(key) != *key {
                        return Err(refused());
                    }
                    result.insert(key.clone(), self.value(value)?);
                }
                Value::Object(result)
            }
            _ => value.clone(),
        })
    }
}

pub(super) fn read<T>(
    conn: &Connection,
    mut render: impl FnMut(&Patterns) -> Result<T, ApiError>,
) -> Result<T, ApiError> {
    for _ in 0..3 {
        let current = Patterns::current(conn)?;
        let all = current.remember(conn)?;
        let output =
            render(&all).map_err(|error| ApiError::new(error.status, all.text(&error.message)));
        let remembered: usize = conn.query_row(
            "SELECT count(*) FROM wrapper_disclosure_patterns",
            [],
            |r| r.get(0),
        )?;
        let captured: usize = all.0.values().map(BTreeSet::len).sum();
        if Patterns::current(conn)? == current && remembered == captured {
            return output;
        }
    }
    Err(refused())
}

pub(crate) fn state(conn: &Connection, value: &Value) -> Result<Value, ApiError> {
    fn identities(patterns: &Patterns, value: &Value) -> Result<(), ApiError> {
        match value {
            Value::Array(values) => {
                for value in values {
                    identities(patterns, value)?;
                }
            }
            Value::Object(values) => {
                for (key, value) in values {
                    if matches!(
                        key.as_str(),
                        "id" | "bot_id"
                            | "conversation_id"
                            | "entry_id"
                            | "open_url"
                            | "preview_url"
                            | "source_version"
                    ) {
                        patterns.identity(value)?;
                    } else {
                        identities(patterns, value)?;
                    }
                }
            }
            _ => (),
        }
        Ok(())
    }
    read(conn, |patterns| {
        identities(patterns, value)?;
        patterns.value(value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (grokbot_test_support::TestDir, Connection, Vault) {
        let dir = grokbot_test_support::TestDir::new("disclosure-boundary");
        let path = dir.join("workspace.sqlite");
        let conn = Connection::open(&path).unwrap();
        migrate(&conn).unwrap();
        let vault = Vault::open(&path.with_extension("secrets.json")).unwrap();
        (dir, conn, vault)
    }
    #[test]
    fn a_vault_write_during_render_retries_the_whole_output() {
        let (_dir, conn, vault) = fixture();
        let mut calls = 0;
        let output = read(&conn, |patterns| {
            calls += 1;
            let value = patterns.text("synthetic-concurrent-281");
            if calls == 1 {
                vault
                    .set(
                        "RACE_TEST",
                        "Secret synthétique",
                        "synthetic-concurrent-281",
                    )
                    .unwrap();
            }
            Ok(value)
        })
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(output, MASK);
        vault.remove("RACE_TEST").unwrap();
        assert_eq!(
            read(&conn, |p| Ok(p.text("synthetic-concurrent-281"))).unwrap(),
            MASK
        );
        let stored: Vec<Vec<u8>> = conn
            .prepare("SELECT digest FROM wrapper_disclosure_patterns")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert!(stored.iter().all(
            |digest| digest.len() == 32 && !digest.windows(9).any(|part| part == b"synthetic")
        ));
    }
    #[test]
    fn a_vault_failure_during_render_refuses_the_output() {
        let (dir, conn, _vault) = fixture();
        let result = read(&conn, |_| {
            std::fs::write(
                dir.join("workspace.secrets.json"),
                b"invalid synthetic vault",
            )
            .unwrap();
            Ok("not emitted")
        });
        assert!(result.is_err());
    }
    #[test]
    fn long_private_values_do_not_hash_every_full_candidate_window() {
        let (_dir, conn, vault) = fixture();
        vault.set("HASH_COST_TEST", "Secret synthétique", &"a".repeat(1280)).unwrap();
        let patterns = Patterns::current(&conn).unwrap();
        let text = "b".repeat(8192);
        VERIFIED_BYTES.with(|count| count.set(0));
        assert_eq!(patterns.text(&text), text);
        let checked = VERIFIED_BYTES.with(std::cell::Cell::get);
        assert!(checked <= 2 * text.len(), "verified {checked} candidate bytes for {} input bytes", text.len());
        assert_eq!(patterns.text(&format!("avant{}après", "a".repeat(1280))), "avant[...]après");
    }

    #[test]
    fn ledger_lookup_cost_is_independent_of_old_run_history() {
        let (_dir, conn, vault) = fixture();
        vault
            .set("COST_TEST", "Secret synthétique", "synthetic-ledger-281")
            .unwrap();
        read(&conn, |p| Ok(p.text("synthetic-ledger-281"))).unwrap();
        conn.execute_batch("CREATE TABLE old_history(value TEXT)")
            .unwrap();
        let cost = || {
            let mut steps = 0;
            for sql in [
                "SELECT length,digest FROM wrapper_disclosure_patterns",
                "SELECT count(*) FROM wrapper_disclosure_patterns",
            ] {
                let mut statement = conn.prepare(sql).unwrap();
                {
                    let mut rows = statement.query([]).unwrap();
                    while rows.next().unwrap().is_some() {}
                }
                steps += statement.get_status(rusqlite::StatementStatus::VmStep);
            }
            steps
        };
        conn.execute_batch("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<1000) INSERT INTO old_history SELECT 'noise' FROM n").unwrap();
        let small = cost();
        conn.execute_batch("WITH RECURSIVE n(i) AS (VALUES(1) UNION ALL SELECT i+1 FROM n WHERE i<9000) INSERT INTO old_history SELECT 'noise' FROM n").unwrap();
        let large = cost();
        assert_eq!(large, small, "the ledger queries must not scan run history");
        eprintln!("VmStep disclosure ledger: 1000 old rows {small}; 10000 old rows {large}");
    }
}
