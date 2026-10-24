//! Content-addressed image staging. Bytes live in `<base>.attachments/<sha256>`,
//! never in the submission journal. Only magic-byte detection is performed, not
//! image decoding. The wrapper owns transactions and component schema versions.
//! The host owns orphan/temporary cleanup, including its durable deletion log.
//! `sweep_temporaries` remains available to standalone callers.
use grokbot_engine::contracts::{Error, Result};
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

pub const MAX_IMAGE_BYTES: usize = 20 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageType {
    Png,
    Jpeg,
    Gif,
    Webp,
    Bmp,
}
impl ImageType {
    pub fn mime_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
            Self::Bmp => "image/bmp",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub sha256: String,
    pub image_type: ImageType,
    pub byte_length: u64,
    pub staged_at: i64,
}

pub fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS attachments_staged (
            id TEXT PRIMARY KEY,
            sha256 TEXT NOT NULL UNIQUE,
            image_type TEXT NOT NULL,
            byte_length INTEGER NOT NULL CHECK(byte_length > 0 AND byte_length <= 20971520),
            staged_at INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS attachments_message_links (
            conversation_id TEXT NOT NULL,
            message_id TEXT NOT NULL,
            attachment_id TEXT NOT NULL REFERENCES attachments_staged(id),
            ordinal INTEGER NOT NULL,
            PRIMARY KEY(conversation_id, message_id, attachment_id),
            UNIQUE(conversation_id, message_id, ordinal)
        );",
    )?;
    let scoped: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('attachments_message_links')
         WHERE name = 'conversation_id' AND pk = 1)",
        [],
        |row| row.get(0),
    )?;
    let upgrade_pending: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'attachments_message_links_scoped')",
        [], |row| row.get(0),
    )?;
    if !scoped || upgrade_pending {
        // The caller owns the transaction. The staging table also makes this
        // copy resumable if a caller ran migration without one and was killed.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS attachments_message_links_scoped (
                conversation_id TEXT NOT NULL,
                message_id TEXT NOT NULL,
                attachment_id TEXT NOT NULL REFERENCES attachments_staged(id),
                ordinal INTEGER NOT NULL,
                PRIMARY KEY(conversation_id, message_id, attachment_id),
                UNIQUE(conversation_id, message_id, ordinal)
            );
            INSERT OR IGNORE INTO attachments_message_links_scoped
                SELECT conversation_id, message_id, attachment_id, ordinal FROM attachments_message_links;
            DROP TABLE attachments_message_links;
            ALTER TABLE attachments_message_links_scoped RENAME TO attachments_message_links;",
        )?;
    }
    Ok(())
}

pub fn sniff(bytes: &[u8]) -> Result<ImageType> {
    if bytes.len() >= 24 && bytes.starts_with(b"\x89PNG\r\n\x1a\n") && &bytes[12..16] == b"IHDR" {
        Ok(ImageType::Png)
    } else if bytes.len() >= 4 && bytes.starts_with(b"\xff\xd8\xff") {
        Ok(ImageType::Jpeg)
    } else if bytes.len() >= 13 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        Ok(ImageType::Gif)
    } else if bytes.len() >= 16
        && bytes.starts_with(b"RIFF")
        && &bytes[8..12] == b"WEBP"
        && matches!(&bytes[12..16], b"VP8 " | b"VP8L" | b"VP8X")
    {
        Ok(ImageType::Webp)
    } else if bytes.len() >= 26
        && bytes.starts_with(b"BM")
        && {
            let declared = u32::from_le_bytes(bytes[2..6].try_into().unwrap()) as usize;
            declared == 0 || (26..=bytes.len()).contains(&declared)
        }
        && matches!(
            u32::from_le_bytes(bytes[14..18].try_into().unwrap()),
            12 | 40 | 52 | 56 | 64 | 108 | 124
        )
    {
        Ok(ImageType::Bmp)
    } else {
        Err(Error::InvalidInput("unsupported image signature".into()))
    }
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub(crate) fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

