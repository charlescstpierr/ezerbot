//! Versioned, per-bot skills. The caller owns transactions; each mutation and its
//! history snapshot are nevertheless one atomic SQLite statement (via triggers).
//!
//! Name slugs and `-2`, `-3`, … collisions follow SR03 in
//! `reports/server-orchestration-residual.md` (the shipped `Lye`/`rt` helpers).
//! Human titles are retained separately from those technical lookup keys.
//! Stable IDs, optimistic versions, body digests and tombstones are local design
//! choices: CM13/CM15 in `reports/goal-context-memory-learning.md` do not establish
//! a server-side history or conflict protocol. Deletion releases the active name,
//! but preserves every version, including bodies pinned by routines.
use grokbot_engine::contracts::{Error, Result};
use rusqlite::{named_params, params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Origin {
    Upload { extension: String, name: String },
    Manual,
    Model,
}

/// UTF-8 companion text, stored in SQLite with the immutable skill revision.
/// Paths are labels, never host filesystem destinations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillFile {
    pub path: String,
    pub content: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillContent {
    /// Normalized lookup key; never use this as the human-facing title.
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub body: String,
    pub origin: Origin,
    #[serde(default)]
    pub files: Vec<SkillFile>,
}

/// User-authored UTF-16 units, excluding host-managed upload annotations.
/// All high-level write paths use the same body + companion-files budget.
pub fn content_characters(content: &SkillContent) -> usize {
    let body = if matches!(&content.origin, Origin::Upload { .. }) {
        crate::import::editable_body(&content.body)
    } else {
        content.body.clone()
    };
    content
        .files
        .iter()
        .fold(body.encode_utf16().count(), |size, file| {
            size.saturating_add(file.content.encode_utf16().count())
        })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    pub skill_id: String,
    pub bot_id: String,
    pub content: SkillContent,
    pub version: u64,
    pub digest: String,
    pub deleted: bool,
}

const COLUMNS: &str =
    "skill_id, bot_id, name, description, body, origin, version, digest, deleted, display_name, files";
// Resolve collisions inside the writing statement, under SQLite's writer lock.
// A separate availability read would race another bot run's add/rename.
const AVAILABLE_NAME: &str = "
    WITH RECURSIVE names(name, suffix) AS (
        SELECT :name, 1
        UNION ALL
        SELECT :name || '-' || (suffix + 1), suffix + 1 FROM names
        WHERE EXISTS(SELECT 1 FROM skills_library
            WHERE bot_id = :bot_id AND name = names.name AND deleted = 0
              AND skill_id <> :skill_id)
    )
    SELECT name FROM names ORDER BY suffix DESC LIMIT 1";

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS component_versions (
            component TEXT PRIMARY KEY, version INTEGER NOT NULL
        );",
    )?;
    let version: Option<i64> = conn
        .query_row(
            "SELECT version FROM component_versions WHERE component = 'skills'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    match version {
        Some(3) => return Ok(()),
        Some(2) => return migrate_files(conn),
        Some(1) => {
            migrate_display_names(conn)?;
            return migrate_files(conn);
        }
        None | Some(0) => {}
        Some(version) => {
            return Err(Error::InvalidInput(format!(
                "unsupported skills schema version: {version}"
            )))
        }
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS skills_library (
            skill_id TEXT PRIMARY KEY NOT NULL,
            bot_id TEXT NOT NULL CHECK(length(trim(bot_id)) > 0),
            name TEXT NOT NULL CHECK(length(name) > 0),
            description TEXT NOT NULL,
            body TEXT NOT NULL,
            origin TEXT NOT NULL,
            version INTEGER NOT NULL CHECK(typeof(version) = 'integer' AND version > 0),
            digest TEXT NOT NULL,
            deleted INTEGER NOT NULL CHECK(deleted IN (0, 1))
        );
        CREATE UNIQUE INDEX IF NOT EXISTS skills_active_name
            ON skills_library(bot_id, name) WHERE deleted = 0;
        CREATE TABLE IF NOT EXISTS skills_versions (
            skill_id TEXT NOT NULL,
            bot_id TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT NOT NULL,
            body TEXT NOT NULL,
            origin TEXT NOT NULL,
            version INTEGER NOT NULL,
            digest TEXT NOT NULL,
            deleted INTEGER NOT NULL,
            PRIMARY KEY(skill_id, version)
        );
        CREATE TRIGGER IF NOT EXISTS skills_snapshot_added
        AFTER INSERT ON skills_library BEGIN
            INSERT INTO skills_versions
                (skill_id, bot_id, name, description, body, origin, version, digest, deleted)
            VALUES (NEW.skill_id, NEW.bot_id, NEW.name, NEW.description, NEW.body,
                    NEW.origin, NEW.version, NEW.digest, NEW.deleted);
        END;
        CREATE TRIGGER IF NOT EXISTS skills_snapshot_updated
        AFTER UPDATE ON skills_library BEGIN
            INSERT INTO skills_versions
                (skill_id, bot_id, name, description, body, origin, version, digest, deleted)
            VALUES (NEW.skill_id, NEW.bot_id, NEW.name, NEW.description, NEW.body,
                    NEW.origin, NEW.version, NEW.digest, NEW.deleted);
        END;
        INSERT INTO component_versions(component, version) VALUES ('skills', 1)
            ON CONFLICT(component) DO UPDATE SET version = excluded.version;",
    )?;
    migrate_display_names(conn)?;
    migrate_files(conn)
}

