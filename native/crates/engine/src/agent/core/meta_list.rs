//! `list_dir` and `stat` of an agent location. A server with `meta-list-v1`
//! (SSH agent and background service of this version) answers
//! `ListDirMeta`/`StatMeta` with every listing attribute: hidden and system
//! flags as the served backend reports them (the service) or by leading dot
//! (the SSH agent, as SFTP), and creation times as the agent's walk carries
//! them. An older server gets `ListDir`/`Stat` as before (hidden by leading
//! dot on an SSH agent, never on the service; no creation time).
use std::io;
use std::sync::atomic::AtomicBool;

use super::agent_error::agent_error;
use super::backend::AgentBackend;
use super::ext_stream::stream;
use super::metadata::{hash_entry_to_vfs, wire_to_vfs, wires_to_vfs};
use crate::agent_proto::{Frame, WireHashEntry};
use crate::vfs::{VfsMeta, VfsResult};

fn unexpected(what: &str, other: &Frame) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("unexpected agent {what} reply: {other:?}"),
    )
}

/// The attributed entries of one `ListDirMeta`/`StatMeta` reply.
fn attributed(backend: &AgentBackend, request: Frame) -> VfsResult<Vec<WireHashEntry>> {
    let mut entries = Vec::new();
    stream(
        &backend.pool,
        request,
        &AtomicBool::new(false),
        |frame| match frame {
            Frame::HashPart {
                entries: part,
                omitted,
            } if omitted.is_empty() => {
                entries.extend(part);
                Ok(())
            }
            other => Err(unexpected("attribute listing", &other)),
        },
    )?;
    Ok(entries)
}

pub(super) fn list_dir(backend: &AgentBackend, path: &str) -> VfsResult<Vec<VfsMeta>> {
    let features = backend.features();
    if features.extensions && features.meta_list {
        return attributed(backend, Frame::ListDirMeta(path.to_string()))?
            .into_iter()
            .map(hash_entry_to_vfs)
            .collect();
    }
    match backend.metadata_call(Frame::ListDir(path.to_string()))? {
        Frame::Dir(v) => Ok(wires_to_vfs(v, !features.service)),
        Frame::Err(e) => Err(agent_error(e)),
        other => Err(unexpected("directory", &other)),
    }
}

pub(super) fn stat(backend: &AgentBackend, path: &str) -> VfsResult<VfsMeta> {
    let features = backend.features();
    if features.extensions && features.meta_list {
        let mut entries = attributed(backend, Frame::StatMeta(path.to_string()))?;
        let entry = match (entries.pop(), entries.is_empty()) {
            (Some(entry), true) => entry,
            _ => return Err(unexpected("status", &Frame::End)),
        };
        let meta = entry.meta.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "attribute status without attributes",
            )
        })?;
        return Ok(VfsMeta {
            name: entry.rel,
            is_dir: entry.is_dir,
            is_symlink: meta.is_symlink,
            special: meta.special,
            size: entry.size,
            mtime_ms: entry.mtime_ms,
            btime_ms: meta.btime_ms,
            hidden: meta.hidden,
            system: meta.system,
            id: None,
            content_md5: entry.digest,
        });
    }
    match backend.metadata_call(Frame::Stat(path.to_string()))? {
        Frame::Meta(m) => Ok(wire_to_vfs(m, !features.service)),
        Frame::Err(e) => Err(agent_error(e)),
        other => Err(unexpected("metadata", &other)),
    }
}
