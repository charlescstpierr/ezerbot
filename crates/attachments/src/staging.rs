//! Content-addressed staging signatures, implemented by gb-attachments-core.
use grokbot_engine::contracts::{pending, Result};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::path::Path;

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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub sha256: String,
    pub image_type: ImageType,
    pub byte_length: u64,
    pub staged_at: i64,
}

pub fn migrate(_conn: &Connection) -> Result<()> {
    Ok(())
}
pub fn sniff(_bytes: &[u8]) -> Result<ImageType> {
    pending("attachments.staging")
}
pub fn stage(
    _conn: &Connection,
    _base_path: &Path,
    _bytes: &[u8],
    _now_ms: i64,
) -> Result<Attachment> {
    pending("attachments.staging")
}
pub fn get(_conn: &Connection, _id: &str) -> Result<Option<Attachment>> {
    pending("attachments.staging")
}
pub fn read(_conn: &Connection, _base_path: &Path, _id: &str) -> Result<Vec<u8>> {
    pending("attachments.staging")
}
pub fn link_attachments(
    _conn: &Connection,
    _conversation_id: &str,
    _message_id: &str,
    _ids: &[String],
) -> Result<()> {
    pending("attachments.staging")
}
pub fn for_message(_conn: &Connection, _message_id: &str) -> Result<Vec<Attachment>> {
    pending("attachments.staging")
}