fn migrate_display_names(conn: &Connection) -> Result<()> {
    // A savepoint also works inside the host's migration transaction.
    conn.execute_batch("SAVEPOINT skills_display_names;")?;
    let result = (|| -> Result<()> {
        conn.execute_batch(
            "ALTER TABLE skills_library ADD COLUMN display_name TEXT NOT NULL DEFAULT '';
             ALTER TABLE skills_versions ADD COLUMN display_name TEXT NOT NULL DEFAULT '';
             DROP TRIGGER skills_snapshot_added;
             DROP TRIGGER skills_snapshot_updated;",
        )?;
        for table in ["skills_library", "skills_versions"] {
            let mut statement = conn.prepare(&format!(
                "SELECT skill_id, version, name, body FROM {table}"
            ))?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, u64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
            // Stream history one body at a time; a long-lived bot can have
            // many 256K-character snapshots. The indexed key is never changed.
            for row in rows {
                let (id, version, name, body) = row?;
                let display = crate::import::legacy_display_name(&body, &name);
                conn.execute(
                    &format!(
                        "UPDATE {table} SET display_name = ?1 WHERE skill_id = ?2 AND version = ?3"
                    ),
                    params![display, id, version],
                )?;
            }
        }
        for (trigger, event) in [
            ("skills_snapshot_added", "INSERT"),
            ("skills_snapshot_updated", "UPDATE"),
        ] {
            conn.execute_batch(&format!(
                "CREATE TRIGGER {trigger} AFTER {event} ON skills_library BEGIN
                    INSERT INTO skills_versions (skill_id, bot_id, name, description, body, origin, version, digest, deleted, display_name)
                    VALUES (NEW.skill_id, NEW.bot_id, NEW.name, NEW.description, NEW.body,
                        NEW.origin, NEW.version, NEW.digest, NEW.deleted, NEW.display_name);
                 END;"
            ))?;
        }
        conn.execute(
            "UPDATE component_versions SET version = 2 WHERE component = 'skills'",
            [],
        )?;
        Ok(())
    })();
    if result.is_ok() {
        conn.execute_batch("RELEASE skills_display_names;")?;
    } else {
        conn.execute_batch("ROLLBACK TO skills_display_names; RELEASE skills_display_names;")?;
    }
    result
}

fn migrate_files(conn: &Connection) -> Result<()> {
    conn.execute_batch("SAVEPOINT skills_files;")?;
    let result = conn.execute_batch(&format!(
        "ALTER TABLE skills_library ADD COLUMN files TEXT NOT NULL DEFAULT '[]';
         ALTER TABLE skills_versions ADD COLUMN files TEXT NOT NULL DEFAULT '[]';
         DROP TRIGGER skills_snapshot_added;
         DROP TRIGGER skills_snapshot_updated;
         CREATE TRIGGER skills_snapshot_added AFTER INSERT ON skills_library BEGIN
             INSERT INTO skills_versions ({COLUMNS}) VALUES
             (NEW.skill_id, NEW.bot_id, NEW.name, NEW.description, NEW.body, NEW.origin,
              NEW.version, NEW.digest, NEW.deleted, NEW.display_name, NEW.files);
         END;
         CREATE TRIGGER skills_snapshot_updated AFTER UPDATE ON skills_library BEGIN
             INSERT INTO skills_versions ({COLUMNS}) VALUES
             (NEW.skill_id, NEW.bot_id, NEW.name, NEW.description, NEW.body, NEW.origin,
              NEW.version, NEW.digest, NEW.deleted, NEW.display_name, NEW.files);
         END;
         UPDATE component_versions SET version=3 WHERE component='skills';"
    ));
    if result.is_ok() {
        conn.execute_batch("RELEASE skills_files;")?;
    } else {
        conn.execute_batch("ROLLBACK TO skills_files; RELEASE skills_files;")?;
    }
    result.map_err(Into::into)
}

