//! Local document reading. REA CM07 establishes named committed references, not
//! a server parser. These caps and local extraction are reconstruction choices.
//! No Office macros, formulas, links or embedded objects are executed. ZIP entries
//! remain in memory, bounded independently of compressed upload size. PDFKit and
//! textutil receive bytes on stdin, never a user-controlled command or file path.
use grokbot_engine::contracts::{Error, Result};
use quick_xml::{events::Event, Reader};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::io::{Cursor, Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const MAX_BYTES: usize = 20 * 1024 * 1024;
pub const MAX_PAGES: usize = 50;
pub const MAX_TEXT: usize = 100_000;
const MAX_XML: u64 = 16 * 1024 * 1024;
const MAX_EXPANDED: u64 = 64 * 1024 * 1024;
const FORMATS: &[&str] = &["pdf", "docx", "xlsx", "csv", "pptx", "txt", "md", "rtf"];

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Document {
    pub id: String,
    pub name: String,
    pub format: String,
    pub byte_length: usize,
    pub page_count: usize,
    /// Physical pages for PDF, slides for PPTX, sheets for XLSX; otherwise an
    /// explicit estimate (4,000 Unicode characters per page).
    pub page_kind: String,
    pub pages_read: usize,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encoding_note: Option<String>,
    pub note: Option<String>,
    pub text: String,
}
fn invalid(message: &str) -> Error {
    Error::InvalidInput(message.into())
}
pub fn format_count(value: usize) -> String {
    let digits = value.to_string();
    let mut text = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            text.push('\u{202f}');
        }
        text.push(digit);
    }
    text
}
fn display_note(document: &Document) -> Option<String> {
    let mut notes = document.encoding_note.iter().cloned().collect::<Vec<_>>();
    if document.truncated {
        let read = format_count(document.pages_read);
        let count = format_count(document.page_count);
        let characters = format_count(MAX_TEXT);
        let detail = if document.page_kind == "pages estimées" {
            format!("environ {read} pages sur {count} transmises. Lecture limitée aux {characters} premiers caractères")
        } else {
            let unit = if document.pages_read == 1 {
                match document.page_kind.as_str() {
                    "pages" => "page",
                    "feuilles" => "feuille",
                    "diapositives" => "diapositive",
                    other => other,
                }
            } else {
                &document.page_kind
            };
            let plural = if document.pages_read == 1 { "" } else { "s" };
            format!("{read} {unit} parcourue{plural} sur {count}. Lecture limitée à {MAX_PAGES} {} et {characters} caractères", document.page_kind)
        };
        notes.push(format!(
            "Texte tronqué : {detail}. La suite du document n’a pas été transmise."
        ));
    }
    (!notes.is_empty()).then(|| notes.join(" "))
}
fn stored_document(record: &str) -> Result<Document> {
    let mut document: Document = serde_json::from_str(record)
        .map_err(|_| invalid("L’extraction enregistrée est illisible."))?;
    // Refresh presentation for older records without re-parsing their source.
    document.note = display_note(&document);
    Ok(document)
}
pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS attachments_documents (
        id TEXT PRIMARY KEY, record TEXT NOT NULL);
        CREATE TABLE IF NOT EXISTS attachments_document_links (
        conversation_id TEXT NOT NULL, message_id TEXT NOT NULL,
        document_id TEXT NOT NULL REFERENCES attachments_documents(id), ordinal INTEGER NOT NULL,
        PRIMARY KEY(conversation_id, message_id, document_id),
        UNIQUE(conversation_id, message_id, ordinal));",
    )?;
    Ok(())
}
pub fn get(conn: &Connection, id: &str) -> Result<Option<Document>> {
    let record: Option<String> = conn
        .query_row(
            "SELECT record FROM attachments_documents WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    record.map(|s| stored_document(&s)).transpose()
}
pub fn for_message(conn: &Connection, conversation: &str, message: &str) -> Result<Vec<Document>> {
    let mut query = conn.prepare("SELECT d.record FROM attachments_document_links l JOIN attachments_documents d ON d.id=l.document_id WHERE conversation_id=?1 AND message_id=?2 ORDER BY ordinal")?;
    let rows = query.query_map(params![conversation, message], |r| r.get::<_, String>(0))?;
    rows.map(|s| stored_document(&s?)).collect()
}
pub fn link(conn: &Connection, conversation: &str, message: &str, ids: &[String]) -> Result<()> {
    for id in ids {
        if get(conn, id)?.is_none() {
            return Err(invalid(
                "Document introuvable. Retirez-le puis joignez-le à nouveau.",
            ));
        }
    }
    for id in ids {
        conn.execute("INSERT INTO attachments_document_links SELECT ?1, ?2, ?3, COALESCE(MAX(ordinal)+1,0) FROM attachments_document_links WHERE conversation_id=?1 AND message_id=?2 ON CONFLICT DO NOTHING", params![conversation, message, id])?;
    }
    Ok(())
}
pub fn stage(conn: &Connection, name: &str, bytes: &[u8]) -> Result<Document> {
    if bytes.is_empty() {
        return Err(invalid("Le document est vide."));
    }
    if bytes.len() > MAX_BYTES {
        return Err(invalid("Le document dépasse la limite de 20 Mo."));
    }
    if name.is_empty()
        || name.len() > 512
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
    {
        return Err(invalid("Le nom du document est invalide."));
    }
    let format = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if !FORMATS.contains(&format.as_str()) {
        return Err(invalid(
            "Format non accepté : PDF, DOCX, XLSX, CSV, PPTX, TXT, MD ou RTF uniquement.",
        ));
    }
    let id = format!(
        "document_{}",
        super::staging::digest(&[name.as_bytes(), b"\0", bytes].concat())
    );
    if let Some(cached) = get(conn, &id)? {
        return Ok(cached);
    }
    if bytes.starts_with(b"\xd0\xcf\x11\xe0") {
        return Err(invalid("Document Office chiffré ou ancien format non accepté. Exportez une copie non protégée en DOCX, XLSX ou PPTX."));
    }
    let mut encoding_note = None;
    let (mut text, page_count, page_kind, pages_read, mut truncated) = match format.as_str() {
        "pdf" => pdf(bytes)?,
        "docx" | "xlsx" | "pptx" => office(bytes, &format)?,
        "rtf" => {
            if !bytes.starts_with(b"{\\rtf") {
                return Err(invalid("Ce fichier RTF est invalide."));
            }
            let output = builtin(
                "/usr/bin/textutil",
                &["-convert", "txt", "-format", "rtf", "-stdin", "-stdout"],
                bytes,
            )?;
            estimated(String::from_utf8(output).map_err(|_| invalid("Texte RTF illisible."))?)
        }
        _ => {
            let (text, encoding) = decode_text(bytes)?;
            encoding_note = encoding;
            estimated(text)
        }
    };
    let char_cap = if page_kind == "pages estimées" {
        MAX_TEXT.min(MAX_PAGES * 4000)
    } else {
        MAX_TEXT
    };
    let cut = text.char_indices().nth(char_cap).map(|(i, _)| i);
    if let Some(cut) = cut {
        text.truncate(cut);
        truncated = true;
    }
    if text.trim().is_empty() {
        return Err(invalid("Aucun texte lisible dans ce document. Pour un PDF numérisé, exportez une version avec reconnaissance de texte (OCR)."));
    }
    let mut document = Document {
        id,
        name: name.into(),
        format,
        byte_length: bytes.len(),
        page_count,
        page_kind,
        pages_read,
        truncated,
        encoding_note,
        note: None,
        text,
    };
    document.note = display_note(&document);
    conn.execute(
        "INSERT INTO attachments_documents VALUES (?1, ?2) ON CONFLICT DO NOTHING",
        params![document.id, serde_json::to_string(&document)?],
    )?;
    Ok(document)
}
fn estimated(text: String) -> (String, usize, String, usize, bool) {
    let pages = text.chars().count().div_ceil(4000).max(1);
    (
        text,
        pages,
        "pages estimées".into(),
        pages.min(MAX_TEXT.div_ceil(4000)),
        pages > MAX_PAGES,
    )
}
fn decode_text(bytes: &[u8]) -> Result<(String, Option<String>)> {
    let mut encoding_note = None;
    let text = if bytes.starts_with(b"\xff\xfe") || bytes.starts_with(b"\xfe\xff") {
        if !bytes.len().is_multiple_of(2) {
            return Err(invalid(
                "Encodage texte invalide. Utilisez UTF-8 ou UTF-16.",
            ));
        }
        let little = bytes.starts_with(b"\xff\xfe");
        let words: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| {
                if little {
                    u16::from_le_bytes([b[0], b[1]])
                } else {
                    u16::from_be_bytes([b[0], b[1]])
                }
            })
            .collect();
        String::from_utf16(&words).map_err(|_| invalid("Encodage UTF-16 invalide."))?
    } else if let Ok(text) = std::str::from_utf8(bytes) {
        text.trim_start_matches('\u{feff}').into()
    } else {
        // Excel's legacy CSV exports commonly use Windows-1252. Never guess
        // around NUL/C0 binary data; keep the assumption visible to the user.
        if bytes
            .iter()
            .any(|b| *b < 0x20 && !matches!(*b, b'\t' | b'\r' | b'\n' | 0x0c))
        {
            return Err(invalid(
                "Ce fichier contient des données binaires, pas du texte.",
            ));
        }
        const C1: [char; 32] = [
            '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
            '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ',
            '\u{9d}', 'ž', 'Ÿ',
        ];
        encoding_note = Some("Encodage Windows-1252 supposé.".into());
        bytes
            .iter()
            .map(|b| {
                if (0x80..=0x9f).contains(b) {
                    C1[usize::from(*b - 0x80)]
                } else {
                    char::from(*b)
                }
            })
            .collect()
    };
    if text
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\u{c}'))
    {
        return Err(invalid(
            "Ce fichier contient des données binaires, pas du texte.",
        ));
    }
    Ok((text, encoding_note))
}

