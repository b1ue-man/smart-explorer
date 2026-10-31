//! `content_digest` of a Drive file: Google computes the SHA-256 of the stored
//! content and reports it as the output-only field `sha256Checksum` of the
//! file resource (Drive API v3, `files`: "The SHA256 checksum associated with
//! this file, if available ... only populated for files with content stored in
//! Google Drive; it's not populated for Docs Editors or shortcut files"). One
//! `files.get` by the exact ID answers; no content moves. A Google-native
//! document, a file Drive has no checksum for (not yet computed, empty) or a
//! folder answers `Ok(None)` or an error as the contract says.
use super::api::FOLDER_MIME;
use super::core::cloud_urlenc;
use super::GDriveBackend;
use crate::vfs::{is_sha256_hex, ContentDigest, VfsResult};
use std::io;

const DIGEST_FIELDS: &str = "id,mimeType,trashed,size,sha256Checksum";

/// The digest in one `files.get` answer for `id`.
pub(super) fn from_file(json: &serde_json::Value, id: &str) -> VfsResult<Option<ContentDigest>> {
    let invalid = |detail: &str| io::Error::new(io::ErrorKind::InvalidData, detail.to_string());
    if json["id"].as_str() != Some(id) {
        return Err(invalid("Drive returned a different object than asked for"));
    }
    if json["trashed"].as_bool() == Some(true) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Drive file is in the trash",
        ));
    }
    let mime = json["mimeType"].as_str().unwrap_or("");
    if mime == FOLDER_MIME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no regular file: Drive folder",
        ));
    }
    if mime.starts_with("application/vnd.google-apps.") {
        // Docs Editors files and shortcuts have no stored content.
        return Ok(None);
    }
    let Some(sha256) = json["sha256Checksum"]
        .as_str()
        .map(str::to_ascii_lowercase)
        .filter(|hex| is_sha256_hex(hex))
    else {
        return Ok(None);
    };
    let size = json["size"]
        .as_str()
        .and_then(|size| size.parse::<u64>().ok())
        .ok_or_else(|| invalid("Drive file has a checksum but no size"))?;
    Ok(Some(ContentDigest { sha256, size }))
}

impl GDriveBackend {
    /// `BackendExtensions::content_digest`: one request by the file's ID.
    pub(super) fn digest_from_checksum(&self, path: &str) -> VfsResult<Option<ContentDigest>> {
        let id = self.resolve(path)?;
        let json = self.get_json(&self.api_url(&format!(
            "files/{}?fields={DIGEST_FIELDS}",
            cloud_urlenc(&id)
        )))?;
        from_file(&json, &id)
    }
}