/// Lowercase lookup slug, transliterating Latin accents before collapsing punctuation.
/// An empty result is invalid for library writes; import can choose its fallback.
pub fn normalize_name(name: &str) -> String {
    let mut normalized = String::new();
    let mut separator = false;
    for character in name.trim().to_lowercase().chars() {
        // No extra dependency/lockfile churn for the French/Latin titles used
        // by this app. Ignore decomposed combining accents as well.
        let latin = match character {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' => "a",
            'ç' | 'ć' | 'č' | 'ĉ' | 'ċ' => "c",
            'ď' | 'đ' | 'ð' => "d",
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' => "e",
            'ĝ' | 'ğ' | 'ġ' | 'ģ' => "g",
            'ĥ' | 'ħ' => "h",
            'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ı' => "i",
            'ĵ' => "j",
            'ķ' => "k",
            'ĺ' | 'ļ' | 'ľ' | 'ł' => "l",
            'ñ' | 'ń' | 'ņ' | 'ň' => "n",
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' => "o",
            'ŕ' | 'ŗ' | 'ř' => "r",
            'ś' | 'ŝ' | 'ş' | 'š' | 'ș' => "s",
            'ţ' | 'ť' | 'ŧ' | 'ț' => "t",
            'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' => "u",
            'ŵ' => "w",
            'ý' | 'ÿ' | 'ŷ' => "y",
            'ź' | 'ż' | 'ž' => "z",
            'æ' => "ae",
            'œ' => "oe",
            'ß' => "ss",
            '\u{0300}'..='\u{036f}' => continue,
            _ => "",
        };
        if !latin.is_empty() || character.is_ascii_alphanumeric() {
            if separator && !normalized.is_empty() {
                normalized.push('-');
            }
            if latin.is_empty() {
                normalized.push(character);
            } else {
                normalized.push_str(latin);
            }
            separator = false;
        } else {
            separator = true;
        }
    }
    normalized
}

pub fn add(conn: &Connection, bot_id: &str, content: &SkillContent) -> Result<Skill> {
    if bot_id.trim().is_empty() {
        return Err(Error::InvalidInput("skill bot_id must not be empty".into()));
    }
    let mut bytes = [0_u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
    let skill_id = format!(
        "skill_{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
    Ok(conn.query_row(
        &format!(
            "INSERT INTO skills_library
                (skill_id, bot_id, name, description, body, origin, version, digest, deleted, display_name, files)
             VALUES (:skill_id, :bot_id, ({AVAILABLE_NAME}), :description, :body,
                     :origin, 1, :digest, 0, :display_name, :files) RETURNING {COLUMNS}"
        ),
        named_params! {
            ":skill_id": skill_id,
            ":bot_id": bot_id,
            ":name": validated_name(&content.name)?,
            ":display_name": content.display_name.trim(),
            ":description": content.description,
            ":body": content.body,
            ":origin": serde_json::to_string(&content.origin)?,
            ":files": serde_json::to_string(&content.files)?,
            ":digest": format!("{:x}", Sha256::digest(content.body.as_bytes())),
        },
        read_skill,
    )?)
}

pub fn update(
    conn: &Connection,
    skill_id: &str,
    expected_version: u64,
    content: &SkillContent,
) -> Result<Skill> {
    revise(conn, skill_id, expected_version, Some(content))
}

/// Append a tombstone revision, leaving historical versions readable by ID.
pub fn delete(conn: &Connection, skill_id: &str, expected_version: u64) -> Result<()> {
    revise(conn, skill_id, expected_version, None).map(|_| ())
}

/// Active skills only, in deterministic normalized-name order within this bot.
pub fn list(conn: &Connection, bot_id: &str) -> Result<Vec<Skill>> {
    let mut statement = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM skills_library WHERE bot_id = ?1 AND deleted = 0 ORDER BY name"
    ))?;
    let rows = statement.query_map([bot_id], read_skill)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

pub fn get_version(conn: &Connection, skill_id: &str, version: u64) -> Result<Option<Skill>> {
    let mut statement = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM skills_versions WHERE skill_id = ?1 AND version = ?2"
    ))?;
    Ok(statement
        .query_row(params![skill_id, version], read_skill)
        .optional()?)
}

