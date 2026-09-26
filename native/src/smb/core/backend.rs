use super::listing;
use super::replace;
use super::session::{SmbConfig, SmbSession};
use super::streams::{SmbReader, SmbWriter};
use super::url::{entry_path, rename_paths, split_path, SmbPath};
use super::wire::{self, DeleteKind};
use crate::vfs::{Backend, Scheme, StagedWriteCapabilities, VfsMeta, VfsResult};
use smb2::ErrorKind;
use std::io::{self, Read, Write};
use std::sync::Arc;

#[derive(Clone)]
pub struct SmbBackend {
    session: Arc<SmbSession>,
    /// The share of the connection root: the only entry at the server level.
    share: String,
    root: String,
    /// `smb://user@host:port/root`, the persisted-state identity.
    url: String,
    /// `smb://user@host:port`, independent of the start folder.
    namespace: String,
}

fn name_of(path: &str) -> String {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

impl SmbBackend {
    /// Connects, signs in and connects the share of `config.root` (a wrong
    /// share name fails the connect with `NotFound`).
    pub fn connect(config: SmbConfig) -> io::Result<SmbBackend> {
        let share = match split_path(&config.root)? {
            Some(root) => root.share,
            None => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Bei SMB beginnt der Startordner mit der Freigabe („/freigabe/ordner“)",
                ))
            }
        };
        let namespace = format!("smb://{}@{}:{}", config.user, config.host, config.port);
        let url = format!("{namespace}{}", config.root);
        let root = config.root.clone();
        let session = SmbSession::connect(config, &share)?;
        Ok(SmbBackend {
            session,
            share,
            root,
            url,
            namespace,
        })
    }

    fn write_file(&self, path: &str, exclusive: bool) -> VfsResult<Box<dyn Write + Send>> {
        let target = entry_path(path)?;
        let (generation, tree) = self.session.target(&target.share)?;
        let opened = self.session.block_on(async {
            if exclusive {
                tree.create_file_writer_exclusive(generation.connection(), &target.rel)
                    .await
            } else {
                tree.create_file_writer(generation.connection(), &target.rel)
                    .await
            }
        });
        let writer = opened.map_err(|error| {
            generation.note(&error);
            super::errors::map(error, "Datei anlegen", path)
        })?;
        Ok(Box::new(SmbWriter::new(
            writer,
            &target.rel,
            path,
            tree,
            generation,
            self.session.runtime(),
        )))
    }

    fn rename_with(&self, src: &str, dst: &str, replace_existing: bool) -> VfsResult<()> {
        let (from, to): (SmbPath, SmbPath) = rename_paths(src, dst)?;
        let (from_rel, to_rel) = (from.rel, to.rel);
        self.session
            .write(&from.share, "Umbenennen", src, |conn, tree| async move {
                replace::rename(&conn, &tree, &from_rel, &to_rel, replace_existing).await
            })
    }

    fn delete(&self, path: &str, kind: DeleteKind) -> VfsResult<()> {
        let target = entry_path(path)?;
        let rel = target.rel;
        self.session
            .write(&target.share, "Löschen", path, |conn, tree| async move {
                wire::delete(&conn, &tree, &rel, kind).await
            })
    }
}

impl Backend for SmbBackend {
    fn scheme(&self) -> Scheme {
        Scheme::Smb
    }

    fn root_display(&self) -> String {
        self.root.clone()
    }

    fn state_identity(&self) -> String {
        self.url.clone()
    }

    fn namespace_identity(&self) -> String {
        self.namespace.clone()
    }

    fn list_dir(&self, path: &str) -> VfsResult<Vec<VfsMeta>> {
        let Some(target) = split_path(path)? else {
            return Ok(listing::server_level(&self.share));
        };
        let rel = target.rel;
        self.session
            .read(&target.share, "Ordner lesen", path, |conn, tree| {
                let rel = rel.clone();
                async move { wire::list(&conn, &tree, &rel).await }
            })
    }

