//! The walk of `WalkHashed2` for a served backend without a walk of its own
//! (`backend_hash.rs`): depth-first over tolerant listings; links, special
//! files and unreadable entries are omissions, only the root's listing
//! failing fails the walk, digests are read only where asked.
use std::io::{self, Read};
use std::sync::atomic::{AtomicBool, Ordering};

use ring::digest::{Context, SHA256};

use crate::agent_proto::CHUNK;
use crate::analytics::HashAlgorithm;
use crate::vfs::{
    self as vfs, BackendHandle, HashWalkItem, HashWalkRequest, OmissionReason, VfsOmission,
};

use super::super::backend_budget::WalkBudget;
use super::{canceled, join_path, rel_join, Items};

pub(super) struct Walk<'a, 'i> {
    pub(super) items: &'a mut Items<'i>,
    pub(super) backend: &'a BackendHandle,
    pub(super) request: HashWalkRequest,
    pub(super) cancel: &'a AtomicBool,
    pub(super) budget: WalkBudget,
}

impl Walk<'_, '_> {
    fn emit(&mut self, item: HashWalkItem) -> io::Result<()> {
        self.items.send(item)
    }

    fn omit(&mut self, rel: String, reason: OmissionReason, detail: String) -> io::Result<()> {
        self.emit(HashWalkItem::Omitted(VfsOmission {
            rel,
            reason,
            detail,
        }))
    }

    /// Depth-first over tolerant listings; only the root's listing failing
    /// fails the walk.
    pub(super) fn run(&mut self, root: &str) -> io::Result<()> {
        self.budget.record(root, 0)?;
        let mut stack = vec![(root.to_string(), String::new(), 0usize)];
        while let Some((directory, rel_dir, depth)) = stack.pop() {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(canceled());
            }
            let listing = match vfs::list_dir_tolerant(&**self.backend, &directory) {
                Ok(listing) => listing,
                Err(error) if rel_dir.is_empty() => return Err(error),
                Err(error) => {
                    let reason = vfs::omission_reason(&error).unwrap_or(OmissionReason::Unreadable);
                    self.omit(rel_dir, reason, error.to_string())?;
                    continue;
                }
            };
            for omitted in listing.omitted {
                let rel = rel_join(&rel_dir, &omitted.rel);
                self.omit(rel, omitted.reason, omitted.detail)?;
            }
            for entry in listing.entries {
                if self.cancel.load(Ordering::Relaxed) {
                    return Err(canceled());
                }
                let rel = rel_join(&rel_dir, &entry.name);
                if crate::vfs::validate_child_name(&entry.name).is_err() {
                    self.omit(
                        rel,
                        OmissionReason::Unrepresentable,
                        "Name ist kein Pfad".into(),
                    )?;
                    continue;
                }
                let path = join_path(&directory, &entry.name);
                self.budget.record(&path, depth + 1)?;
                if entry.is_symlink {
                    self.omit(rel, OmissionReason::Link, "Verknüpfung".into())?;
                    continue;
                }
                if entry.special {
                    self.omit(
                        rel,
                        OmissionReason::Special,
                        "Pipe, Socket oder Gerät".into(),
                    )?;
                    continue;
                }
                if entry.is_dir {
                    self.emit(HashWalkItem::Entry(vfs::HashWalkEntry {
                        rel: rel.clone(),
                        is_dir: true,
                        size: 0,
                        mtime_ms: entry.mtime_ms,
                        digest: None,
                        meta: None,
                    }))?;
                    stack.push((path, rel, depth + 1));
                    continue;
                }
                if entry.size < self.request.min_bytes {
                    continue;
                }
                let digest = match self.request.algorithm {
                    None => None,
                    // A provider MD5 of the listing needs no download.
                    Some(HashAlgorithm::Md5) if entry.content_md5.is_some() => {
                        entry.content_md5.clone()
                    }
                    Some(algorithm) => {
                        match self.digest(&path, entry.id.as_deref(), entry.size, algorithm) {
                            Ok(digest) => Some(digest),
                            Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                                return Err(error)
                            }
                            Err(error) => {
                                let reason = vfs::omission_reason(&error)
                                    .unwrap_or(OmissionReason::Unreadable);
                                self.omit(rel, reason, error.to_string())?;
                                continue;
                            }
                        }
                    }
                };
                self.emit(HashWalkItem::Entry(vfs::HashWalkEntry {
                    rel,
                    is_dir: false,
                    size: entry.size,
                    mtime_ms: entry.mtime_ms,
                    digest,
                    meta: None,
                }))?;
            }
        }
        Ok(())
    }

    /// Lowercase hex digest of the whole content; a length other than the
    /// listed one is a file that changed while it was walked.
    fn digest(
        &self,
        path: &str,
        id: Option<&str>,
        size: u64,
        algorithm: HashAlgorithm,
    ) -> io::Result<String> {
        let mut reader = vfs::open_read_regular(&**self.backend, path, id)?;
        let mut md5 = md5::Context::new();
        let mut sha256 = Context::new(&SHA256);
        let mut buffer = vec![0u8; CHUNK];
        let mut read = 0u64;
        loop {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(canceled());
            }
            let count = match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            };
            read = read.saturating_add(count as u64);
            match algorithm {
                HashAlgorithm::Md5 => md5.consume(&buffer[..count]),
                HashAlgorithm::Sha256 => sha256.update(&buffer[..count]),
            }
        }
        if read != size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Datei hat sich während des Walks geändert",
            ));
        }
        Ok(match algorithm {
            HashAlgorithm::Md5 => format!("{:x}", md5.compute()),
            HashAlgorithm::Sha256 => sha256
                .finish()
                .as_ref()
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        })
    }
}