/// All snapshots, oldest first, including the final tombstone if deleted.
pub fn history(conn: &Connection, skill_id: &str) -> Result<Vec<Skill>> {
    let mut statement = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM skills_versions WHERE skill_id = ?1 ORDER BY version"
    ))?;
    let rows = statement.query_map([skill_id], read_skill)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn revise(
    conn: &Connection,
    skill_id: &str,
    expected_version: u64,
    content: Option<&SkillContent>,
) -> Result<Skill> {
    let mut skill = conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM skills_library WHERE skill_id = ?1"),
            [skill_id],
            read_skill,
        )
        .optional()?
        .ok_or_else(|| Error::InvalidInput("skill not found".into()))?;
    if skill.version != expected_version {
        return Err(Error::InvalidInput(format!(
            "skill version conflict: expected {expected_version}, found {}",
            skill.version
        )));
    }
    if skill.deleted {
        return Err(Error::InvalidInput("skill has been deleted".into()));
    }
    if skill.version >= i64::MAX as u64 {
        return Err(Error::InvalidInput("skill version exhausted".into()));
    }
    if let Some(content) = content {
        skill.content = content.clone();
        skill.content.name = validated_name(&content.name)?;
        skill.digest = format!("{:x}", Sha256::digest(content.body.as_bytes()));
    } else {
        skill.deleted = true;
    }
    skill.version += 1;
    // The SQL predicate is essential: another connection may have revised the
    // skill after the read above. The history trigger runs only for a winning CAS.
    conn.query_row(
        &format!(
            "UPDATE skills_library
             SET name = CASE WHEN :deleted THEN :name ELSE ({AVAILABLE_NAME}) END,
                 display_name = :display_name, description = :description, body = :body, origin = :origin, files = :files,
                 version = :version, digest = :digest, deleted = :deleted
             WHERE skill_id = :skill_id AND version = :expected_version AND deleted = 0
             RETURNING {COLUMNS}"
        ),
        named_params! {
            ":name": skill.content.name,
            ":display_name": skill.content.display_name.trim(),
            ":bot_id": skill.bot_id,
            ":description": skill.content.description,
            ":body": skill.content.body,
            ":origin": serde_json::to_string(&skill.content.origin)?,
            ":files": serde_json::to_string(&skill.content.files)?,
            ":version": skill.version,
            ":digest": skill.digest,
            ":deleted": skill.deleted,
            ":skill_id": skill_id,
            ":expected_version": expected_version,
        },
        read_skill,
    )
    .optional()?
    .ok_or_else(|| Error::InvalidInput("skill version conflict".into()))
}

fn validated_name(name: &str) -> Result<String> {
    let name = normalize_name(name);
    if name.is_empty() {
        return Err(Error::InvalidInput(
            "skill name must contain a letter or digit".into(),
        ));
    }
    Ok(name)
}