// Collect just visible XML text, retaining paragraph/cell boundaries and cached
// values. DTDs/entities are refused; no external resource is ever resolved.
fn xml(bytes: &[u8], spreadsheet: bool, shared: &[String]) -> Result<String> {
    let mut reader = Reader::from_reader(bytes);
    let mut out = String::new();
    let mut capture = false;
    let mut shared_cell = false;
    let mut cell_address = String::new();
    let mut value = String::new();
    let mut depth = 0usize;
    let mut root = false;
    loop {
        match reader
            .read_event()
            .map_err(|_| invalid("Document Office invalide : XML illisible."))?
        {
            Event::Start(e) => {
                if depth == 0 && root {
                    return Err(invalid("Document Office invalide : plusieurs racines XML."));
                }
                for attribute in e.attributes() {
                    attribute.map_err(|_| invalid("Attribut XML invalide."))?;
                }
                depth += 1;
                root = true;
                match e.local_name().as_ref() {
                    b"c" => {
                        shared_cell = false;
                        cell_address.clear();
                        for attribute in e.attributes() {
                            let attribute = attribute
                                .map_err(|_| invalid("Attribut de cellule XLSX invalide."))?;
                            if attribute.key.as_ref() == b"t" {
                                shared_cell = attribute.value.as_ref() == b"s";
                            }
                            if attribute.key.as_ref() == b"r" {
                                let address = attribute
                                    .decode_and_unescape_value(reader.decoder())
                                    .map_err(|_| invalid("Adresse de cellule XLSX invalide."))?;
                                cell_address = address.into_owned();
                            }
                        }
                    }
                    b"t" | b"v" => {
                        capture = true;
                        value.clear();
                    }
                    _ => {}
                }
            }
            Event::Text(e) if capture => value.push_str(
                &e.xml_content()
                    .map_err(|_| invalid("Encodage XML invalide."))?,
            ),
            Event::GeneralRef(e) if capture => {
                let reference = e.decode().map_err(|_| invalid("Entité XML invalide."))?;
                let escaped = format!("&{reference};");
                value.push_str(
                    &quick_xml::escape::unescape(&escaped)
                        .map_err(|_| invalid("Entité XML non acceptée."))?,
                );
            }
            Event::CData(e) if capture => {
                value.push_str(&e.decode().map_err(|_| invalid("Texte XML invalide."))?)
            }
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                match e.local_name().as_ref() {
                    b"t" | b"v" => {
                        if spreadsheet && !value.is_empty() && !cell_address.is_empty() {
                            out.push_str(&format!("[{cell_address}] "));
                            cell_address.clear();
                        }
                        if spreadsheet && shared_cell && e.local_name().as_ref() == b"v" {
                            let index: usize = value
                                .parse()
                                .map_err(|_| invalid("Index de cellule XLSX invalide."))?;
                            out.push_str(
                                shared
                                    .get(index)
                                    .ok_or_else(|| invalid("Texte partagé XLSX manquant."))?,
                            );
                        } else {
                            out.push_str(&value);
                        }
                        capture = false;
                    }
                    b"p" | b"row" | b"si" => out.push('\n'),
                    b"tc" | b"c" => out.push('\t'),
                    _ => {}
                }
            }
            Event::Empty(e) => {
                if depth == 0 && root {
                    return Err(invalid("Document Office invalide : plusieurs racines XML."));
                }
                for attribute in e.attributes() {
                    attribute.map_err(|_| invalid("Attribut XML invalide."))?;
                }
                root = true;
                if matches!(e.local_name().as_ref(), b"tab" | b"br" | b"cr") {
                    out.push(' ');
                }
            }
            Event::DocType(_) => {
                return Err(invalid(
                    "Les déclarations DTD ne sont pas acceptées dans les documents.",
                ))
            }
            Event::Eof => {
                if depth != 0 || !root {
                    return Err(invalid("Document Office invalide : XML incomplet."));
                }
                break;
            }
            _ => {}
        }
        if out.len() > MAX_XML as usize || value.len() > MAX_XML as usize {
            return Err(invalid("Texte Office trop volumineux après décompression."));
        }
    }
    Ok(out)
}
fn shared_strings(bytes: &[u8]) -> Result<Vec<String>> {
    // Validate the outer XML too (read_to_end below skips the rich-text runs).
    let _ = xml(bytes, false, &[])?;
    let mut reader = Reader::from_reader(bytes);
    let mut strings = Vec::new();
    loop {
        match reader
            .read_event()
            .map_err(|_| invalid("Textes partagés XLSX invalides."))?
        {
            Event::Start(e) if e.local_name().as_ref() == b"si" => {
                let start = reader.buffer_position() as usize;
                let span = reader
                    .read_to_end(e.name())
                    .map_err(|_| invalid("Texte partagé XLSX incomplet."))?;
                let inner = &bytes[start..span.end as usize];
                let wrapped = [b"<si>".as_slice(), inner, b"</si>"].concat();
                let mut text = xml(&wrapped, false, &[])?;
                text.pop(); // Only the synthetic si boundary, not embedded newlines.
                strings.push(text);
            }
            Event::Empty(e) if e.local_name().as_ref() == b"si" => strings.push(String::new()),
            Event::DocType(_) => return Err(invalid("DTD XLSX non acceptée.")),
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(strings)
}
fn office(bytes: &[u8], format: &str) -> Result<(String, usize, String, usize, bool)> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))
        .map_err(|_| invalid("Document Office invalide ou chiffré."))?;
    if archive.len() > 10_000 {
        return Err(invalid(
            "Document Office trop complexe (plus de 10 000 éléments).",
        ));
    }
    let mut expanded = 0u64;
    let mut names = Vec::new();
    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|_| invalid("Document Office chiffré ou illisible."))?;
        expanded = expanded.saturating_add(file.size());
        if file.encrypted() {
            return Err(invalid(
                "Document chiffré : exportez une copie sans mot de passe.",
            ));
        }
        if expanded > MAX_EXPANDED || file.size() > MAX_XML {
            return Err(invalid("Document Office trop volumineux après décompression (64 Mio au total, 16 Mio par élément)."));
        }
        names.push(file.name().to_owned());
    }
    let read = |archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str| -> Result<Vec<u8>> {
        let file = archive
            .by_name(name)
            .map_err(|_| invalid("Document Office invalide : élément requis manquant."))?;
        let mut bytes = Vec::new();
        file.take(MAX_XML + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| {
                invalid(
                    "Document Office corrompu : contrôle d’intégrité ou décompression impossible.",
                )
            })?;
        if bytes.len() as u64 > MAX_XML {
            return Err(invalid("Élément Office trop volumineux."));
        }
        Ok(bytes)
    };
    if !names.iter().any(|n| n == "[Content_Types].xml") {
        return Err(invalid("Ce fichier n’est pas un document Office valide."));
    }
    if format == "docx" {
        return Ok(estimated(xml(
            &read(&mut archive, "word/document.xml")?,
            false,
            &[],
        )?));
    }
    let (prefix, kind) = if format == "pptx" {
        ("ppt/slides/slide", "diapositives")
    } else {
        ("xl/worksheets/sheet", "feuilles")
    };
    let mut parts: Vec<_> = names
        .iter()
        .filter_map(|name| {
            name.strip_prefix(prefix)?
                .strip_suffix(".xml")?
                .parse::<usize>()
                .ok()
                .map(|n| (n, name.clone()))
        })
        .collect();
    parts.sort_by_key(|(n, _)| *n);
    if parts.is_empty() {
        return Err(invalid(
            "Le document ne contient aucune diapositive ou feuille lisible.",
        ));
    }
    let shared = if format == "xlsx" && names.iter().any(|n| n == "xl/sharedStrings.xml") {
        // Each si is one string, even when it consists of several rich-text runs.
        shared_strings(&read(&mut archive, "xl/sharedStrings.xml")?)?
    } else {
        Vec::new()
    };
    let mut text = String::new();
    let mut pages_read = 0;
    let mut has_text = false;
    for (n, name) in parts.iter().take(MAX_PAGES) {
        pages_read += 1;
        text.push_str(&format!("\n[{kind} {n}]\n"));
        let extracted = xml(&read(&mut archive, name)?, format == "xlsx", &shared)?;
        has_text |= !extracted.trim().is_empty();
        text.push_str(&extracted);
        if text.chars().count() > MAX_TEXT {
            break;
        }
    }
    if !has_text {
        return Err(invalid("Aucun texte lisible dans les premières feuilles ou diapositives. Les images et diagrammes seuls ne sont pas convertis en texte."));
    }
    let truncated = parts.len() > MAX_PAGES || text.chars().count() > MAX_TEXT;
    Ok((text, parts.len(), kind.into(), pages_read, truncated))
}

