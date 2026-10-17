//! Writing into the vault (C2.6): every write encrypts into a stage of the
//! storage and is published by the storage's own promotion (replace,
//! no-replace, by ID). Stage names map by `path_map::stage_plan` and stay
//! in the content directory of their folder. A new long-named file gets its
//! `<digest>.c9s/` folder and `name.c9s` at publication; `contents.c9r`
//! arrives last with the storage's no-replace promotion as the gate.
use std::io::{self, Write};

use crate::cryptomator::{
    cleartext_file_size, encrypt_name, file_size, read_bounded, CONTENTS_FILE, DIR_FILE,
    INFLATED_NAME_FILE, INFLATED_NAME_READ_LIMIT, SYMLINK_FILE,
};
use crate::vault::core::listing::LONG_NAME_MISMATCH;
use crate::vault::core::path_map::{child, stage_plan};
use crate::vfs::{self, Backend, VfsMeta};

use super::backend::VaultBackend;
use super::backend_io::{Abandon, Commit, VaultWriter};
use super::backend_nodes::{
    check_name, invalid_data, invalid_input, not_found, Found, NodeKind, NodeRef, Parts,
};

/// How a stage is published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Promote {
    /// Replace an existing file, create a missing one.
    Replace,
    /// Only when the destination is missing.
    Create,
    /// Replace the file with this storage ID.
    ToId(Option<String>),
}

fn already_exists() -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        "Eintrag im Tresor existiert bereits",
    )
}