fn blob_directory(base_path: &Path) -> PathBuf {
    let mut directory = base_path.as_os_str().to_os_string();
    directory.push(".attachments");
    PathBuf::from(directory)
}

pub fn content_path(base_path: &Path, attachment: &Attachment) -> Result<PathBuf> {
    blob_path(base_path, &attachment.sha256)
}

fn blob_path(base_path: &Path, sha256: &str) -> Result<PathBuf> {
    if !valid_digest(sha256) {
        return Err(Error::InvalidInput("invalid attachment SHA-256".into()));
    }
    Ok(blob_directory(base_path).join(sha256))
}

/// Remove only our regular, hour-old temporary files, never content blobs or
/// fresh uploads. Also callable at host startup; staging invokes it best-effort.
pub fn sweep_temporaries(base_path: &Path) -> Result<usize> {
    let cutoff = SystemTime::now() - Duration::from_secs(3_600);
    let entries = match fs::read_dir(blob_directory(base_path)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.into()),
    };
    let mut removed = 0;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        let Some(sha) = name
            .to_str()
            .and_then(|s| s.strip_prefix('.'))
            .and_then(|s| s.strip_suffix(".tmp"))
        else {
            continue;
        };
        if !valid_digest(sha) || !entry.file_type()?.is_file() {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.into()),
        };
        if metadata.modified()? <= cutoff {
            match fs::remove_file(entry.path()) {
                Ok(()) => removed += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    Ok(removed)
}

/// Publish a complete file atomically so concurrent readers cannot see a partial
/// upload. A rolled-back database transaction can leave an unreferenced blob;
/// this is safe because blobs are immutable and addressed by their content.
fn write_blob(path: &Path, bytes: &[u8]) -> Result<()> {
    let directory = path.parent().expect("blob path has a parent");
    fs::create_dir_all(directory)?;
    let mut nonce = [0; 16];
    getrandom::getrandom(&mut nonce).map_err(|e| std::io::Error::other(e.to_string()))?;
    let temporary = directory.join(format!(".{}.tmp", digest(&nonce)));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    let result = (|| -> Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        fs::File::open(directory)?.sync_all()?;
        // Persist the attachments directory's entry as well if just created.
        let parent = directory
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

pub fn stage(conn: &Connection, base_path: &Path, bytes: &[u8], now_ms: i64) -> Result<Attachment> {
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err(Error::InvalidInput(
            "image must contain between 1 byte and 20 MiB".into(),
        ));
    }
    let image_type = sniff(bytes)?;
    let sha256 = digest(bytes);
    let id = format!("attachment_{sha256}");
    write_blob(&blob_path(base_path, &sha256)?, bytes)?;
    conn.execute(
        "INSERT INTO attachments_staged(id, sha256, image_type, byte_length, staged_at)
         VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(sha256) DO UPDATE SET staged_at=excluded.staged_at",
        params![
            id,
            sha256,
            image_type.mime_type(),
            bytes.len() as u64,
            now_ms
        ],
    )?;
    get(conn, &id)?.ok_or_else(|| Error::InvalidInput("staged attachment is missing".into()))
}

fn attachment_row(row: &Row<'_>) -> rusqlite::Result<Attachment> {
    let mime: String = row.get(2)?;
    let image_type = match mime.as_str() {
        "image/png" => ImageType::Png,
        "image/jpeg" => ImageType::Jpeg,
        "image/gif" => ImageType::Gif,
        "image/webp" => ImageType::Webp,
        "image/bmp" => ImageType::Bmp,
        _ => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                2,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "invalid stored image type",
                )),
            ));
        }
    };
    Ok(Attachment {
        id: row.get(0)?,
        sha256: row.get(1)?,
        image_type,
        byte_length: row.get(3)?,
        staged_at: row.get(4)?,
    })
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Attachment>> {
    Ok(conn
        .query_row(
            "SELECT id, sha256, image_type, byte_length, staged_at
             FROM attachments_staged WHERE id = ?1",
            [id],
            attachment_row,
        )
        .optional()?)
}