/// Fixed macOS readers, no shell. Drain bounded output concurrently to avoid pipe
/// deadlock, kill on timeout, and reap before returning. Never expose stderr.
fn builtin(program: &str, args: &[&str], input: &[u8]) -> Result<Vec<u8>> {
    builtin_with_timeout(program, args, input, Duration::from_secs(20))
}
fn builtin_with_timeout(
    program: &str,
    args: &[&str],
    input: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| invalid("Le lecteur local macOS n’est pas disponible."))?;
    let mut stdin = child.stdin.take().expect("piped stdin");
    let input = input.to_vec();
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let stdout = child.stdout.take().expect("piped stdout");
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        stdout.take(MAX_XML + 1).read_to_end(&mut out).map(|_| out)
    });
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let _ = writer.join();
    let out = reader
        .join()
        .map_err(|_| invalid("Lecture locale interrompue."))?
        .map_err(|_| invalid("La sortie du lecteur local est illisible."))?;
    if timed_out {
        return Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "Délai de lecture locale dépassé.",
        )
        .into());
    }
    if out.len() as u64 > MAX_XML {
        return Err(invalid(
            "Le texte extrait dépasse la limite de sécurité de 16 Mio.",
        ));
    }
    if !status.is_some_and(|s| s.success()) {
        return Err(invalid(
            "Document invalide ou illisible. Exportez une copie non protégée.",
        ));
    }
    Ok(out)
}
fn pdf(bytes: &[u8]) -> Result<(String, usize, String, usize, bool)> {
    if !bytes.starts_with(b"%PDF-") {
        return Err(invalid("Ce fichier PDF est invalide."));
    }
    // PDFKit reads only text, not JavaScript, actions, annotations or links.
    const SCRIPT: &str = r#"ObjC.import('Foundation'); ObjC.import('PDFKit');
const bytes = $.NSFileHandle.fileHandleWithStandardInput.readDataToEndOfFile;
function extract() {
const doc = $.PDFDocument.alloc.initWithData(bytes);
if (!doc || doc.isNil()) return {error:'invalid'};
if (doc.isLocked) return {error:'encrypted'};
const count = Number(doc.pageCount); let text = ''; let read = 0;
for (let i=0; i<Math.min(count,50); i++) { const raw=doc.pageAtIndex(i).string; const s=raw ? ObjC.unwrap(raw) : ''; text += '\n[Page '+(i+1)+']\n'+(typeof s==='string' ? s : ''); read++; if (text.length>100000) break; }
const chars = Array.from(text);
return {text:chars.slice(0,100000).join(''), count:count, read:read, truncated:read<count || chars.length>100000};
}
const result = JSON.stringify(extract());
$.NSFileHandle.fileHandleWithStandardOutput.writeData($(result).dataUsingEncoding($.NSUTF8StringEncoding));"#;
    let out = builtin(
        "/usr/bin/osascript",
        &["-l", "JavaScript", "-e", SCRIPT],
        bytes,
    )?;
    #[derive(Deserialize)]
    struct Pdf {
        text: String,
        count: usize,
        read: usize,
        truncated: bool,
    }
    let value: serde_json::Value =
        serde_json::from_slice(&out).map_err(|_| invalid("Ce PDF est illisible."))?;
    if value["error"] == "encrypted" {
        return Err(invalid(
            "Ce PDF est chiffré. Exportez une copie sans mot de passe.",
        ));
    }
    let pdf: Pdf = serde_json::from_value(value).map_err(|_| invalid("Ce PDF est invalide."))?;
    // Page labels alone do not count as extracted text.
    let visible = pdf
        .text
        .lines()
        .filter(|s| !s.starts_with("[Page "))
        .collect::<String>();
    if visible.trim().is_empty() {
        return Err(invalid("Ce PDF ne contient pas de texte lisible. Une reconnaissance de texte (OCR) est nécessaire."));
    }
    Ok((pdf.text, pdf.count, "pages".into(), pdf.read, pdf.truncated))
}

#[cfg(test)]
#[path = "document_tests.rs"]
mod tests;
