//! `content_digest` of a WebDAV server that stores a SHA-256 for the file:
//! ownCloud and Nextcloud report `oc:checksums` (`SHA1:… MD5:… SHA256:…`,
//! Nextcloud developer manual, "Basic file and folder operations", checksum
//! types MD5, SHA1, SHA256, SHA3-256 and Adler32), so the digest comes back in
//! the one PROPFIND that also tells size and time, and no content moves.
//!
//! What the server reports is what a client stored with the upload
//! (`OC-Checksum`); a plain `PUT` is stored unchecked. This backend never
//! sends that header, so it is not the source of what it reads back, but the
//! value is a claim by whoever uploaded the file, not a hash the server
//! computed: callers that compare a copy with its source must not take it as
//! proof of the stored bytes (the engine's verification leaves WebDAV out,
//! `file_evidence::on_device` in `transfer/os/shared/engine/file_evidence.rs`). A server without the property, or without a
//! SHA-256 entry (MD5 and SHA1 do not fit the SHA-256 contract of
//! `vfs::content_digest`), answers `Ok(None)`.
use super::core_impl::WebdavBackend;
use super::multistatus::successful_props;
use crate::vfs::{is_sha256_hex, ContentDigest, VfsResult};
use std::io;

/// The lowercase hex SHA-256 among the `ALGORITHM:value` tokens of one
/// `oc:checksums` text; `SHA256` and `SHA-256` both name the algorithm.
pub(super) fn sha256_token(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|token| {
        let (kind, value) = token.split_once(':')?;
        let named = kind.eq_ignore_ascii_case("SHA256") || kind.eq_ignore_ascii_case("SHA-256");
        let value = value.to_ascii_lowercase();
        (named && is_sha256_hex(&value)).then_some(value)
    })
}

/// The SHA-256 and the size of the resource a Depth-0 answer describes.
pub(super) fn from_propfind(xml: &str) -> VfsResult<Option<ContentDigest>> {
    let invalid = |detail: &str| io::Error::new(io::ErrorKind::InvalidData, detail.to_string());
    let doc = roxmltree::Document::parse(xml).map_err(|error| invalid(&error.to_string()))?;
    let mut responses = doc
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "response");
    let response = responses
        .next()
        .ok_or_else(|| invalid("WebDAV Depth-0 response is missing"))?;
    if responses.next().is_some() {
        return Err(invalid("WebDAV Depth-0 returned multiple resources"));
    }
    let props = successful_props(response)?;
    let find = |name: &str| {
        props
            .iter()
            .flat_map(|prop| prop.descendants())
            .find(|node| node.is_element() && node.tag_name().name() == name)
    };
    if find("collection").is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "no regular file: WebDAV collection",
        ));
    }
    let Some(size) = find("getcontentlength")
        .and_then(|node| node.text())
        .and_then(|text| text.trim().parse::<u64>().ok())
    else {
        return Ok(None);
    };
    let sha256 = find("checksums").and_then(|node| {
        node.descendants()
            .find_map(|node| node.text().and_then(sha256_token))
    });
    Ok(sha256.map(|sha256| ContentDigest { sha256, size }))
}

impl WebdavBackend {
    /// `BackendExtensions::content_digest`: one PROPFIND.
    pub(super) fn digest_from_checksums(&self, path: &str) -> VfsResult<Option<ContentDigest>> {
        from_propfind(&self.propfind(path, "0")?)
    }
}
