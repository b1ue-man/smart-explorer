//! The proof the daylight-saving rule needs (`time_window.rs`): two files of
//! equal size whose times are exactly one hour apart count as the same only
//! when their contents are the same, shown by digests that each side forms
//! where it stores its file (`vfs::content_digest`: a local disk reads its
//! own file, an SSH server, se-agent, Share host, FTP `HASH` or Drive hashes
//! there and sends only the digest; `vfs::content_md5` where both sides form
//! a MD5 there). Nothing moves file content over the network for it. WebDAV's
//! `oc:checksums` is a claim of whoever uploaded the file, not a hash the
//! server formed, so it proves nothing here (as in the copy verification,
//! `file_evidence::on_device`).
//!
//! `None` when either side cannot form a digest where it stores the file,
//! or a digest failed: the caller then compares the times as if there were
//! no rule (an hour newer is newer).
use crate::vfs::{content_digest, content_md5, Backend, Scheme};
use std::sync::atomic::AtomicBool;

/// One file on its side.
#[derive(Clone, Copy)]
pub struct Stored<'a> {
    pub backend: &'a dyn Backend,
    pub path: &'a str,
}

impl<'a> Stored<'a> {
    pub fn new(backend: &'a dyn Backend, path: &'a str) -> Self {
        Self { backend, path }
    }
}

/// Whether `a` and `b` hold the same bytes, by digests formed where each is
/// stored; `None` when that cannot be shown without moving content.
pub fn same_content_where_stored(
    a: Stored<'_>,
    b: Stored<'_>,
    cancel: &AtomicBool,
) -> Option<bool> {
    if !hashes_itself(a.backend) || !hashes_itself(b.backend) {
        return None;
    }
    // The side behind the network first: when it cannot hash, the local
    // side is not read for nothing.
    let (first, second) = if a.backend.is_local() { (b, a) } else { (a, b) };
    if let Some(one) = sha256_of(first, cancel) {
        if let Some(other) = sha256_of(second, cancel) {
            return Some(one == other);
        }
    }
    let one = md5_of(first, cancel)?;
    let other = md5_of(second, cancel)?;
    Some(one == other)
}

/// Whether a digest of this backend is formed from the stored bytes.
fn hashes_itself(backend: &dyn Backend) -> bool {
    backend.scheme() != Scheme::Webdav
}

fn sha256_of(file: Stored<'_>, cancel: &AtomicBool) -> Option<(String, u64)> {
    content_digest(file.backend, file.path, cancel)
        .ok()
        .flatten()
        .map(|digest| (digest.sha256, digest.size))
}

fn md5_of(file: Stored<'_>, cancel: &AtomicBool) -> Option<String> {
    content_md5(file.backend, file.path, cancel).ok().flatten()
}

#[cfg(test)]
#[path = "hour_proof_tests.rs"]
mod tests;