fn read_skill(row: &Row<'_>) -> rusqlite::Result<Skill> {
    let origin: String = row.get(5)?;
    let origin = serde_json::from_str(&origin).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(5, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(Skill {
        skill_id: row.get(0)?,
        bot_id: row.get(1)?,
        content: SkillContent {
            name: row.get(2)?,
            display_name: row.get(9)?,
            description: row.get(3)?,
            body: row.get(4)?,
            origin,
            files: serde_json::from_str(&row.get::<_, String>(10)?).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    10,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
        },
        version: row.get(6)?,
        digest: row.get(7)?,
        deleted: row.get(8)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        conn
    }

    fn content(name: &str, body: &str) -> SkillContent {
        SkillContent {
            name: name.into(),
            display_name: name.trim().into(),
            description: "A useful skill".into(),
            body: body.into(),
            origin: Origin::Manual,
            files: Vec::new(),
        }
    }

    #[test]
    fn v1_migration_recovers_accented_titles_without_revising_history() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE component_versions(component TEXT PRIMARY KEY, version INTEGER NOT NULL);
             INSERT INTO component_versions VALUES ('skills', 1);
             CREATE TABLE skills_library(skill_id TEXT PRIMARY KEY, bot_id TEXT, name TEXT, description TEXT, body TEXT, origin TEXT, version INTEGER, digest TEXT, deleted INTEGER);
             CREATE TABLE skills_versions(skill_id TEXT, bot_id TEXT, name TEXT, description TEXT, body TEXT, origin TEXT, version INTEGER, digest TEXT, deleted INTEGER, PRIMARY KEY(skill_id, version));
             CREATE TRIGGER skills_snapshot_added AFTER INSERT ON skills_library BEGIN SELECT 1; END;
             CREATE TRIGGER skills_snapshot_updated AFTER UPDATE ON skills_library BEGIN SELECT 1; END;"
        ).unwrap();
        let body = "---\nname: Résumé express\norigin: upload\n---\nTexte.";
        conn.execute("INSERT INTO skills_library VALUES ('skill_old', 'bot', 'r-sum-express', '', ?1, ?2, 1, 'original-digest', 0)", params![body, serde_json::to_string(&Origin::Manual).unwrap()]).unwrap();
        conn.execute_batch("INSERT INTO skills_versions SELECT * FROM skills_library;")
            .unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        let first = list(&conn, "bot").unwrap().remove(0);
        assert_eq!(first.content.display_name, "Résumé express");
        assert_eq!(first.content.name, "r-sum-express");
        assert_eq!(first.content.body, body);
        assert_eq!(first.digest, "original-digest");
        assert_eq!(first.version, 1);
        assert_eq!(
            history(&conn, &first.skill_id).unwrap(),
            vec![first.clone()]
        );
        let mut changed = first.content.clone();
        changed.display_name = "Résumé révisé".into();
        let updated = update(&conn, &first.skill_id, 1, &changed).unwrap();
        assert_eq!(
            history(&conn, &first.skill_id).unwrap(),
            vec![first, updated]
        );
    }

    #[test]
    fn v2_migration_adds_files_without_revising_existing_snapshots() {
        let conn = database();
        let first = add(&conn, "bot", &content("Ancienne", "Texte original")).unwrap();
        // Reconstruct the v2 shape, including its history-trigger names.
        conn.execute_batch("DROP TRIGGER skills_snapshot_added;
            DROP TRIGGER skills_snapshot_updated;
            ALTER TABLE skills_library DROP COLUMN files;
            ALTER TABLE skills_versions DROP COLUMN files;
            CREATE TRIGGER skills_snapshot_added AFTER INSERT ON skills_library BEGIN SELECT 1; END;
            CREATE TRIGGER skills_snapshot_updated AFTER UPDATE ON skills_library BEGIN SELECT 1; END;
            UPDATE component_versions SET version=2 WHERE component='skills';").unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        assert_eq!(
            history(&conn, &first.skill_id).unwrap(),
            vec![first.clone()]
        );
        let mut revised = first.content.clone();
        revised.files.push(SkillFile {
            path: "exemple.md".into(),
            content: "Exemple révisé".into(),
        });
        let second = update(&conn, &first.skill_id, 1, &revised).unwrap();
        assert_eq!(
            history(&conn, &first.skill_id).unwrap(),
            vec![first, second.clone()]
        );
        delete(&conn, &second.skill_id, 2).unwrap();
        assert_eq!(
            get_version(&conn, &second.skill_id, 3)
                .unwrap()
                .unwrap()
                .content
                .files,
            revised.files
        );
    }

    #[test]
    fn stale_expected_version_is_refused() {
        let conn = database();
        let first = add(&conn, "bot-a", &content("Review", "old")).unwrap();
        let second = update(&conn, &first.skill_id, 1, &content("Review", "new")).unwrap();
        let failure = update(&conn, &first.skill_id, 1, &content("Changed", "stale"));
        assert!(
            matches!(failure, Err(Error::InvalidInput(message)) if message.contains("version conflict"))
        );
        assert!(delete(&conn, &first.skill_id, 1).is_err());
        assert_eq!(list(&conn, "bot-a").unwrap(), vec![second.clone()]);
        assert_eq!(
            history(&conn, &first.skill_id).unwrap(),
            vec![first, second]
        );
        assert!(update(&conn, "missing", 1, &content("Review", "new")).is_err());
        assert!(delete(&conn, "missing", 1).is_err());
    }

    #[test]
    fn name_collision_gets_suffix() {
        let conn = database();
        let first = add(&conn, "bot-a", &content("  Code__REVIEW! ", "a")).unwrap();
        let second = add(&conn, "bot-a", &content("Code Review", "b")).unwrap();
        let third = add(&conn, "bot-a", &content("code-review", "c")).unwrap();
        assert_eq!(first.content.name, "code-review");
        assert_eq!(second.content.name, "code-review-2");
        assert_eq!(third.content.name, "code-review-3");
        let renamed = update(&conn, &first.skill_id, 1, &content("Code Review", "d")).unwrap();
        assert_eq!(renamed.content.name, "code-review");
        let other = add(&conn, "bot-a", &content("Other", "e")).unwrap();
        assert_eq!(
            update(&conn, &other.skill_id, 1, &content("Code Review", "f"))
                .unwrap()
                .content
                .name,
            "code-review-4"
        );
        assert_eq!(
            add(&conn, "bot-b", &content("Code Review", "g"))
                .unwrap()
                .content
                .name,
            "code-review"
        );
        assert_eq!(list(&conn, "bot-a").unwrap().len(), 4);
        assert_eq!(list(&conn, "bot-b").unwrap().len(), 1);
        assert_ne!(first.skill_id, second.skill_id);
        assert!(list(&conn, "absent").unwrap().is_empty());
        assert_eq!(normalize_name("__HELLO / world. 42__"), "hello-world-42");
        assert_eq!(normalize_name("Été / 東京"), "ete");
        assert_eq!(normalize_name("Résumé express"), "resume-express");
        assert_eq!(
            normalize_name("Re\u{301}sume\u{301} express"),
            "resume-express"
        );
        assert_eq!(normalize_name("Cœur à cœur"), "coeur-a-coeur");
        assert!(add(&conn, "bot-a", &content(" --- ", "bad")).is_err());
        assert!(add(&conn, " ", &content("Valid", "bad")).is_err());
    }

    #[test]
    fn history_keeps_every_version() {
        let conn = database();
        let mut uploaded = content("Résumé", "abc");
        uploaded.origin = Origin::Upload {
            extension: "md".into(),
            name: "notes.md".into(),
        };
        let first = add(&conn, "bot-a", &uploaded).unwrap();
        assert_eq!(
            first.digest,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let mut edited = content("New name", "# Révision\nKeep spacing.\n");
        edited.origin = Origin::Model;
        edited.description = "Revised description".into();
        let second = update(&conn, &first.skill_id, 1, &edited).unwrap();
        let third = update(&conn, &first.skill_id, 2, &edited).unwrap();
        assert_eq!(first.skill_id, third.skill_id);
        assert_eq!(third.version, 3);
        assert_ne!(first.digest, second.digest);
        assert_eq!(second.digest, third.digest);
        assert_eq!(
            get_version(&conn, &first.skill_id, 1).unwrap(),
            Some(first.clone())
        );
        assert_eq!(
            get_version(&conn, &first.skill_id, 2).unwrap(),
            Some(second.clone())
        );
        assert_eq!(
            history(&conn, &first.skill_id).unwrap(),
            vec![first, second, third.clone()]
        );
        assert_eq!(list(&conn, "bot-a").unwrap(), vec![third]);
        assert_eq!(get_version(&conn, "absent", 1).unwrap(), None);
        assert!(history(&conn, "absent").unwrap().is_empty());
    }

    #[test]
    fn deleted_skill_leaves_tombstone() {
        let conn = database();
        let first = add(&conn, "bot-a", &content("Pinned", "routine body")).unwrap();
        delete(&conn, &first.skill_id, 1).unwrap();
        assert!(list(&conn, "bot-a").unwrap().is_empty());
        let tombstone = get_version(&conn, &first.skill_id, 2).unwrap().unwrap();
        assert!(tombstone.deleted);
        assert_eq!(tombstone.content, first.content);
        assert_eq!(tombstone.digest, first.digest);
        assert_eq!(
            history(&conn, &first.skill_id).unwrap(),
            vec![first.clone(), tombstone]
        );
        assert_eq!(
            get_version(&conn, &first.skill_id, 1).unwrap(),
            Some(first.clone())
        );
        assert!(update(&conn, &first.skill_id, 2, &content("Pinned", "resurrect")).is_err());
        assert!(delete(&conn, &first.skill_id, 2).is_err());
        let replacement = add(&conn, "bot-a", &content("Pinned", "replacement")).unwrap();
        assert_eq!(replacement.content.name, "pinned");
        assert_ne!(replacement.skill_id, first.skill_id);
        assert_eq!(history(&conn, &first.skill_id).unwrap().len(), 2);
    }

    #[test]
    fn migrations_preserve_other_components_and_user_version() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 37).unwrap();
        migrate(&conn).unwrap();
        let first = add(&conn, "bot-a", &content("Persisted", "body")).unwrap();
        conn.execute("INSERT INTO component_versions VALUES ('memory', 9)", [])
            .unwrap();
        migrate(&conn).unwrap();
        assert_eq!(
            conn.pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            37
        );
        assert_eq!(
            conn.query_row(
                "SELECT version FROM component_versions WHERE component = 'memory'",
                [],
                |row| row.get::<_, i64>(0)
            )
            .unwrap(),
            9
        );
        assert_eq!(get_version(&conn, &first.skill_id, 1).unwrap(), Some(first));
        conn.execute(
            "UPDATE component_versions SET version = 99 WHERE component = 'skills'",
            [],
        )
        .unwrap();
        assert!(migrate(&conn).is_err());
    }

    #[test]
    fn mutations_participate_in_caller_transaction() {
        let mut conn = database();
        let first = add(&conn, "bot-a", &content("Original", "old")).unwrap();
        assert!(conn.is_autocommit());
        let tx = conn.transaction().unwrap();
        add(&tx, "bot-a", &content("Temporary", "new")).unwrap();
        update(&tx, &first.skill_id, 1, &content("Original", "changed")).unwrap();
        delete(&tx, &first.skill_id, 2).unwrap();
        assert_eq!(history(&tx, &first.skill_id).unwrap().len(), 3);
        tx.rollback().unwrap();
        assert_eq!(list(&conn, "bot-a").unwrap(), vec![first.clone()]);
        assert_eq!(history(&conn, &first.skill_id).unwrap(), vec![first]);
    }

    #[test]
    fn concurrent_adds_allocate_distinct_suffixes() {
        use std::{
            sync::{Arc, Barrier},
            thread,
            time::Duration,
        };
        let mut random = [0_u8; 16];
        getrandom::getrandom(&mut random).unwrap();
        let path = std::env::temp_dir().join(format!(
            "skills-library-{:x}.sqlite",
            u128::from_le_bytes(random)
        ));
        let conn = Connection::open(&path).unwrap();
        migrate(&conn).unwrap();
        let barrier = Arc::new(Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                thread::spawn(move || {
                    let conn = Connection::open(path).unwrap();
                    conn.busy_timeout(Duration::from_secs(5)).unwrap();
                    barrier.wait();
                    add(&conn, "bot-a", &content("Concurrent", "body")).unwrap()
                })
            })
            .collect();
        let mut names: Vec<_> = threads
            .into_iter()
            .map(|thread| thread.join().unwrap().content.name)
            .collect();
        names.sort();
        assert_eq!(
            names,
            (1..=8)
                .map(|suffix| if suffix == 1 {
                    "concurrent".into()
                } else {
                    format!("concurrent-{suffix}")
                })
                .collect::<Vec<String>>()
        );
        assert_eq!(list(&conn, "bot-a").unwrap().len(), 8);
        drop(conn);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn snapshot_failure_rolls_back_current_revision() {
        let conn = database();
        let first = add(&conn, "bot-a", &content("Original", "old")).unwrap();
        conn.execute_batch(
            "CREATE TRIGGER reject_history BEFORE INSERT ON skills_versions
             BEGIN SELECT RAISE(ABORT, 'history unavailable'); END;",
        )
        .unwrap();
        assert!(update(&conn, &first.skill_id, 1, &content("Original", "new")).is_err());
        assert!(delete(&conn, &first.skill_id, 1).is_err());
        assert!(add(&conn, "bot-a", &content("New", "orphan")).is_err());
        assert_eq!(list(&conn, "bot-a").unwrap(), vec![first.clone()]);
        assert_eq!(history(&conn, &first.skill_id).unwrap(), vec![first]);
    }
}