fn random_u64() -> io::Result<u64> {
    let mut bytes = [0u8; 8];
    getrandom::getrandom(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(u64::from_le_bytes(bytes))
}

/// `/<parent…>/<name>`.
fn join_path(parent: &[&str], name: &str) -> String {
    let mut path = String::new();
    for component in parent {
        path.push('/');
        path.push_str(component);
    }
    format!("{path}/{name}")
}

/// Removes a storage stage this process created and never published:
/// the storage's own discard, else a plain removal of the unique name.
pub(super) fn discard_storage_stage(inner: &dyn Backend, at: &str) {
    match inner.discard_copy_stage(at) {
        Err(error) if error.kind() == io::ErrorKind::Unsupported => {
            let _ = inner.remove_file(at);
        }
        _ => {}
    }
}

impl VaultBackend {
    /// Where the cleartext stage `parts` is stored (vault-root-relative);
    /// `None` when its name is no stage name.
    pub(super) fn stage_rel(&self, parts: &Parts<'_>) -> io::Result<Option<String>> {
        let Some(plan) = stage_plan(parts.name) else {
            return Ok(None);
        };
        let parent = self.dir_for(&parts.parent)?;
        let stored = plan.stored(|base| {
            check_name(base)?;
            self.session
                .with_key(|key| encrypt_name(key, base, &parent.id))
        })?;
        Ok(Some(child(&parent.content, &stored)))
    }

    pub(super) fn stage_rel_required(&self, parts: &Parts<'_>) -> io::Result<String> {
        self.stage_rel(parts)?
            .ok_or_else(|| invalid_input("Stufen im Tresor brauchen einen se-Stufennamen"))
    }

    /// Metadata of an existing stage named like `parts`; `None` when there
    /// is none (the name may still be an entry written by another client).
    pub(super) fn stage_meta(&self, parts: &Parts<'_>) -> io::Result<Option<VfsMeta>> {
        let Some(rel) = self.stage_rel(parts)? else {
            return Ok(None);
        };
        match self.inner.stat(&self.at(&rel)) {
            Ok(meta) if !meta.is_dir && !meta.is_symlink && !meta.special => Ok(Some(VfsMeta {
                name: parts.name.to_string(),
                size: cleartext_file_size(meta.size, self.session.config().combo).unwrap_or(0),
                mtime_ms: meta.mtime_ms,
                btime_ms: meta.btime_ms,
                hidden: parts.name.starts_with('.'),
                id: meta.id,
                ..VfsMeta::default()
            })),
            Ok(_) => Err(invalid_data("Stufe im Tresor ist keine Datei")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Copy stages: `open_write_copy_stage` (`size = None`), `…_sized`,
    /// `…_unsynced`; closed by `flush`, published by the caller.
    pub(super) fn stage_writer(
        &self,
        path: &str,
        size: Option<u64>,
        unsynced: bool,
    ) -> io::Result<Box<dyn Write + Send>> {
        self.session.check()?;
        let result = (|| -> io::Result<_> {
            let at = self.at(&self.stage_rel_required(&Self::parts(path)?)?);
            let combo = self.session.config().combo;
            let stored = match size {
                None => self.inner.open_write_copy_stage(&at)?,
                Some(size) if unsynced => self
                    .inner
                    .open_write_copy_stage_unsynced(&at, file_size(size, combo))?,
                Some(size) => self
                    .inner
                    .open_write_copy_stage_sized(&at, file_size(size, combo))?,
            };
            let writer = VaultWriter::open(&self.session, stored, size, None, None)?;
            Ok(Box::new(writer) as Box<dyn Write + Send>)
        })();
        result.map_err(|error| self.session.relock(error))
    }

    /// `open_write_copy_stage_timed` of the extensions.
    pub(super) fn stage_writer_timed(
        &self,
        path: &str,
        size: u64,
        mtime_ms: i64,
    ) -> io::Result<Box<dyn Write + Send>> {
        self.session.check()?;
        let result = (|| -> io::Result<_> {
            let at = self.at(&self.stage_rel_required(&Self::parts(path)?)?);
            let stored_size = file_size(size, self.session.config().combo);
            let stored =
                vfs::open_write_copy_stage_timed(&*self.inner, &at, stored_size, mtime_ms)?;
            let writer = VaultWriter::open(&self.session, stored, Some(size), None, None)?;
            Ok(Box::new(writer) as Box<dyn Write + Send>)
        })();
        result.map_err(|error| self.session.relock(error))
    }

    /// A cleartext path that names an entry (not the vault root).
    pub(super) fn parts(path: &str) -> io::Result<Parts<'_>> {
        Parts::of(path)?.ok_or_else(|| invalid_input("Die Tresorwurzel ist keine Datei"))
    }

    /// `open_write` (`exclusive = false`: create or replace) and
    /// `open_write_new` (`exclusive = true`): an internal stage published on
    /// `flush`. A stage name opened with `open_write_new` is a private
    /// stage of the caller (mounted writes): created exclusively, never
    /// published here.
    pub(super) fn write_file(
        &self,
        path: &str,
        exclusive: bool,
    ) -> io::Result<Box<dyn Write + Send>> {
        self.session.check()?;
        let result = (|| -> io::Result<_> {
            let parts = Self::parts(path)?;
            if exclusive && stage_plan(parts.name).is_some() {
                let at = self.at(&self.stage_rel_required(&parts)?);
                let stored = self.inner.open_write_new(&at)?;
                let writer = VaultWriter::open(&self.session, stored, None, None, None)?;
                return Ok(Box::new(writer) as Box<dyn Write + Send>);
            }
            match self.find_at(&parts) {
                Ok(_) if exclusive => return Err(already_exists()),
                Ok(Found {
                    kind: NodeKind::File { .. },
                    ..
                }) => {}
                Ok(Found {
                    kind: NodeKind::Dir { .. },
                    ..
                }) => {
                    return Err(io::Error::new(
                        io::ErrorKind::IsADirectory,
                        "Eintrag im Tresor ist ein Ordner",
                    ))
                }
                Ok(_) => {
                    return Err(invalid_input(
                        "Verknüpfungen im Tresor werden nicht verfolgt",
                    ))
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            let tail = format!(".se-vault-{:016x}", random_u64()?);
            let stage_name = crate::vfs::fit_stage_name("", parts.name, &tail);
            let stage_parts = Parts {
                parent: parts.parent.clone(),
                name: &stage_name,
            };
            let at = self.at(&self.stage_rel_required(&stage_parts)?);
            let stored = self.inner.open_write_copy_stage(&at)?;
            let mode = if exclusive {
                Promote::Create
            } else {
                Promote::Replace
            };
            let (backend, stage_path, destination) = (
                self.clone(),
                join_path(&parts.parent, &stage_name),
                path.to_string(),
            );
            let commit: Commit = Box::new(move || backend.promote(&stage_path, &destination, mode));
            let inner = self.inner.clone();
            let abandon: Abandon = Box::new(move || discard_storage_stage(&*inner, &at));
            let writer =
                VaultWriter::open(&self.session, stored, None, Some(commit), Some(abandon))?;
            Ok(Box::new(writer) as Box<dyn Write + Send>)
        })();
        result.map_err(|error| self.session.relock(error))
    }

    /// Publishes the stage `staged` as `destination`.
    pub(super) fn promote(&self, staged: &str, destination: &str, mode: Promote) -> io::Result<()> {
        self.session.check()?;
        let result = (|| -> io::Result<_> {
            let from = self.at(&self.stage_rel_required(&Self::parts(staged)?)?);
            let target = Parts::of(destination)?
                .ok_or_else(|| invalid_input("Die Tresorwurzel ist kein Ziel"))?;
            let parent = self.dir_for(&target.parent)?;
            let parent_key = target.parent_key();
            let existing = match self.find(&parent_key, &parent, target.name) {
                Ok(found) => Some(found),
                Err(error) if error.kind() == io::ErrorKind::NotFound => None,
                Err(error) => return Err(error),
            };
            let published = match (existing, mode) {
                (Some(_), Promote::Create) => Err(already_exists()),
                (
                    Some(Found {
                        node,
                        kind: NodeKind::File { .. },
                    }),
                    Promote::Replace,
                ) => self.inner.promote_staged(&from, &self.at(&node.contents())),
                (
                    Some(Found {
                        node,
                        kind: NodeKind::File { .. },
                    }),
                    Promote::ToId(id),
                ) => self.inner.promote_staged_to_id(
                    &from,
                    &self.at(&node.contents()),
                    id.as_deref(),
                ),
                (Some(_), _) => Err(invalid_input(
                    "Ein Ordner oder eine Verknüpfung wird nicht durch eine Datei ersetzt",
                )),
                (None, Promote::ToId(_)) => Err(not_found()),
                (None, _) => self.publish_new(&from, &self.canonical(&parent, target.name)?),
            };
            self.cache.forget_aliases(&parent_key);
            published
        })();
        result.map_err(|error| self.session.relock(error))
    }

    /// A new file entry from the stage `from` (storage path).
    fn publish_new(&self, from: &str, node: &NodeRef) -> io::Result<()> {
        if !node.shortened {
            return self
                .inner
                .promote_staged_no_replace(from, &self.at(&node.node()));
        }
        let folder = self.at(&node.node());
        let created = match self.inner.create_dir_new(&folder) {
            Ok(()) => true,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                // A long-named folder without a type file (left by an
                // aborted publication) is reused; a typed one is an entry.
                let inside = self.inner.list_dir(&folder)?;
                let typed = [CONTENTS_FILE, DIR_FILE, SYMLINK_FILE];
                if inside
                    .iter()
                    .any(|entry| typed.contains(&entry.name.as_str()))
                {
                    return Err(already_exists());
                }
                false
            }
            Err(error) => return Err(error),
        };
        let published = self.ensure_long_name(node).and_then(|()| {
            self.inner
                .promote_staged_no_replace(from, &self.at(&node.inside(CONTENTS_FILE)))
        });
        if published.is_err() && created {
            self.remove_quietly(&[node.inside(INFLATED_NAME_FILE)], &node.node());
        }
        published
    }

    /// Writes `name.c9s` of a long-named folder, or checks the one there.
    pub(super) fn ensure_long_name(&self, node: &NodeRef) -> io::Result<()> {
        let rel = node.inside(INFLATED_NAME_FILE);
        match self.write_new_file(&rel, node.encrypted.as_bytes()) {
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let reader = self.inner.open_read(&self.at(&rel))?;
                let stored = read_bounded(reader, INFLATED_NAME_READ_LIMIT)?;
                if stored.as_deref() == Some(node.encrypted.as_bytes()) {
                    Ok(())
                } else {
                    Err(invalid_data(LONG_NAME_MISMATCH))
                }
            }
            other => other,
        }
    }

    /// A small new file of the vault structure (`dir.c9r`, `dirid.c9r`,
    /// `name.c9s`), created exclusively and flushed.
    pub(super) fn write_new_file(&self, rel: &str, bytes: &[u8]) -> io::Result<()> {
        let at = self.at(rel);
        let mut writer = match self
            .inner
            .open_write_copy_stage_sized(&at, bytes.len() as u64)
        {
            Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                self.inner.open_write(&at)?
            }
            other => other?,
        };
        writer.write_all(bytes)?;
        writer.flush()
    }

    /// `remove_file`: a stage of that name, else the entry (a file or a
    /// link; folders are refused).
    pub(super) fn remove_entry(&self, path: &str, id: Option<&str>) -> io::Result<()> {
        self.session.check()?;
        let result = (|| -> io::Result<_> {
            let parts = Self::parts(path)?;
            if let Some(rel) = self.stage_rel(&parts)? {
                match self.inner.remove_file(&self.at(&rel)) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    other => return other,
                }
            }
            let found = self.find_at(&parts)?;
            let removed = match found.kind {
                NodeKind::File { .. } if !found.node.shortened => {
                    self.inner.remove_file_id(&self.at(&found.node.node()), id)
                }
                NodeKind::File { .. } => self.remove_node_folder(&found.node, CONTENTS_FILE),
                NodeKind::Symlink { .. } => self.remove_node_folder(&found.node, SYMLINK_FILE),
                NodeKind::Dir { .. } => Err(io::Error::new(
                    io::ErrorKind::IsADirectory,
                    "Eintrag im Tresor ist ein Ordner",
                )),
            };
            self.cache.forget_aliases(&parts.parent_key());
            removed
        })();
        result.map_err(|error| self.session.relock(error))
    }

    /// `discard_copy_stage`: the storage's discard of the mapped stage.
    pub(super) fn discard_stage(&self, stage: &str) -> io::Result<()> {
        self.session.check()?;
        let result = (|| -> io::Result<_> {
            let at = self.at(&self.stage_rel_required(&Self::parts(stage)?)?);
            self.inner.discard_copy_stage(&at)
        })();
        result.map_err(|error| self.session.relock(error))
    }
}