    fn stat(&self, path: &str) -> VfsResult<VfsMeta> {
        let Some(target) = split_path(path)? else {
            return Ok(VfsMeta {
                is_dir: true,
                ..VfsMeta::default()
            });
        };
        let rel = target.rel;
        let attributes =
            self.session
                .read(&target.share, "Eigenschaften lesen", path, |conn, tree| {
                    let rel = rel.clone();
                    async move { wire::stat(&conn, &tree, &rel).await }
                })?;
        Ok(listing::meta(name_of(path), &attributes))
    }

    fn open_read(&self, path: &str) -> VfsResult<Box<dyn Read + Send>> {
        let target = entry_path(path)?;
        let mut retried = false;
        loop {
            let (generation, tree) = self.session.target(&target.share)?;
            let opened = self
                .session
                .block_on(tree.open_file_reader(generation.connection(), &target.rel));
            match opened {
                Ok(reader) => {
                    return Ok(Box::new(SmbReader::new(
                        reader,
                        path,
                        generation,
                        self.session.runtime(),
                    )))
                }
                // A lost connection before any byte was read: reopen once.
                Err(error) if generation.note(&error) && !retried => retried = true,
                Err(error) => return Err(super::errors::map(error, "Öffnen", path)),
            }
        }
    }

    fn open_write(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.write_file(path, false)
    }

    /// One CREATE with `FileCreate`: an existing name fails with
    /// `AlreadyExists` instead of being truncated.
    fn open_write_new(&self, path: &str) -> VfsResult<Box<dyn Write + Send>> {
        self.write_file(path, true)
    }

    /// ReplaceIfExists = 1: an existing file is replaced in one step.
    fn rename(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.rename_with(src, dst, true)
    }

    /// ReplaceIfExists = 0: the server refuses an existing target.
    fn rename_no_replace(&self, src: &str, dst: &str) -> VfsResult<()> {
        self.rename_with(src, dst, false)
    }

    fn remove_file(&self, path: &str) -> VfsResult<()> {
        self.delete(path, DeleteKind::File)
    }

    fn remove_dir(&self, path: &str) -> VfsResult<()> {
        self.delete(path, DeleteKind::Directory)
    }

    fn mkdir_all(&self, path: &str) -> VfsResult<()> {
        let Some(target) = split_path(path)? else {
            return Ok(());
        };
        let rel = target.rel;
        self.session.write(
            &target.share,
            "Ordner anlegen",
            path,
            |mut conn, tree| async move {
                let mut current = String::new();
                for part in rel.split('/').filter(|part| !part.is_empty()) {
                    if !current.is_empty() {
                        current.push('/');
                    }
                    current.push_str(part);
                    match tree.create_directory(&mut conn, &current).await {
                        Err(error) if error.kind() != ErrorKind::AlreadyExists => {
                            return Err(error)
                        }
                        _ => {}
                    }
                }
                // The share root always exists; a name taken by a file must
                // not pass as the created folder.
                let attributes = wire::stat(&conn, &tree, &rel).await?;
                if !attributes.is_dir() {
                    return Err(wire::protocol(
                        smb2::types::status::NtStatus::OBJECT_NAME_COLLISION,
                        smb2::types::Command::Create,
                    ));
                }
                Ok::<(), smb2::Error>(())
            },
        )
    }

    fn parallelism(&self) -> usize {
        // One session multiplexes requests; two keep a walk moving without
        // starving the credit window of a small NAS.
        2
    }

    /// Replacing is one ReplaceIfExists rename and new files are created
    /// exclusively, so every staged-write guarantee holds. Drive mounts
    /// still refuse SMB explicitly (`daemon` mount source).
    fn rename_overwrites(&self) -> bool {
        true
    }

    fn staged_write_capabilities(&self, _root: &str) -> StagedWriteCapabilities {
        StagedWriteCapabilities::complete()
    }
}
