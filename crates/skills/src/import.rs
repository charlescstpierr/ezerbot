//! Text uploads, following `reports/server-orchestration-residual.md` SR03/SR04
//! (REA parent ev_a8b822a62831e30bd01eb4c4488b36f0bd50cb8ccc41d83e2dcff4e00cf82a4d).
//! The recovered importer (anchors A11–A17) checks the first 8192 bytes for NUL,
//! validates UTF-8, counts UTF-16 units, and annotates the preserved document.
//! Names follow the plan's frontmatter → heading → SKILL.md directory → filename
//! precedence. Digest deduplication, named updates at capacity, and exact per-file
//! reports are local choices (Grok applies its remaining-slot cap to updates too).
//! `convert` is pure. Writes are serial and failures do not roll back earlier
//! successes (SR04, A19). The caller owns transactions and HTTP batch limits.
use crate::library::{self, Origin, Skill, SkillContent};
use grokbot_engine::contracts::{Error, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

/// JavaScript string length (UTF-16 code units), not UTF-8 bytes or scalar values.
pub const MAX_SKILL_CHARACTERS: usize = 256_000;
pub const MAX_SKILLS_PER_BOT: usize = 250;
pub const IMPORT_REQUEST_BYTES: usize = 16 * 1024 * 1024;
pub const IMPORT_CLIENT_BATCH_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ImportFile {
    pub path: String,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportedSkill {
    pub content: SkillContent,
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ImportStatus {
    Written { skill_id: String },
    Skipped { reason: String },
    Duplicate { skill_id: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportItem {
    pub file_name: String,
    pub status: ImportStatus,
}

pub fn convert(file: &ImportFile) -> Result<ImportedSkill> {
    convert_parts(file).map(|(imported, _)| imported)
}

fn convert_parts(file: &ImportFile) -> Result<(ImportedSkill, Option<String>)> {
    let path = file.path.replace('\\', "/");
    let parts: Vec<_> = path
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    let file_name = parts.last().copied().ok_or_else(|| invalid("empty-path"))?;
    let dot = file_name
        .rfind('.')
        .filter(|&i| i > 0)
        .ok_or_else(|| invalid("unsupported"))?;
    let extension = &file_name[dot..];
    if !matches!(
        extension.to_ascii_lowercase().as_str(),
        ".txt" | ".md" | ".markdown" | ".csv" | ".json" | ".yaml" | ".yml"
    ) || file.bytes.iter().take(8192).any(|&b| b == 0)
    {
        return Err(invalid("unsupported"));
    }
    let text = std::str::from_utf8(&file.bytes).map_err(|_| invalid("unsupported"))?;
    // TextDecoder("utf-8") consumes a leading BOM.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.trim_matches(js_space).is_empty() {
        return Err(invalid("empty"));
    }
    if text.encode_utf16().count() > MAX_SKILL_CHARACTERS {
        return Err(invalid("too-large"));
    }

    let (header, body) = frontmatter(text).unwrap_or(("", text));
    let front_name = field(header, "name").trim_matches(js_space).to_owned();
    // A human title must not depend on whether it has an ASCII lookup key.
    let has_name = !front_name.is_empty();
    let fallback = if file_name == "SKILL.md" && parts.len() > 1 {
        parts[parts.len() - 2]
    } else {
        &file_name[..dot]
    };
    let title = if has_name {
        front_name.clone()
    } else {
        heading(body).unwrap_or(fallback).replace(['\r', '\n'], " ")
    };
    let mut name = normalize(&title);
    if name.is_empty() {
        name = "skill".into();
    }
    let upload_name = if has_name || file_name == "SKILL.md" {
        ""
    } else {
        file_name
    };
    let body = annotate(text, &title, has_name, extension, upload_name);
    let digest = format!("{:x}", Sha256::digest(body.as_bytes()));
    Ok((
        ImportedSkill {
            content: SkillContent {
                name,
                display_name: if title.trim().is_empty() {
                    "skill".into()
                } else {
                    title.trim().into()
                },
                description: field(header, "description"),
                body,
                files: Vec::new(),
                origin: Origin::Upload {
                    extension: extension.into(),
                    name: file_name.into(),
                },
            },
            digest,
        },
        has_name.then_some(front_name),
    ))
}

/// A listing failure aborts before writing. Conversion, capacity and ordinary write
/// errors are reported per file. Matching named frontmatter updates the existing
/// version once per batch, even at capacity (SR03 `ut` / `plannedInDrop`).
///
/// Concurrent callers MUST hold a caller-owned `BEGIN IMMEDIATE` transaction
/// (`with_tx`) across this call: capacity and digest checks are read-then-write.
/// We never begin/commit/roll back. If SQLite loses that transaction, abort with
/// an error rather than reporting rolled-back successes or writing in autocommit.
pub fn import_batch(
    conn: &Connection,
    bot_id: &str,
    files: &[ImportFile],
) -> Result<Vec<ImportItem>> {
    let known = library::list(conn, bot_id)?;
    persist_files(conn, files, known, |existing, content| match existing {
        Some(skill) => library::update(conn, &skill.skill_id, skill.version, content),
        None => library::add(conn, bot_id, content),
    })
}

fn persist_files(
    conn: &Connection,
    files: &[ImportFile],
    known: Vec<Skill>,
    mut write: impl FnMut(Option<&Skill>, &SkillContent) -> Result<Skill>,
) -> Result<Vec<ImportItem>> {
    let in_tx = !conn.is_autocommit();
    let mut lost = false;
    let report = import_files(files, known, |existing, content| {
        if lost {
            return Err(invalid("import transaction was rolled back by SQLite"));
        }
        let result = write(existing, content);
        if in_tx && conn.is_autocommit() {
            lost = true;
        }
        result
    });
    if lost {
        Err(invalid("import transaction was rolled back by SQLite"))
    } else {
        Ok(report)
    }
}

fn import_files(
    files: &[ImportFile],
    mut known: Vec<Skill>,
    mut write: impl FnMut(Option<&Skill>, &SkillContent) -> Result<Skill>,
) -> Vec<ImportItem> {
    known.retain(|skill| !skill.deleted);
    let mut planned = HashSet::new();
    files
        .iter()
        .map(|file| {
            let status = match convert_parts(file) {
                Err(error) => ImportStatus::Skipped {
                    reason: error.to_string(),
                },
                Ok((imported, front_name)) => {
                    if let Some(skill) = known.iter().find(|skill| skill.digest == imported.digest)
                    {
                        planned.insert(skill.content.name.clone());
                        ImportStatus::Duplicate {
                            skill_id: skill.skill_id.clone(),
                        }
                    } else {
                        let reusable = front_name.as_ref().and_then(|front| {
                            known.iter().position(|skill| {
                                (skill.content.name == imported.content.name
                                    || (skill.content.name == legacy_slug(front)
                                        && skill.content.display_name
                                            == imported.content.display_name))
                                    && !planned.contains(&skill.content.name)
                                    && field(
                                        frontmatter(&skill.content.body).map_or("", |(h, _)| h),
                                        "name",
                                    )
                                    .trim_matches(js_space)
                                        == front
                            })
                        });
                        // Only final names claim reuse; suffixes and failed/refused
                        // writes must not reserve their requested base name.
                        if reusable.is_none() && known.len() >= MAX_SKILLS_PER_BOT {
                            ImportStatus::Skipped {
                                reason: "too-many-skills".into(),
                            }
                        } else {
                            let existing = reusable.map(|index| &known[index]);
                            let mut content = imported.content;
                            if let Some(existing) = existing {
                                // Pre-fix uploads retain their lookup key and stable ID.
                                content.name = existing.content.name.clone();
                            }
                            match write(existing, &content) {
                                Ok(skill) => {
                                    planned.insert(skill.content.name.clone());
                                    let status = ImportStatus::Written {
                                        skill_id: skill.skill_id.clone(),
                                    };
                                    match reusable {
                                        Some(index) => known[index] = skill,
                                        None => known.push(skill),
                                    }
                                    status
                                }
                                Err(error) => ImportStatus::Skipped {
                                    reason: format!("write-failed: {error}"),
                                },
                            }
                        }
                    }
                }
            };
            ImportItem {
                file_name: file.path.clone(),
                status,
            }
        })
        .collect()
}

fn invalid(reason: &str) -> Error {
    Error::InvalidInput(reason.into())
}

fn js_space(c: char) -> bool {
    c == '\u{feff}' || (c.is_whitespace() && c != '\u{85}')
}

// These intentionally parse only top-level single-line fields, like ny/rk in the
// recovered client (index.eager-app-BfVhIMYf.js:49261/49359), not arbitrary YAML.
fn frontmatter(text: &str) -> Option<(&str, &str)> {
    let rest = text
        .strip_prefix("---\r\n")
        .or_else(|| text.strip_prefix("---\n"))?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let marker = line
            .strip_suffix("\r\n")
            .or_else(|| line.strip_suffix('\n'))
            .unwrap_or(line);
        if offset > 0 && marker == "---" {
            let header = &rest[..offset];
            let header = header.strip_suffix('\n').unwrap_or(header);
            return Some((
                header.strip_suffix('\r').unwrap_or(header),
                &rest[offset + line.len()..],
            ));
        }
        offset += line.len();
    }
    None
}

fn field(header: &str, key: &str) -> String {
    let prefix = format!("{key}:");
    let value = header
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .unwrap_or("")
        .trim_matches(js_space);
    // Host-written multiline metadata is a JSON-quoted single-line YAML scalar.
    if value.starts_with('"') {
        if let Ok(decoded) = serde_json::from_str::<String>(value) {
            return decoded;
        }
    }
    for quote in ['\'', '"'] {
        if let Some(value) = value
            .strip_prefix(quote)
            .and_then(|v| v.strip_suffix(quote))
        {
            return value.into();
        }
    }
    value.into()
}

fn heading(body: &str) -> Option<&str> {
    let line = body
        .lines()
        .find(|line| !line.trim_matches(js_space).is_empty())?;
    let title = line.strip_prefix('#')?;
    if !title.starts_with([' ', '\t']) || title.contains(['\r', '\u{2028}', '\u{2029}']) {
        return None;
    }
    let title = title.trim_matches([' ', '\t']);
    let without_hashes = title.trim_end_matches('#');
    let title = if without_hashes.ends_with([' ', '\t']) {
        without_hashes.trim_end_matches([' ', '\t'])
    } else {
        title
    };
    (!title.is_empty()).then_some(title)
}

fn normalize(title: &str) -> String {
    library::normalize_name(title)
}

/// Recover the title of a pre-display-name upload without changing its stable
/// lookup key, digest, version or body. Don't undo a later manual rename.
pub(crate) fn legacy_display_name(body: &str, name: &str) -> String {
    let title = field(frontmatter(body).map_or("", |(header, _)| header), "name");
    let legacy = legacy_slug(&title);
    if !legacy.is_empty()
        && (name == legacy
            || name
                .strip_prefix(&format!("{legacy}-"))
                .is_some_and(|suffix| {
                    !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit())
                }))
    {
        title
    } else {
        name.into()
    }
}

fn legacy_slug(title: &str) -> String {
    title
        .to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

const MANAGED_KEYS: [&str; 5] = [
    "name:",
    "description:",
    "origin:",
    "upload-extension:",
    "upload-name:",
];
/// Only standalone, single-line scalar metadata belongs to the host. Complex
/// YAML remains authored content, including its key and all continuation lines.
fn host_lines(header: &str) -> Vec<(&str, bool)> {
    let lines: Vec<_> = header.lines().collect();
    let mut owned: Vec<_> = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let scalar = MANAGED_KEYS.iter().any(|key| {
                line.strip_prefix(key).is_some_and(|value| {
                    let value = value.trim();
                    !value.is_empty() && !value.starts_with(['|', '>', '[', '{'])
                })
            });
            // Blank/comment lines can separate a plain scalar from its continuation.
            // Non-candidate lines need no lookahead; this also keeps headers
            // with long blank/comment runs linear rather than quadratic.
            let continued = scalar
                && lines[index + 1..]
                    .iter()
                    .find(|next| !next.trim().is_empty() && !next.trim_start().starts_with('#'))
                    .is_some_and(|next| next.starts_with([' ', '\t']));
            (*line, scalar && !continued)
        })
        .collect();
    // If a key also occurs as authored YAML, preserve every occurrence rather
    // than silently choosing one value from an ambiguous author document.
    for prefix in ["name:", "description:"] {
        if owned
            .iter()
            .any(|(line, host)| !host && line.starts_with(prefix))
        {
            for (line, host) in &mut owned {
                if line.starts_with(prefix) {
                    *host = false;
                }
            }
        }
    }
    owned
}

/// Only host-owned top-level, single-line fields are hidden. User frontmatter,
/// fenced examples and similarly named fields in the Markdown body are intact.
pub fn editable_body(text: &str) -> String {
    let Some((header, body)) = frontmatter(text) else {
        return text.into();
    };
    let header = host_lines(header);
    if !header.iter().any(|(_, host)| *host) {
        return text.into();
    }
    let lines: Vec<_> = header
        .into_iter()
        .filter_map(|(line, host)| (!host).then_some(line))
        .collect();
    if lines.iter().all(|line| line.trim().is_empty()) {
        // annotate adds one blank separator before a headerless upload's text.
        let rest = body.strip_prefix('\n').unwrap_or(body);
        // Keep the boundary before rule-like text (even an unfinished marker at
        // EOF), so a later body edit cannot turn it into the hidden header.
        if rest.starts_with("---") {
            format!("\n{rest}")
        } else {
            rest.into()
        }
    } else {
        format!("---\n{}\n---\n{body}", lines.join("\n"))
    }
}

/// A single-line metadata value typed in an editable upload header. Authored
/// blocks, continuations and ambiguous duplicate keys are not field edits.
pub fn typed_metadata(edited: &str, key: &str) -> Option<String> {
    let (header, _) = frontmatter(edited)?;
    let prefix = format!("{key}:");
    host_lines(header)
        .into_iter()
        .find(|(line, host)| *host && line.starts_with(prefix.as_str()))
        .map(|(line, _)| field(line, key))
}

/// Reattach upload provenance and synchronize scalar metadata with the editor
/// fields. New typed scalars become fields; authored replacements stay intact.
/// Never parse Markdown-body YAML.
pub fn preserve_managed_frontmatter(
    original: &str,
    edited: &str,
    name: &str,
    description: &str,
) -> String {
    let original_header = frontmatter(original).map_or("", |(header, _)| header);
    // A complex replacement belongs to the author. Do not replay the old hidden
    // scalar beside it, creating a duplicate key the author never entered.
    let (edited_header, body) = frontmatter(edited).unwrap_or(("", edited));
    let edited_lines = host_lines(edited_header);
    let authored: Vec<_> = ["name:", "description:"]
        .into_iter()
        .filter(|prefix| {
            edited_lines
                .iter()
                .any(|(line, host)| !host && line.starts_with(prefix))
        })
        .collect();
    let mut metadata: Vec<_> = host_lines(original_header)
        .into_iter()
        .filter_map(|(line, host)| {
            (host && !authored.iter().any(|prefix| line.starts_with(prefix))).then_some(line)
        })
        .map(|line| {
            if line.starts_with("name:") {
                metadata_line("name", name)
            } else if line.starts_with("description:") {
                metadata_line("description", description)
            } else {
                line.into()
            }
        })
        .collect();
    // The wrapper adopts typed scalar values when the dedicated fields are
    // unchanged. Re-emit newly introduced fields too, not just original keys.
    for (key, value) in [("name", name), ("description", description)] {
        let prefix = format!("{key}:");
        if typed_metadata(edited, key).is_some()
            && !metadata
                .iter()
                .any(|line| line.starts_with(prefix.as_str()))
        {
            metadata.push(metadata_line(key, value));
        }
    }
    if metadata.is_empty() {
        return edited.into();
    }
    // Preserve the raw Markdown body's separator and line endings.
    let header = edited_lines
        .into_iter()
        .filter_map(|(line, host)| (!host).then_some(line))
        .collect::<Vec<_>>()
        .join("\n");
    let separator = if header.is_empty() { "" } else { "\n" };
    format!(
        "---\n{header}{separator}{}\n---\n{body}",
        metadata.join("\n")
    )
}

fn metadata_line(key: &str, value: &str) -> String {
    // Keep ordinary metadata readable, but never let a multiline description or
    // YAML delimiter create extra header fields (or close the header).
    let quote = value.is_empty()
        || value.chars().any(char::is_control)
        || value.trim() != value
        || value.starts_with([
            '-', '?', ':', ',', '[', ']', '{', '}', '#', '&', '*', '!', '|', '>', '\'', '"', '%',
            '@', '`',
        ])
        || value.contains(": ")
        || value.contains(" #");
    let value = if quote {
        serde_json::to_string(value).expect("string serialization")
    } else {
        value.into()
    };
    format!("{key}: {value}")
}

fn annotate(text: &str, title: &str, has_name: bool, extension: &str, file_name: &str) -> String {
    let parsed = frontmatter(text);
    let (header, body) = parsed.unwrap_or(("", text));
    // split (not lines) preserves the empty header/last line, like JS /\r?\n/.
    let mut lines: Vec<String> = if parsed.is_some() {
        header
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line))
            .filter(|line| {
                !["origin:", "upload-extension:", "upload-name:"]
                    .iter()
                    .any(|key| line.starts_with(key))
            })
            .map(|line| {
                if !has_name && line.starts_with("name:") {
                    format!("name: {title}")
                } else {
                    line.into()
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    if !has_name && !header.lines().any(|line| line.starts_with("name:")) {
        lines.insert(0, format!("name: {title}"));
    }
    lines.push("origin: upload".into());
    lines.push(format!("upload-extension: {extension}"));
    let file_name = file_name.replace(['\r', '\n'], " ");
    let file_name = file_name.trim_matches(js_space);
    if !file_name.is_empty() {
        lines.push(format!("upload-name: '{}'", file_name.replace('\'', "''")));
    }
    let body = if parsed.is_none() {
        body.trim_start_matches('\n')
    } else {
        body
    };
    let separator = if body.is_empty() || body.starts_with('\n') {
        ""
    } else {
        "\n"
    };
    format!("---\n{}\n---\n{separator}{body}", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, text: &str) -> ImportFile {
        ImportFile {
            path: path.into(),
            bytes: text.as_bytes().to_vec(),
        }
    }

    fn skill(id: usize, content: &SkillContent) -> Skill {
        Skill {
            skill_id: format!("skill_{id}"),
            bot_id: "bot".into(),
            content: content.clone(),
            version: 1,
            digest: format!("{:x}", Sha256::digest(content.body.as_bytes())),
            deleted: false,
        }
    }

    #[test]
    fn name_prefers_frontmatter_then_heading_then_skill_dir_then_filename() {
        for (path, text, expected) in [
            (
                "folder/SKILL.md",
                "---\nname: 'Front Matter'\ndescription: \"A description\"\n---\n# Heading",
                "front-matter",
            ),
            ("folder/SKILL.md", "\n# A heading ###\nbody", "a-heading"),
            ("Parent Folder/SKILL.md", "plain body", "parent-folder"),
            ("Other File.markdown", "plain body", "other-file"),
            (
                "./one\\Parent Folder\\SKILL.md",
                "plain body",
                "parent-folder",
            ),
            ("no-heading.md", "intro\n# Not first", "no-heading"),
            ("a.md", "# A\rB\nx", "a"),
            ("a.md", "# A\u{2028}B\nx", "a"),
            ("a.md", "# A\u{2029}B\nx", "a"),
            ("fallback.md", "---\nname: ???\n---\n# Heading", "skill"),
            ("☃.txt", "plain body", "skill"),
        ] {
            assert_eq!(convert(&file(path, text)).unwrap().content.name, expected);
        }
    }

    #[test]
    fn accents_are_preserved_in_titles_and_transliterated_in_lookup_names() {
        for (path, text) in [
            ("resume.md", "---\nname: Résumé express\n---\nCorps"),
            ("resume.md", "# Résumé express\nCorps"),
            ("Résumé express/SKILL.md", "Corps"),
            ("Résumé express.txt", "Corps"),
        ] {
            let content = convert(&file(path, text)).unwrap().content;
            assert_eq!(content.name, "resume-express");
            assert_eq!(content.display_name, "Résumé express");
        }
        let conn = database();
        let first = file("resume.md", "---\nname: Résumé express\n---\nv1");
        import_batch(&conn, "bot", &[first.clone(), first]).unwrap();
        let id = library::list(&conn, "bot").unwrap()[0].skill_id.clone();
        import_batch(
            &conn,
            "bot",
            &[file("resume.md", "---\nname: Résumé express\n---\nv2")],
        )
        .unwrap();
        let saved = library::list(&conn, "bot").unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].skill_id, id);
        assert_eq!(saved[0].version, 2);
    }

    #[test]
    fn non_latin_human_titles_are_independent_of_lookup_key_fallbacks() {
        for (path, text, title) in [
            ("tokyo.md", "# 東京メモ\nTexte", "東京メモ"),
            ("russe.md", "---\nname: Наш стиль\n---\nTexte", "Наш стиль"),
            ("東京メモ/SKILL.md", "Texte", "東京メモ"),
            ("東京メモ.txt", "Texte", "東京メモ"),
            ("punctuation.md", "---\nname: ???\n---\nTexte", "???"),
        ] {
            let content = convert(&file(path, text)).unwrap().content;
            assert_eq!(content.display_name, title);
            assert_eq!(content.name, "skill");
        }
        let conn = database();
        let first = file("russe.md", "---\nname: Наш стиль\n---\nv1");
        import_batch(&conn, "bot", &[first]).unwrap();
        let original = library::list(&conn, "bot").unwrap().remove(0);
        import_batch(
            &conn,
            "bot",
            &[file("russe.md", "---\nname: Наш стиль\n---\nv2")],
        )
        .unwrap();
        let updated = library::list(&conn, "bot").unwrap().remove(0);
        assert_eq!(updated.skill_id, original.skill_id);
        assert_eq!(updated.version, 2);
        assert_eq!(updated.content.display_name, "Наш стиль");
    }

    #[test]
    fn legacy_accented_lookup_keys_are_reused_instead_of_duplicated() {
        let conn = database();
        let mut old = convert(&file("resume.md", "---\nname: Résumé express\n---\nv1"))
            .unwrap()
            .content;
        old.name = "r-sum-express".into();
        let first = library::add(&conn, "bot", &old).unwrap();
        import_batch(
            &conn,
            "bot",
            &[file("resume.md", "---\nname: Résumé express\n---\nv2")],
        )
        .unwrap();
        let saved = library::list(&conn, "bot").unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].skill_id, first.skill_id);
        assert_eq!(saved[0].content.name, "r-sum-express");
        assert_eq!(saved[0].content.display_name, "Résumé express");
        assert_eq!(saved[0].version, 2);
    }

    #[test]
    fn editable_frontmatter_hides_only_host_fields_and_preserves_them_on_save() {
        let original = "---\r\nname: Été\r\ncustom: gardé\r\norigin: upload\r\nupload-extension: .txt\r\nupload-name: 'été.txt'\r\n---\r\n\r\norigin: exemple\r\n";
        let editable = editable_body(original);
        assert!(editable.contains("custom: gardé"));
        assert!(!editable.contains("name: Été"));
        assert!(editable.ends_with("origin: exemple\r\n"));
        assert!(!editable.contains("origin: upload"));
        let saved = preserve_managed_frontmatter(
            original,
            "---\ncustom: nouveau\norigin: model\n---\nCorps",
            "Été révisé",
            "Description",
        );
        assert!(saved.contains("name: Été révisé"));
        assert!(saved.contains(
            "custom: nouveau\nname: Été révisé\norigin: upload\nupload-extension: .txt\nupload-name: 'été.txt'"
        ));
        assert!(saved.ends_with("Corps"));
        assert!(
            preserve_managed_frontmatter(original, "Corps seul", "Été", "Description")
                .ends_with("---\nCorps seul")
        );
        for text in [
            "origin: exemple\nTexte",
            "---\ncustom: valeur\n---\nTexte",
            "---\norigin: exemple sans fermeture",
        ] {
            assert_eq!(editable_body(text), text);
            assert_eq!(
                preserve_managed_frontmatter("Corps manuel", text, "Nom", "Description"),
                text
            );
        }
        assert_eq!(editable_body("---\norigin: upload\n---\nCorps"), "Corps");
    }

    #[test]
    fn metadata_rewrites_stay_single_line_and_preserve_user_fields() {
        let original = "---\nname: Ancien\ndescription: Ancienne\nauteur: Élodie\norigin: upload\n---\nCorps\nname: exemple dans le corps";
        let name = "\"Titre\": résumé";
        let description = "Première ligne\n---\nauteur: pas un champ\r\nDernière ligne";
        let saved =
            preserve_managed_frontmatter(original, &editable_body(original), name, description);
        let (header, body) = frontmatter(&saved).unwrap();
        assert_eq!(field(header, "name"), name);
        assert_eq!(field(header, "description"), description);
        assert_eq!(field(header, "auteur"), "Élodie");
        assert_eq!(header.lines().count(), 4);
        assert_eq!(body, "Corps\nname: exemple dans le corps");
        assert_eq!(editable_body(&saved), editable_body(original));
        let only_name = preserve_managed_frontmatter(
            "---\nname: Ancien\norigin: upload\n---\nTexte",
            "Texte",
            "Nouveau",
            "Description absente du fichier",
        );
        assert!(!only_name.contains("description:"));
        assert_eq!(editable_body(&only_name), "Texte");
    }

    #[test]
    fn multiline_name_and_description_stay_authored_and_intact() {
        for header in [
            "description: |\n  Analyse un document\n  en profondeur.",
            "description: >-\n  Rédige un plan\n  en cinq points.",
            "description: Première partie\n  suite de la description.",
            "description: Première partie\n\n# commentaire\n  suite de la description.",
            "description: [premier, second]",
            "description: {fr: Bonjour}",
            "name:\n  - pas un titre",
        ] {
            let original =
                format!("---\nname: Titre\n{header}\nlicence: MIT\norigin: upload\n---\nCorps")
                    .replacen("name: Titre\nname:", "name:", 1);
            let editable = editable_body(&original);
            assert!(editable.contains(header), "{editable}");
            assert!(!editable.contains("origin: upload"));
            for edited in [&original, &editable] {
                let saved = preserve_managed_frontmatter(
                    &original,
                    edited,
                    "Nouveau",
                    "Description courte",
                );
                assert!(saved.contains(header), "{saved}");
                assert!(!saved.contains("description: \"|\""));
                assert_eq!(editable_body(&saved), editable_body(&original));
            }
        }
        let original = "---\nname: Titre\ndescription: Ancienne\norigin: upload\n---\nCorps";
        let cleared = preserve_managed_frontmatter(original, original, "Titre", "");
        assert!(cleared.contains("description: \"\""));
        assert_eq!(editable_body(&cleared), "Corps");
        let described = preserve_managed_frontmatter(&cleared, &cleared, "Titre", "Nouvelle");
        assert!(described.contains("description: Nouvelle"));
        assert_eq!(described.matches("description:").count(), 1);
    }

    #[test]
    fn a_body_opening_with_a_rule_keeps_its_separator_in_the_editor() {
        for body in [
            "---\nname: Exemple\nprix: 12 €\n---\nTexte",
            "---\ndescription: Exemple\n---\nTexte",
            "---\nTexte sans fermeture",
            "---\r\nname: Exemple\r\n---\r\nTexte",
            "---",
            "----\nTexte",
        ] {
            let original = format!("---\nname: Fiche\norigin: upload\n---\n\n{body}");
            let editable = editable_body(&original);
            assert!(editable.starts_with("\n---"), "{editable:?}");
            assert_eq!(typed_metadata(&editable, "name"), None);
            assert_eq!(typed_metadata(&editable, "description"), None);
            let saved = preserve_managed_frontmatter(&original, &editable, "Fiche", "");
            assert_eq!(saved, original);
            let extended = format!("{editable}\nname: Ajouté plus tard\n---\nCorps");
            assert_eq!(typed_metadata(&extended, "name"), None);
        }
    }

    #[test]
    fn typed_scalars_become_fields_but_authored_replacements_are_not_duplicated() {
        let original = "---\nname: Ancien\ndescription: Ancienne\norigin: upload\n---\nCorps";
        let typed =
            "---\nname: Nouveau\ndescription: \"Ligne 1\\nLigne 2\"\nauteur: Élodie\n---\nCorps";
        assert_eq!(typed_metadata(typed, "name").as_deref(), Some("Nouveau"));
        assert_eq!(
            typed_metadata(typed, "description").as_deref(),
            Some("Ligne 1\nLigne 2")
        );
        assert_eq!(
            typed_metadata("Corps\ndescription: exemple", "description"),
            None
        );
        let saved = preserve_managed_frontmatter(original, typed, "Nouveau", "Ligne 1\nLigne 2");
        assert!(saved.contains("description: \"Ligne 1\\nLigne 2\""));
        assert!(saved.contains("auteur: Élodie"));
        assert_eq!(saved.matches("description:").count(), 1);
        assert!(!editable_body(&saved).contains("description:"));
        // Older versions could store an uploaded body without any header.
        // Typed fields still follow the dedicated values in that case.
        let legacy = preserve_managed_frontmatter(
            "Ancien corps sans en-tête",
            typed,
            "Champ",
            "Description du champ",
        );
        assert!(legacy.contains("name: Champ"));
        assert!(legacy.contains("description: Description du champ"));
        assert!(!legacy.contains("name: Nouveau"));
        let authored =
            "description: |\n  Première ligne\n  suite.\ndescription: Autre ligne écrite";
        let edited = format!("---\n{authored}\nlicence: MIT\n---\nCorps");
        assert_eq!(typed_metadata(&edited, "description"), None);
        let saved = preserve_managed_frontmatter(original, &edited, "Nouveau", "Ancienne");
        assert!(saved.contains(authored));
        assert!(!saved.contains("description: Ancienne"));
        assert_eq!(saved.matches("description:").count(), 2);
        assert!(editable_body(&saved).contains(authored));
    }

    #[test]
    fn binary_and_invalid_utf8_are_skipped() {
        for bytes in [
            b"hello\0world".to_vec(),
            vec![0xff],
            vec![0xc0, 0xaf],
            vec![0xed, 0xa0, 0x80],
            vec![0xe2, 0x82],
        ] {
            assert_eq!(
                convert(&ImportFile {
                    path: "file.txt".into(),
                    bytes
                })
                .unwrap_err()
                .to_string(),
                "unsupported"
            );
        }
        let reports = import_files(
            &[
                file("bad.pdf", "text"),
                file("empty.txt", " \n\t"),
                file("", "text"),
            ],
            vec![],
            |_, _| panic!("must not write"),
        );
        assert!(reports
            .iter()
            .all(|item| matches!(item.status, ImportStatus::Skipped { .. })));
    }

    #[test]
    fn accepted_extensions_bom_and_crlf() {
        for extension in ["txt", "MD", "markdown", "csv", "json", "yaml", "yml"] {
            let imported = convert(&file(
                &format!("file.{extension}"),
                "\u{feff}---\r\nname: Test\r\ndescription: 'Description'\r\n---\r\n\r\nbody\r\n",
            ))
            .unwrap();
            assert_eq!(imported.content.name, "test");
            assert_eq!(imported.content.description, "Description");
            assert!(imported.content.body.ends_with("\nbody\r\n"));
        }
        assert!(convert(&file(".md", "text")).is_err());
        assert_eq!(
            convert(&file("file.txt", "\u{feff}\u{feff}"))
                .unwrap_err()
                .to_string(),
            "empty"
        );
        assert!(convert(&file("file.txt", "\u{85}")).is_ok());
        assert_eq!(
            convert(&file("file.md", "\n\u{feff}\n# Title"))
                .unwrap()
                .content
                .name,
            "title"
        );
    }

    #[test]
    fn content_over_256000_chars_is_skipped() {
        assert!(convert(&file("file.txt", &"a".repeat(MAX_SKILL_CHARACTERS))).is_ok());
        assert_eq!(
            convert(&file("file.txt", &"a".repeat(MAX_SKILL_CHARACTERS + 1)))
                .unwrap_err()
                .to_string(),
            "too-large"
        );
        // Match JS length: an emoji occupies two units, an accented letter one.
        assert!(convert(&file("file.txt", &"é".repeat(MAX_SKILL_CHARACTERS))).is_ok());
        assert!(convert(&file("file.txt", &"😀".repeat(MAX_SKILL_CHARACTERS / 2))).is_ok());
        assert!(convert(&file(
            "file.txt",
            &"😀".repeat(MAX_SKILL_CHARACTERS / 2 + 1)
        ))
        .is_err());
    }

    #[test]
    fn frontmatter_records_upload_origin() {
        let imported = convert(&file("O'Brien.md", "---\nname: Custom\ndescription: Desc\norigin: model\nupload-extension: .bad\nupload-name: old\nother: retained\n---\n\n# Body\n")).unwrap();
        assert_eq!(imported.content.description, "Desc");
        assert_eq!(
            imported.content.origin,
            Origin::Upload {
                extension: ".md".into(),
                name: "O'Brien.md".into()
            }
        );
        assert_eq!(imported.content.body, "---\nname: Custom\ndescription: Desc\nother: retained\norigin: upload\nupload-extension: .md\n---\n\n# Body\n");
        let imported = convert(&file("O'Brien.md", "# Title\n\nbody")).unwrap();
        assert!(imported.content.body.contains("name: Title\n"));
        assert!(imported
            .content
            .body
            .contains("upload-name: 'O''Brien.md'\n"));
        assert!(imported.content.body.ends_with("# Title\n\nbody"));
        assert!(!convert(&file("dir/SKILL.md", "body"))
            .unwrap()
            .content
            .body
            .contains("upload-name:"));
        assert_eq!(
            imported.digest,
            format!("{:x}", Sha256::digest(imported.content.body.as_bytes()))
        );
    }

    #[test]
    fn cap_of_250_counts_existing() {
        let conn = database();
        let content = convert(&file("existing.txt", "existing")).unwrap().content;
        let mut last = None;
        for _ in 0..250 {
            last = Some(library::add(&conn, "bot", &content).unwrap());
        }
        let input = file("one.txt", "one");
        let reports = import_batch(&conn, "bot", std::slice::from_ref(&input)).unwrap();
        assert_eq!(
            reports[0].status,
            ImportStatus::Skipped {
                reason: "too-many-skills".into()
            }
        );
        let last = last.unwrap();
        library::delete(&conn, &last.skill_id, last.version).unwrap();
        let reports = import_batch(&conn, "bot", std::slice::from_ref(&input)).unwrap();
        assert!(matches!(reports[0].status, ImportStatus::Written { .. }));
        assert_eq!(library::list(&conn, "bot").unwrap().len(), 250);
        let retry = import_batch(&conn, "bot", &[input]).unwrap();
        assert!(matches!(retry[0].status, ImportStatus::Duplicate { .. }));
    }

    #[test]
    fn same_digest_reimport_is_duplicate() {
        let input = file("file.md", "# Title\nbody");
        let imported = convert(&input).unwrap();
        let reports = import_files(&[input.clone(), input.clone()], vec![], |_, content| {
            Ok(skill(1, content))
        });
        assert_eq!(
            reports[0].status,
            ImportStatus::Written {
                skill_id: "skill_1".into()
            }
        );
        assert_eq!(
            reports[1].status,
            ImportStatus::Duplicate {
                skill_id: "skill_1".into()
            }
        );
        let reports = import_files(&[input], vec![skill(1, &imported.content); 250], |_, _| {
            panic!("duplicate at cap must not write")
        });
        assert_eq!(
            reports[0].status,
            ImportStatus::Duplicate {
                skill_id: "skill_1".into()
            }
        );
    }

    #[test]
    fn write_failure_keeps_successes_and_continues_in_order() {
        let files = [
            file("one.txt", "one"),
            file("two.txt", "two"),
            file("three.txt", "three"),
        ];
        let mut writes = Vec::new();
        let reports = import_files(&files, vec![], |_, content| {
            writes.push(content.name.clone());
            if content.name == "two" {
                Err(invalid("rejected"))
            } else {
                Ok(skill(writes.len(), content))
            }
        });
        assert_eq!(writes, ["one", "two", "three"]);
        assert_eq!(
            reports
                .iter()
                .map(|item| item.file_name.as_str())
                .collect::<Vec<_>>(),
            ["one.txt", "two.txt", "three.txt"]
        );
        assert!(matches!(reports[0].status, ImportStatus::Written { .. }));
        assert_eq!(
            reports[1].status,
            ImportStatus::Skipped {
                reason: "write-failed: rejected".into()
            }
        );
        assert!(matches!(reports[2].status, ImportStatus::Written { .. }));
    }

    #[test]
    fn deleted_skills_do_not_consume_capacity_or_block_reimport() {
        let input = file("file.txt", "body");
        let mut deleted = skill(1, &convert(&input).unwrap().content);
        deleted.deleted = true;
        let reports = import_files(&[input], vec![deleted; 250], |_, content| {
            Ok(skill(2, content))
        });
        assert_eq!(
            reports[0].status,
            ImportStatus::Written {
                skill_id: "skill_2".into()
            }
        );
    }

    fn database() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        crate::migrate(&conn).unwrap();
        conn
    }

    fn written_id(report: &[ImportItem]) -> &str {
        match &report[0].status {
            ImportStatus::Written { skill_id } => skill_id,
            status => panic!("expected Written, got {status:?}"),
        }
    }

    #[test]
    fn import_batch_persists_and_second_call_is_duplicate() {
        let conn = database();
        let input = file("file.md", "# Title\nbody");
        let first = import_batch(&conn, "bot", std::slice::from_ref(&input)).unwrap();
        let second = import_batch(&conn, "bot", std::slice::from_ref(&input)).unwrap();
        assert_eq!(
            second[0].status,
            ImportStatus::Duplicate {
                skill_id: written_id(&first).into()
            }
        );
        let saved = library::list(&conn, "bot").unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].digest, convert(&input).unwrap().digest);
        // Digests are scoped to a bot, not the entire workspace.
        assert!(matches!(
            import_batch(&conn, "other-bot", &[input]).unwrap()[0].status,
            ImportStatus::Written { .. }
        ));
    }

    #[test]
    fn reupload_with_same_frontmatter_name_updates_in_place() {
        let conn = database();
        let first = import_batch(
            &conn,
            "bot",
            &[file("deploy/SKILL.md", "---\nname: Deploy\n---\nv1")],
        )
        .unwrap();
        let second = import_batch(
            &conn,
            "bot",
            &[file("deploy/SKILL.md", "---\nname: ' Deploy '\n---\nv2")],
        )
        .unwrap();
        assert_eq!(written_id(&first), written_id(&second));
        let saved = library::list(&conn, "bot").unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].content.name, "deploy");
        assert_eq!(saved[0].version, 2);
        assert!(saved[0].content.body.ends_with("v2"));
        assert_eq!(
            library::history(&conn, written_id(&first)).unwrap().len(),
            2
        );
    }

    #[test]
    fn same_frontmatter_name_twice_in_one_drop_gets_suffix() {
        let conn = database();
        import_batch(
            &conn,
            "bot",
            &[file("deploy/SKILL.md", "---\nname: Deploy\n---\nv0")],
        )
        .unwrap();
        let reports = import_batch(
            &conn,
            "bot",
            &[
                file("one/SKILL.md", "---\nname: Deploy\n---\nv1"),
                file("two/SKILL.md", "---\nname: Deploy\n---\nv2"),
            ],
        )
        .unwrap();
        assert_ne!(written_id(&reports), written_id(&reports[1..]));
        let saved = library::list(&conn, "bot").unwrap();
        assert_eq!(
            saved
                .iter()
                .map(|skill| skill.content.name.as_str())
                .collect::<Vec<_>>(),
            ["deploy", "deploy-2"]
        );
        assert_eq!(saved[0].version, 2);
        assert_eq!(saved[1].version, 1);
    }

    #[test]
    fn frontmatter_reuse_requires_exact_title_and_unclaimed_name() {
        let conn = database();
        let input = file("deploy/SKILL.md", "---\nname: Deploy\n---\nv1");
        let first = import_batch(&conn, "bot", std::slice::from_ref(&input)).unwrap();
        let reports = import_batch(
            &conn,
            "bot",
            &[
                input,
                file("deploy/SKILL.md", "---\nname: Deploy\n---\nv2"),
                file("deploy/SKILL.md", "---\nname: DEPLOY\n---\nv3"),
            ],
        )
        .unwrap();
        assert_eq!(
            reports[0].status,
            ImportStatus::Duplicate {
                skill_id: written_id(&first).into()
            }
        );
        assert_ne!(written_id(&reports[1..]), written_id(&first));
        assert_eq!(library::list(&conn, "bot").unwrap().len(), 3);
    }

    #[test]
    fn named_reupload_can_update_at_capacity() {
        let conn = database();
        let input = file("deploy/SKILL.md", "---\nname: Deploy\n---\nv1");
        let first = import_batch(&conn, "bot", &[input]).unwrap();
        let content = convert(&file("other.txt", "other")).unwrap().content;
        for _ in 0..249 {
            library::add(&conn, "bot", &content).unwrap();
        }
        let second = import_batch(
            &conn,
            "bot",
            &[
                file("x.md", "# Deploy\nreadme"),
                file("deploy/SKILL.md", "---\nname: Deploy\n---\nv2"),
            ],
        )
        .unwrap();
        assert_eq!(
            second[0].status,
            ImportStatus::Skipped {
                reason: "too-many-skills".into()
            }
        );
        assert_eq!(written_id(&first), written_id(&second[1..]));
        let saved = library::list(&conn, "bot").unwrap();
        assert_eq!(saved.len(), 250);
        let updated = saved
            .iter()
            .find(|skill| skill.skill_id == written_id(&first))
            .unwrap();
        assert_eq!(updated.version, 2);
        assert!(updated.content.body.ends_with("v2"));
    }

    #[test]
    fn heading_file_suffixed_before_named_reupload_still_updates() {
        let conn = database();
        let first = import_batch(
            &conn,
            "bot",
            &[file("deploy/SKILL.md", "---\nname: Deploy\n---\nv1")],
        )
        .unwrap();
        let reports = import_batch(
            &conn,
            "bot",
            &[
                file("deploy/README.md", "# Deploy\nreadme"),
                file("deploy/SKILL.md", "---\nname: Deploy\n---\nv2"),
            ],
        )
        .unwrap();
        assert_eq!(written_id(&reports[1..]), written_id(&first));
        let saved = library::list(&conn, "bot").unwrap();
        assert_eq!(saved.len(), 2);
        assert_eq!(saved[0].content.name, "deploy");
        assert_eq!(saved[0].version, 2);
        assert!(saved[0].content.body.ends_with("v2"));
        assert_eq!(saved[1].content.name, "deploy-2");
        assert_eq!(saved[1].skill_id, written_id(&reports));
        assert!(saved[1].content.body.ends_with("readme"));
    }

    #[test]
    fn failed_heading_does_not_claim_existing_name() {
        let conn = database();
        let first = import_batch(
            &conn,
            "bot",
            &[file("deploy/SKILL.md", "---\nname: Deploy\n---\nv1")],
        )
        .unwrap();
        conn.execute_batch(
            "CREATE TRIGGER reject_readme BEFORE INSERT ON skills_library
            WHEN NEW.body LIKE '%readme' BEGIN SELECT RAISE(ABORT, 'reject readme'); END;",
        )
        .unwrap();
        let reports = import_batch(
            &conn,
            "bot",
            &[
                file("deploy/README.md", "# Deploy\nreadme"),
                file("deploy/SKILL.md", "---\nname: Deploy\n---\nv2"),
            ],
        )
        .unwrap();
        assert!(
            matches!(&reports[0].status, ImportStatus::Skipped { reason } if reason.starts_with("write-failed:"))
        );
        assert_eq!(written_id(&reports[1..]), written_id(&first));
        let saved = library::list(&conn, "bot").unwrap();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].content.name, "deploy");
        assert_eq!(saved[0].version, 2);
        assert!(saved[0].content.body.ends_with("v2"));
    }

    #[test]
    fn lost_transaction_stops_writes_and_refuses_false_report() {
        let conn = database();
        conn.execute_batch("BEGIN IMMEDIATE").unwrap();
        let files = [file("a.txt", "a"), file("b.txt", "b"), file("c.txt", "c")];
        let mut writes = 0;
        let result = persist_files(&conn, &files, vec![], |_, content| {
            writes += 1;
            if writes == 2 {
                conn.execute_batch("ROLLBACK").unwrap();
                Err(invalid("simulated SQLite transaction loss"))
            } else {
                library::add(&conn, "bot", content)
            }
        });
        assert_eq!(
            result.unwrap_err().to_string(),
            "import transaction was rolled back by SQLite"
        );
        assert_eq!(writes, 2);
        assert!(conn.is_autocommit());
        assert!(library::list(&conn, "bot").unwrap().is_empty());
    }

    #[test]
    fn horizontal_rules_and_empty_frontmatter_preserve_client_content() {
        let imported = convert(&file("a.md", "---\n---\n# Head")).unwrap();
        assert_eq!(imported.content.name, "a");
        assert!(imported.content.body.ends_with("\n\n---\n---\n# Head"));
        assert!(frontmatter("---\nname: a\n---\r").is_none());
        let imported = convert(&file("a.md", "---\n\n---\nbody")).unwrap();
        assert_eq!(imported.content.body, "---\nname: a\n\norigin: upload\nupload-extension: .md\nupload-name: 'a.md'\n---\n\nbody");
    }
}
