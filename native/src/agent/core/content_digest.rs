//! `query::FILE_DIGEST` client: the agent (or the background service) hashes
//! one of its files and sends only the SHA-256. A server that does not
//! answer the query with 1 yields `Ok(None)` and never gets the request; any
//! reply other than exactly one root entry with a valid digest is an error.
use std::io;
use std::sync::atomic::AtomicBool;

use super::super::backend::AgentBackend;
use super::super::ext_stream::{is_unsupported_error, stream, unexpected};
use crate::agent_proto::{digest, query, Frame};
use crate::vfs::{ContentDigest, VfsResult};

fn invalid(text: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, text.to_string())
}

pub(super) fn file_digest(
    backend: &AgentBackend,
    path: &str,
    cancel: &AtomicBool,
) -> VfsResult<Option<ContentDigest>> {
    if backend.query(query::FILE_DIGEST, path)? != 1 {
        return Ok(None);
    }
    let request = Frame::WalkHashed2 {
        root: path.to_string(),
        algorithm: digest::SHA256,
        min_bytes: 0,
    };
    let mut found: Option<ContentDigest> = None;
    let result = stream(&backend.pool, request, cancel, |frame| match frame {
        Frame::HashEntry {
            rel,
            is_dir,
            size,
            md5,
            ..
        } => {
            let valid = found.is_none() && rel.is_empty() && !is_dir;
            match md5 {
                Some(sha256) if valid && crate::vfs::is_sha256_hex(&sha256) => {
                    found = Some(ContentDigest { sha256, size });
                    Ok(())
                }
                _ => Err(invalid("invalid agent file digest entry")),
            }
        }
        Frame::HashOmitted(omitted) => {
            let text = format!("agent could not hash the file: {}", omitted.detail);
            Err(invalid(&text))
        }
        other => Err(unexpected("file digest", &other)),
    });
    match result {
        Ok(()) => found
            .map(Some)
            .ok_or_else(|| invalid("agent file digest ended without its entry")),
        // A folder root (or a server that withdrew the extension).
        Err(error) if found.is_none() && is_unsupported_error(&error) => Ok(None),
        Err(error) => Err(error),
    }
}
