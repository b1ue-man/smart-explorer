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
//!
//! `content_md5` (the sync signature) reads the `MD5:` entry of the same
//! property from the same one PROPFIND; the value is the uploader's claim
//! here too, exactly as the MD5 the listings already carry. No MD5 entry
//! answers `Ok(None)` and the caller keeps its way (hashing the content).
//!
//! A server that does not report the property (Apache `mod_dav`, nginx, IIS
//! answer it in a 404 propstat or not at all) never will for another file of
//! the same connection either, and the listings read the same property: so
//! each connection remembers, per algorithm, whether its answers carry it
//! (`ChecksumMemory`). Once a listing with files or the first Depth-0 answer
//! showed none, `content_md5`/`content_digest` answer `Ok(None)` without a
//! request; a later listing or answer that carries one makes the connection
//! ask again.
use super::core_impl::WebdavBackend;
use super::multistatus::{extract_md5, successful_props};
use crate::vfs::{is_sha256_hex, ContentDigest, VfsMeta, VfsResult};
use std::io;
use std::sync::atomic::{AtomicU8, Ordering};

const UNKNOWN: u8 = 0;
const ABSENT: u8 = 1;
const PRESENT: u8 = 2;

/// What this connection's answers showed about `oc:checksums`, per
/// algorithm. "Present" stays once seen; "absent" holds until an answer
/// carries a checksum.
#[derive(Default, Debug)]
pub(super) struct ChecksumMemory {
    md5: AtomicU8,
    sha256: AtomicU8,
}

impl ChecksumMemory {
    /// Whether answers of this connection carried no MD5 so far.
    pub(super) fn md5_absent(&self) -> bool {
        self.md5.load(Ordering::Relaxed) == ABSENT
    }

    /// Whether answers of this connection carried no SHA-256 so far.
    pub(super) fn sha256_absent(&self) -> bool {
        self.sha256.load(Ordering::Relaxed) == ABSENT
    }

    pub(super) fn note_md5(&self, found: bool) {
        note(&self.md5, found);
    }

    pub(super) fn note_sha256(&self, found: bool) {
        note(&self.sha256, found);
    }

    /// A listing: its files' MD5s come from the same property. A listing
    /// without files says nothing.
    pub(super) fn note_listing(&self, entries: &[VfsMeta]) {
        let mut files = entries.iter().filter(|entry| !entry.is_dir).peekable();
        if files.peek().is_none() {
            return;
        }
        let found = files.any(|entry| entry.content_md5.is_some());
        self.note_md5(found);
    }
}

fn note(state: &AtomicU8, found: bool) {
    if found {
        state.store(PRESENT, Ordering::Relaxed);
    } else {
        let _ = state.compare_exchange(UNKNOWN, ABSENT, Ordering::Relaxed, Ordering::Relaxed);
    }
}

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
    Ok(checksum_of(xml, sha256_token)?.map(|(size, sha256)| ContentDigest { sha256, size }))
}

/// The lowercase MD5 the Depth-0 answer reports for the regular file.
pub(super) fn md5_from_propfind(xml: &str) -> VfsResult<Option<String>> {
    Ok(checksum_of(xml, |text| {
        extract_md5(text).map(|hex| hex.to_ascii_lowercase())
    })?
    .map(|(_, md5)| md5))
}

/// The size and the checksum `pick` takes from the `oc:checksums` text of
/// the one resource a Depth-0 answer describes.
fn checksum_of(
    xml: &str,
    pick: impl Fn(&str) -> Option<String>,
) -> VfsResult<Option<(u64, String)>> {
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
    let found = find("checksums").and_then(|node| {
        node.descendants()
            .find_map(|node| node.text().and_then(&pick))
    });
    Ok(found.map(|value| (size, value)))
}

impl WebdavBackend {
    /// `BackendExtensions::content_digest`: one PROPFIND, none where this
    /// connection's answers carried no SHA-256.
    pub(super) fn digest_from_checksums(&self, path: &str) -> VfsResult<Option<ContentDigest>> {
        if self.checksums.sha256_absent() {
            return Ok(None);
        }
        let found = from_propfind(&self.propfind(path, "0")?)?;
        self.checksums.note_sha256(found.is_some());
        Ok(found)
    }

    /// `BackendExtensions::content_md5`: the same one PROPFIND, none where
    /// this connection's answers carried no MD5.
    pub(super) fn md5_from_checksums(&self, path: &str) -> VfsResult<Option<String>> {
        if self.checksums.md5_absent() {
            return Ok(None);
        }
        let found = md5_from_propfind(&self.propfind(path, "0")?)?;
        self.checksums.note_md5(found.is_some());
        Ok(found)
    }
}