pub fn read(conn: &Connection, base_path: &Path, id: &str) -> Result<Vec<u8>> {
    let attachment =
        get(conn, id)?.ok_or_else(|| Error::InvalidInput("attachment not found".into()))?;
    let bytes = fs::read(blob_path(base_path, &attachment.sha256)?)?;
    if bytes.len() as u64 != attachment.byte_length
        || digest(&bytes) != attachment.sha256
        || sniff(&bytes)? != attachment.image_type
    {
        return Err(Error::InvalidInput("attachment content is corrupt".into()));
    }
    Ok(bytes)
}

/// Link in first-seen order; retries do not duplicate or reorder attachments.
/// Validate the entire list before writing. The caller wraps this together with
/// the message submission in its own transaction.
pub fn link_attachments(
    conn: &Connection,
    conversation_id: &str,
    message_id: &str,
    ids: &[String],
) -> Result<()> {
    if conversation_id.is_empty() || message_id.is_empty() {
        return Err(Error::InvalidInput(
            "conversation and message IDs are required".into(),
        ));
    }
    for id in ids {
        if get(conn, id)?.is_none() {
            return Err(Error::InvalidInput(format!("attachment not found: {id}")));
        }
    }
    for id in ids {
        conn.execute(
            "INSERT INTO attachments_message_links(conversation_id, message_id, attachment_id, ordinal)
             SELECT ?1, ?2, ?3, COALESCE(MAX(ordinal) + 1, 0)
             FROM attachments_message_links WHERE conversation_id = ?1 AND message_id = ?2
             ON CONFLICT(conversation_id, message_id, attachment_id) DO NOTHING",
            params![conversation_id, message_id, id],
        )?;
    }
    Ok(())
}

pub fn for_message(
    conn: &Connection,
    conversation_id: &str,
    message_id: &str,
) -> Result<Vec<Attachment>> {
    let mut query = conn.prepare_cached(
        "SELECT a.id, a.sha256, a.image_type, a.byte_length, a.staged_at
         FROM attachments_message_links l JOIN attachments_staged a ON a.id = l.attachment_id
         WHERE l.conversation_id = ?1 AND l.message_id = ?2 ORDER BY l.ordinal",
    )?;
    let rows = query.query_map(params![conversation_id, message_id], attachment_row)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR\0\0\0\x01\0\0\0\x01image payload";
    const GIF: &[u8] = b"GIF89a\x01\0\x01\0\0\0\0payload";

    struct Fixture {
        directory: PathBuf,
        base: PathBuf,
        conn: Connection,
    }
    impl Fixture {
        fn new() -> Self {
            let mut nonce = [0; 16];
            getrandom::getrandom(&mut nonce).unwrap();
            let directory =
                std::env::temp_dir().join(format!("grokbot-attachments-{}", digest(&nonce)));
            fs::create_dir_all(&directory).unwrap();
            let base = directory.join("workspace.sqlite");
            let conn = Connection::open(&base).unwrap();
            migrate(&conn).unwrap();
            Self {
                directory,
                base,
                conn,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn mime_sniffed_from_magic_bytes() {
        let mut bmp = vec![0; 26];
        bmp[..2].copy_from_slice(b"BM");
        bmp[2..6].copy_from_slice(&26_u32.to_le_bytes());
        bmp[14..18].copy_from_slice(&12_u32.to_le_bytes());
        let mut padded_bmp = vec![0; 72];
        padded_bmp[..2].copy_from_slice(b"BM");
        padded_bmp[2..6].copy_from_slice(&70_u32.to_le_bytes());
        padded_bmp[14..18].copy_from_slice(&40_u32.to_le_bytes());
        let mut zero_size_bmp = padded_bmp.clone();
        zero_size_bmp[2..6].copy_from_slice(&0_u32.to_le_bytes());
        for (bytes, expected) in [
            (PNG, ImageType::Png),
            (&b"\xff\xd8\xff\xe0"[..], ImageType::Jpeg),
            (&b"GIF87a\x01\0\x01\0\0\0\0"[..], ImageType::Gif),
            (GIF, ImageType::Gif),
            (&b"RIFF\x10\0\0\0WEBPVP8 "[..], ImageType::Webp),
            (bmp.as_slice(), ImageType::Bmp),
            (padded_bmp.as_slice(), ImageType::Bmp),
            (zero_size_bmp.as_slice(), ImageType::Bmp),
        ] {
            assert_eq!(sniff(bytes).unwrap(), expected);
            assert!(expected.mime_type().starts_with("image/"));
        }
        for invalid in [
            b"".as_slice(),
            b"png",
            b"RIFF1234WAVE",
            b"\x89PNG",
            b"<svg/>",
            b"BMW is a car brand, not a bitmap image",
            b"BM",
            b"\xff\xd8\xff",
            b"\x89PNG\r\n\x1a\n",
            b"GIF87a",
            b"GIF89a",
            b"RIFF\0\0\0\0WEBP",
            b"RIFF1234WEBPFAKE",
        ] {
            assert!(sniff(invalid).is_err());
        }
        bmp[2] = 27;
        assert!(sniff(&bmp).is_err());
        bmp[2] = 26;
        bmp[14] = 11;
        assert!(sniff(&bmp).is_err());
        let mut fake_png = PNG.to_vec();
        fake_png[12] = b'X';
        assert!(sniff(&fake_png).is_err());
    }

    #[test]
    fn stale_temporaries_are_swept_without_removing_blobs_or_live_uploads() {
        let f = Fixture::new();
        assert_eq!(sweep_temporaries(&f.base).unwrap(), 0);
        let attachment = stage(&f.conn, &f.base, PNG, 0).unwrap();
        let directory = blob_directory(&f.base);
        let old = directory.join(format!(".{}.tmp", digest(b"old")));
        let fresh = directory.join(format!(".{}.tmp", digest(b"fresh")));
        let foreign = directory.join(".foreign.tmp");
        for path in [&old, &fresh, &foreign] {
            fs::write(path, b"temporary").unwrap();
        }
        fs::File::open(&old)
            .unwrap()
            .set_times(
                fs::FileTimes::new().set_modified(SystemTime::now() - Duration::from_secs(3_601)),
            )
            .unwrap();
        assert_eq!(sweep_temporaries(&f.base).unwrap(), 1);
        assert!(!old.exists());
        assert!(fresh.exists());
        assert!(foreign.exists());
        assert_eq!(read(&f.conn, &f.base, &attachment.id).unwrap(), PNG);
    }

    #[test]
    fn empty_and_over_20_mib_rejected() {
        let f = Fixture::new();
        assert!(stage(&f.conn, &f.base, b"", 0).is_err());
        let mut bytes = vec![0; MAX_IMAGE_BYTES + 1];
        bytes[..PNG.len()].copy_from_slice(PNG);
        assert!(stage(&f.conn, &f.base, &bytes, 0).is_err());
        assert!(!f.base.with_extension("sqlite.attachments").exists());
        bytes.pop();
        let attachment = stage(&f.conn, &f.base, &bytes, 0).unwrap();
        assert_eq!(attachment.byte_length, MAX_IMAGE_BYTES as u64);
        assert_eq!(read(&f.conn, &f.base, &attachment.id).unwrap(), bytes);
    }

    #[test]
    fn staging_is_content_addressed_and_survives_restart() {
        let f = Fixture::new();
        let first = stage(&f.conn, &f.base, PNG, 123).unwrap();
        let again = stage(&f.conn, &f.base, PNG, 999).unwrap();
        assert_eq!(first.id, again.id);
        assert_eq!(again.staged_at, 999);
        let path = blob_path(&f.base, &first.sha256).unwrap();
        assert_eq!(path.file_name().unwrap(), first.sha256.as_str());
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        let reopened = Connection::open(&f.base).unwrap();
        migrate(&reopened).unwrap();
        assert_eq!(get(&reopened, &first.id).unwrap(), Some(again));
        assert_eq!(read(&reopened, &f.base, &first.id).unwrap(), PNG);
        assert!(get(&reopened, "../../missing").unwrap().is_none());
        assert!(read(&reopened, &f.base, "../../missing").is_err());
        fs::write(path, b"corrupt").unwrap();
        assert!(read(&reopened, &f.base, &first.id).is_err());
    }

    #[test]
    fn links_preserve_order_are_idempotent_and_validate_all_ids() {
        let f = Fixture::new();
        let a = stage(&f.conn, &f.base, PNG, 1).unwrap();
        let b = stage(&f.conn, &f.base, GIF, 2).unwrap();
        let ids = vec![b.id.clone(), a.id.clone(), b.id.clone()];
        link_attachments(&f.conn, "conversation", "message", &ids).unwrap();
        link_attachments(&f.conn, "conversation", "message", &ids).unwrap();
        assert_eq!(
            for_message(&f.conn, "conversation", "message").unwrap(),
            vec![b.clone(), a.clone()]
        );
        link_attachments(&f.conn, "other", "message", std::slice::from_ref(&a.id)).unwrap();
        assert_eq!(
            for_message(&f.conn, "other", "message").unwrap(),
            vec![a.clone()]
        );
        assert_eq!(
            for_message(&f.conn, "conversation", "message").unwrap(),
            vec![b, a.clone()]
        );
        assert!(for_message(&f.conn, "unrelated", "message")
            .unwrap()
            .is_empty());
        assert!(
            link_attachments(&f.conn, "conversation", "new", &[a.id, "missing".into()]).is_err()
        );
        assert!(for_message(&f.conn, "conversation", "new")
            .unwrap()
            .is_empty());
    }

    #[test]
    fn legacy_message_links_gain_conversation_scoping_without_losing_links() {
        let f = Fixture::new();
        let a = stage(&f.conn, &f.base, PNG, 1).unwrap();
        let b = stage(&f.conn, &f.base, GIF, 2).unwrap();
        f.conn.execute_batch("DROP TABLE attachments_message_links;
            CREATE TABLE attachments_message_links (
                conversation_id TEXT NOT NULL, message_id TEXT NOT NULL,
                attachment_id TEXT NOT NULL REFERENCES attachments_staged(id), ordinal INTEGER NOT NULL,
                PRIMARY KEY(message_id, attachment_id), UNIQUE(message_id, ordinal));").unwrap();
        link_legacy(&f.conn, &a.id);
        migrate(&f.conn).unwrap();
        migrate(&f.conn).unwrap();
        assert_eq!(for_message(&f.conn, "first", "1").unwrap(), vec![a]);
        link_attachments(&f.conn, "second", "1", std::slice::from_ref(&b.id)).unwrap();
        assert_eq!(for_message(&f.conn, "second", "1").unwrap(), vec![b]);
    }

    fn link_legacy(conn: &Connection, attachment_id: &str) {
        conn.execute(
            "INSERT INTO attachments_message_links VALUES ('first', '1', ?1, 0)",
            [attachment_id],
        )
        .unwrap();
    }

    #[test]
    fn migrations_and_links_participate_in_callers_transaction() {
        let f = Fixture::new();
        f.conn.pragma_update(None, "user_version", 77).unwrap();
        migrate(&f.conn).unwrap();
        crate::precaption::migrate(&f.conn).unwrap();
        let version: i64 = f
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 77);
        let tx = f.conn.unchecked_transaction().unwrap();
        let a = stage(&tx, &f.base, PNG, 1).unwrap();
        link_attachments(&tx, "conversation", "message", std::slice::from_ref(&a.id)).unwrap();
        tx.rollback().unwrap();
        assert!(get(&f.conn, &a.id).unwrap().is_none());
        assert!(for_message(&f.conn, "conversation", "message")
            .unwrap()
            .is_empty());
        // An unreferenced content blob after rollback can be staged again safely.
        assert_eq!(stage(&f.conn, &f.base, PNG, 2).unwrap().id, a.id);
    }
}
